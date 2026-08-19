// A skew of exactly zero is the documented result of a single accepted
// sample (a regression needs several), so exact equality is the correct
// assertion here, not a tolerance.
#![allow(clippy::float_cmp)]

use super::error::ListenerPlaybackError;
use super::pump::{active_pump_threads_for_test, drain_due_frames};
use super::runtime::ListenerPlaybackRuntime;
use crate::audio_abi::{
    register_render_ring, registry_test_guard, release_render_ring,
    silent_disco_audio_read_interleaved_f32,
};
use silent_disco_core::audio::{
    PlaybackPhase, PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PACKET_DURATION_MS: u32 = 20;
const SAMPLES_PER_PACKET: u32 = 960;

fn scheduler_config() -> SchedulerConfig {
    let mut config = SchedulerConfig::new(
        SessionId::new("session-runtime").expect("session id"),
        StreamId::new("stream-runtime").expect("stream id"),
        48_000,
        0,
        SAMPLES_PER_PACKET,
        2,
    );
    config.startup_buffer_target_ms = 0;
    config
}

fn ring_config() -> RenderRingConfig {
    RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    }
}

fn datagram(sequence: u64) -> AudioDatagram {
    AudioDatagram {
        session_id: SessionId::new("session-runtime").expect("session id"),
        stream_id: StreamId::new("stream-runtime").expect("stream id"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: SAMPLES_PER_PACKET,
        first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
        host_presentation_time_ms: MonotonicMillis::new(sequence * u64::from(PACKET_DURATION_MS)),
        payload: (0..SAMPLES_PER_PACKET * 2)
            .flat_map(|_| 16_384_i16.to_le_bytes())
            .collect(),
    }
}

/// Real-time throughput must not depend on how the stream is packetized.
///
/// One `PlaybackPump::tick` releases at most one frame, so a pump thread
/// that ticked once per 10ms wake-up could never exceed 100 packets per
/// second. At 20ms packets that is 2x real time and invisible; at the 5ms
/// production packet duration it is *half* real time, and a device run
/// showed exactly that — 60-95 packets/second emitted against the 200 the
/// stream needed, the ring at zero for every playing second, and the
/// jitter buffer backing up until arrivals fell outside the reorder
/// window. One wake-up therefore has to drain everything already due.
#[test]
fn one_pump_wake_up_drains_every_frame_already_due() {
    const SHORT_PACKET_MS: u32 = 5;
    const SHORT_SAMPLES: u32 = 240;
    const DUE_PACKETS: u64 = 40;

    let _guard = registry_test_guard();
    let mut config = SchedulerConfig::new(
        SessionId::new("session-drain").expect("session id"),
        StreamId::new("stream-drain").expect("stream id"),
        48_000,
        0,
        SHORT_SAMPLES,
        2,
    );
    config.startup_buffer_target_ms = 0;
    let scheduler = PlaybackScheduler::new(config, 0.0).expect("valid scheduler");
    let (producer, token) = register_render_ring(ring_config()).expect("ring registered");
    let mut pump =
        PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default()).expect("valid pump");
    // Nothing may play against a placeholder offset, so lock sync first.
    pump.apply_sync_offset(0.0);

    for sequence in 0..DUE_PACKETS {
        pump.scheduler_mut()
            .submit_packet(AudioDatagram {
                session_id: SessionId::new("session-drain").expect("session id"),
                stream_id: StreamId::new("stream-drain").expect("stream id"),
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: SHORT_SAMPLES,
                first_sample_index: SampleIndex::new(sequence * u64::from(SHORT_SAMPLES)),
                host_presentation_time_ms: MonotonicMillis::new(
                    sequence * u64::from(SHORT_PACKET_MS),
                ),
                payload: (0..SHORT_SAMPLES * 2)
                    .flat_map(|_| 16_384_i16.to_le_bytes())
                    .collect(),
            })
            .expect("accepted");
    }

    // Every packet above is due at this instant.
    let released = drain_due_frames(&mut pump, DUE_PACKETS * u64::from(SHORT_PACKET_MS));

    assert!(
        released >= usize::try_from(DUE_PACKETS).expect("fits"),
        "one wake-up released only {released} of {DUE_PACKETS} due frames, which caps \
         throughput below real time at short packet durations"
    );
    assert_eq!(pump.diagnostics().packets_emitted, DUE_PACKETS);
    release_render_ring(token).expect("ring released");
}

/// Locks sync on a near-zero offset so the helper datagrams, whose
/// presentation times start at zero, are immediately due.
fn lock_sync_at_zero_offset(runtime: &ListenerPlaybackRuntime) {
    let send = runtime.now_ms();
    runtime.begin_sync_probe(1, send).expect("probe registered");
    let receive = runtime.now_ms();
    let outcome = runtime
        .observe_sync_response(1, send, send, send, receive)
        .expect("a correlated response is not an error");
    assert!(outcome.sync_locked, "playback cannot start without sync");
}

fn token_as_engine(token: u64) -> *mut core::ffi::c_void {
    usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void
}

#[test]
fn start_registers_a_readable_engine_token_and_stop_releases_it() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    let token = runtime.engine_token();
    assert!(token > 0);

    // While running, the token resolves to a live engine.
    let mut output = [0.0_f32; 2];
    let mut frames_from_ring = 0_u32;
    let status = silent_disco_audio_read_interleaved_f32(
        token_as_engine(token),
        output.as_mut_ptr(),
        1,
        2,
        &raw mut frames_from_ring,
    );
    assert!(
        status == 0 || status == 1,
        "expected OK or PARTIAL, got {status}"
    );

    runtime.stop().expect("stop succeeds");

    // After stopping, the same token must report stopping rather than
    // being silently reused or treated as unknown.
    let status = silent_disco_audio_read_interleaved_f32(
        token_as_engine(token),
        output.as_mut_ptr(),
        1,
        2,
        &raw mut frames_from_ring,
    );
    assert_eq!(status, 2 /* STOPPING */);
}

/// Stopping must let the render ring play out, not discard it.
///
/// The ring holds the write-ahead cushion by design — roughly 400ms, and
/// measured at 397ms still queued on a device at stop. Closing the output
/// with that queued throws away the end of the stream, which is audible as
/// an abrupt cut-off and is invisible to the debug capture, since that
/// records frames on their way *into* the ring.
#[test]
fn stopping_plays_out_what_is_still_queued_in_the_ring() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    let token = runtime.engine_token();
    lock_sync_at_zero_offset(&runtime);
    for sequence in 0..8 {
        runtime.submit_packet(datagram(sequence)).expect("accepted");
    }
    // Let the pump fill the ring before stopping.
    let filled = Instant::now() + Duration::from_millis(500);
    while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        runtime.diagnostics().ring_queued_frames > 0,
        "nothing was queued, so this would pass without draining anything"
    );

    // A consumer standing in for the real-time audio callback.
    let consuming = Arc::new(AtomicBool::new(true));
    let consumer_flag = Arc::clone(&consuming);
    let consumer = thread::spawn(move || {
        let mut output = [0.0_f32; 512];
        while consumer_flag.load(Ordering::SeqCst) {
            let mut frames_read = 0_u32;
            let _ = silent_disco_audio_read_interleaved_f32(
                token_as_engine(token),
                output.as_mut_ptr(),
                256,
                2,
                &raw mut frames_read,
            );
            thread::sleep(Duration::from_millis(2));
        }
    });

    runtime.stop().expect("stop succeeds");
    consuming.store(false, Ordering::SeqCst);
    consumer.join().expect("consumer thread ends");

    let final_diagnostics = runtime
        .final_diagnostics()
        .expect("diagnostics captured at stop");
    assert_eq!(
        final_diagnostics.ring_queued_frames, 0,
        "stop abandoned {} frames still queued in the ring",
        final_diagnostics.ring_queued_frames
    );
}

/// The drain must not turn into a hang when nothing is consuming — an
/// output that failed to open, or one already closed.
#[test]
fn stopping_without_a_consumer_gives_up_instead_of_waiting_forever() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    lock_sync_at_zero_offset(&runtime);
    for sequence in 0..8 {
        runtime.submit_packet(datagram(sequence)).expect("accepted");
    }
    let filled = Instant::now() + Duration::from_millis(500);
    while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(runtime.diagnostics().ring_queued_frames > 0);

    let started = Instant::now();
    runtime.stop().expect("stop succeeds");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "stop took {elapsed:?} with no consumer draining the ring"
    );
}

#[test]
fn stop_is_idempotent() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    runtime.stop().expect("first stop succeeds");
    runtime
        .stop()
        .expect("a repeated stop is a no-op, not an error");
}

#[test]
fn submitting_after_stop_is_an_explicit_failure_not_a_silent_no_op() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    runtime
        .submit_packet(datagram(0))
        .expect("accepted while running");
    runtime.stop().expect("stop succeeds");

    let error = runtime
        .submit_packet(datagram(1))
        .expect_err("a stopped runtime must reject further packets");
    assert!(matches!(error, ListenerPlaybackError::Stopped(_)));
}

#[test]
fn final_diagnostics_survive_the_teardown_that_produced_them() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    // Nothing is captured while the stream is still live.
    assert!(runtime.final_diagnostics().is_none());
    assert!(!runtime.diagnostics().sync_locked);

    // Packets are only accepted once a clock offset exists; before that
    // they are dropped rather than stranding the buffer.
    for sequence in 0..5 {
        runtime.submit_packet(datagram(sequence)).expect("no error");
    }
    assert_eq!(runtime.diagnostics().dropped_before_sync, 5);

    runtime.begin_sync_probe(1, 0).expect("probe registered");
    runtime
        .observe_sync_response(1, 0, 500_000, 500_001, 20)
        .expect("correlated response");
    for sequence in 0..5 {
        runtime.submit_packet(datagram(sequence)).expect("accepted");
    }
    assert_eq!(runtime.diagnostics().packets_accepted, 5);

    runtime.stop().expect("stop succeeds");

    // A stream's final accounting must outlive the teardown, or the one
    // moment worth reporting is the one moment it cannot be read.
    let summary = runtime
        .final_diagnostics()
        .expect("stopping captures a final summary");
    assert_eq!(summary.packets_accepted, 5);
    assert_eq!(summary.phase, PlaybackPhase::Stopped);
}

#[test]
fn debug_capture_writes_a_playable_wav_for_the_streams_audio() {
    let _guard = registry_test_guard();
    let directory = std::env::temp_dir().join(format!(
        "silent-disco-runtime-capture-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("temp directory");
    let path = directory.join("runtime-capture.wav");

    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    runtime
        .start_debug_capture(path.to_str().expect("utf-8 path"))
        .expect("capture starts");

    // Lock sync and feed audio so the pump actually releases frames.
    runtime.begin_sync_probe(1, 0).expect("probe registered");
    runtime
        .observe_sync_response(1, 0, 500_000, 500_001, 20)
        .expect("correlated response");
    for sequence in 0..5 {
        runtime.submit_packet(datagram(sequence)).expect("accepted");
    }
    runtime.stop().expect("stop succeeds");

    assert!(runtime.debug_capture_error().is_none());
    let bytes = std::fs::read(&path).expect("capture readable");
    assert_eq!(&bytes[0..4], b"RIFF");
    // The tail drained at stop is captured, so the file holds real audio
    // rather than just a header.
    assert!(
        bytes.len() > 44,
        "expected captured samples, got {} bytes",
        bytes.len()
    );

    std::fs::remove_file(&path).ok();
}

#[test]
fn debug_capture_fails_explicitly_when_the_path_cannot_be_written() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    let error = runtime
        .start_debug_capture("/nonexistent-directory/capture.wav")
        .expect_err("an unwritable path must fail rather than silently not record");
    assert!(matches!(error, ListenerPlaybackError::DebugCapture(_)));

    runtime.stop().expect("stop succeeds");
}

#[test]
fn a_rejected_ring_configuration_leaves_nothing_registered() {
    let _guard = registry_test_guard();
    let error = ListenerPlaybackRuntime::start(
        scheduler_config(),
        RenderRingConfig {
            capacity_frames: 1,
            target_fill_frames: 1,
        },
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect_err("an undersized ring must be rejected");

    assert!(matches!(
        error,
        ListenerPlaybackError::InvalidConfiguration(_)
    ));
}

#[test]
fn a_rejected_pump_configuration_releases_the_ring_it_had_already_registered() {
    let _guard = registry_test_guard();
    let error = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig {
            volume: 2.0,
            ..PlaybackPumpConfig::default()
        },
        0.0,
    )
    .expect_err("an out-of-range volume must be rejected");

    assert!(matches!(
        error,
        ListenerPlaybackError::InvalidConfiguration(_)
    ));
    // A partially-completed start must not leave a registered engine
    // behind; the next start gets a fresh token that reads normally.
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("a later start still succeeds");
    runtime.stop().expect("stop succeeds");
}

#[test]
fn dropping_without_stopping_still_releases_the_token() {
    let _guard = registry_test_guard();
    let token = {
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        runtime.engine_token()
    };

    let mut output = [0.0_f32; 2];
    let mut frames_from_ring = 0_u32;
    let status = silent_disco_audio_read_interleaved_f32(
        token_as_engine(token),
        output.as_mut_ptr(),
        1,
        2,
        &raw mut frames_from_ring,
    );
    assert_eq!(status, 2 /* STOPPING */);
}

/// Block 24: repeatedly races `stop()` against a simulated Oboe callback
/// thread (continuously reading the engine token) and a simulated
/// network-arrival thread (continuously submitting packets), across many
/// fresh runtimes. This is the scenario the architecture spec calls out
/// by name: "stop and join the stream before releasing the Rust engine
/// token" -- if that ordering were ever violated, the callback thread
/// would read a released or reused token while still "active" from the
/// platform's point of view.
///
/// Every iteration must: never panic on any thread, have `stop()` return
/// in bounded time regardless of how much concurrent traffic is in
/// flight, and leave the token reporting `Stopping` (never
/// `InvalidState`) once both threads have been joined.
#[test]
fn stop_races_repeatedly_against_a_simulated_audio_callback_and_packet_arrivals() {
    let _guard = registry_test_guard();
    for iteration in 0_u64..20 {
        let runtime = Arc::new(
            ListenerPlaybackRuntime::start(
                scheduler_config(),
                ring_config(),
                PlaybackPumpConfig::default(),
                0.0,
            )
            .expect("runtime starts"),
        );
        lock_sync_at_zero_offset(&runtime);
        let token = runtime.engine_token();

        let callback_running = Arc::new(AtomicBool::new(true));
        let callback_flag = Arc::clone(&callback_running);
        let callback_thread = thread::spawn(move || {
            let mut output = [0.0_f32; 512];
            while callback_flag.load(Ordering::SeqCst) {
                let mut frames_read = 0_u32;
                let status = silent_disco_audio_read_interleaved_f32(
                    token_as_engine(token),
                    output.as_mut_ptr(),
                    256,
                    2,
                    &raw mut frames_read,
                );
                assert_ne!(
                    status, -3, /* PANIC_CONTAINED */
                    "the real-time read path must never panic under a stop race"
                );
            }
        });

        let arrivals_running = Arc::new(AtomicBool::new(true));
        let arrivals_flag = Arc::clone(&arrivals_running);
        let arrivals_runtime = Arc::clone(&runtime);
        let arrivals_thread = thread::spawn(move || {
            let mut sequence = iteration * 10_000;
            while arrivals_flag.load(Ordering::SeqCst) {
                // Both outcomes (accepted, or rejected because the
                // runtime already stopped) are legitimate; only a panic
                // or hang here would be a bug.
                let _ = arrivals_runtime.submit_packet(datagram(sequence));
                sequence += 1;
            }
        });

        // Let both threads generate real concurrent load before racing
        // the stop itself.
        thread::sleep(Duration::from_millis(5));
        let stop_started = Instant::now();
        runtime
            .stop()
            .expect("stop succeeds even under concurrent load");
        let stop_elapsed = stop_started.elapsed();
        assert!(
            stop_elapsed < Duration::from_secs(3),
            "stop took {stop_elapsed:?} while racing a simulated audio callback"
        );

        arrivals_running.store(false, Ordering::SeqCst);
        arrivals_thread
            .join()
            .expect("arrivals thread must not panic");
        callback_running.store(false, Ordering::SeqCst);
        callback_thread
            .join()
            .expect("callback thread must not panic");

        let mut output = [0.0_f32; 2];
        let mut frames_from_ring = 0_u32;
        let status_after = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status_after, 2 /* STOPPING */);
    }
}

#[test]
fn repeated_runtime_start_stop_cycles_leave_no_pump_worker_alive() {
    let _guard = registry_test_guard();
    let baseline = active_pump_threads_for_test();

    for cycle in 0..10 {
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        let start_deadline = Instant::now() + Duration::from_millis(500);
        while active_pump_threads_for_test() <= baseline && Instant::now() < start_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            active_pump_threads_for_test(),
            baseline + 1,
            "cycle {cycle} did not own exactly one pump worker"
        );

        runtime.stop().expect("runtime stops");
        let stop_deadline = Instant::now() + Duration::from_millis(500);
        while active_pump_threads_for_test() != baseline && Instant::now() < stop_deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            active_pump_threads_for_test(),
            baseline,
            "cycle {cycle} left a pump worker alive after stop"
        );
    }
}

#[test]
fn a_contained_pump_panic_is_visible_and_later_calls_fail_as_pump_thread_errors() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    let ticking_deadline = Instant::now() + Duration::from_millis(500);
    while runtime.pump_liveness().tick_count == 0 && Instant::now() < ticking_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        runtime.pump_liveness().tick_count > 0,
        "pump never reported liveness"
    );

    runtime.inject_pump_panic_for_test();
    let panic_deadline = Instant::now() + Duration::from_millis(500);
    while runtime.pump_liveness().contained_panics == 0 && Instant::now() < panic_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let liveness = runtime.pump_liveness();
    assert_eq!(liveness.contained_panics, 1);
    assert!(liveness.tick_count > 0);

    let submit_error = runtime
        .submit_packet(datagram(0))
        .expect_err("a dead pump must not masquerade as a live runtime");
    assert!(matches!(submit_error, ListenerPlaybackError::PumpThread(_)));

    let stop_error = runtime
        .stop()
        .expect_err("explicit cleanup must preserve the contained worker failure");
    assert!(matches!(stop_error, ListenerPlaybackError::PumpThread(_)));
    // Idempotence applies after the first explicit cleanup even when that
    // cleanup reported the terminal worker failure.
    runtime.stop().expect("second stop is idempotent");
}
