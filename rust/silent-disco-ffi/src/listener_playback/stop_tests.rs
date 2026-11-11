// Host-timed stop: trim the tail past the host's stop time and cut the ring.

use super::runtime::ListenerPlaybackRuntime;
use super::tests::{
    datagram, lock_sync_at_zero_offset, ring_config, scheduler_config, token_as_engine,
};
use crate::audio_abi::{registry_test_guard, silent_disco_audio_read_interleaved_f32};
use silent_disco_core::audio::PlaybackPumpConfig;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// 240 frames every 5 ms: the real-time rate at 48 kHz, so draining a
/// backlog takes as long as it would on a device.
fn spawn_real_time_consumer(token: u64) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let consuming = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&consuming);
    let consumer = thread::spawn(move || {
        let mut output = [0.0_f32; 480];
        while flag.load(Ordering::SeqCst) {
            let mut frames_read = 0_u32;
            let _ = silent_disco_audio_read_interleaved_f32(
                token_as_engine(token),
                output.as_mut_ptr(),
                240,
                2,
                &raw mut frames_read,
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
    (consuming, consumer)
}

/// A mid-song stop must not play out the second of audio the host sent
/// ahead: on a device that held up the next song's start by ~0.7 s, during
/// which its first packets had nowhere to go and were lost.
#[test]
fn a_host_timed_stop_drops_the_tail_past_the_stop_and_returns_promptly() {
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
    // 1.2 s of audio from host time 0; with a zero offset, host time is the
    // runtime's own clock, so most of it is still ahead.
    for sequence in 0..60 {
        runtime.submit_packet(datagram(sequence)).expect("accepted");
    }
    let filled = Instant::now() + Duration::from_millis(500);
    while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(runtime.diagnostics().ring_queued_frames > 0);
    let (consuming, consumer) = spawn_real_time_consumer(token);

    let stop_host_ms = runtime.now_ms() + 50;
    let started = Instant::now();
    runtime.stop_at(stop_host_ms).expect("stop succeeds");
    let elapsed = started.elapsed();
    consuming.store(false, Ordering::SeqCst);
    consumer.join().expect("consumer thread ends");

    let diagnostics = runtime
        .final_diagnostics()
        .expect("diagnostics captured at stop");
    assert!(
        diagnostics.stop_trimmed_packets > 0,
        "the audio past the stop time was not trimmed"
    );
    assert_eq!(
        diagnostics.ring_queued_frames, 0,
        "audio written ahead past the stop time was left queued"
    );
    // Playing the whole backlog out would take most of a second.
    assert!(
        elapsed < Duration::from_millis(400),
        "stop took {elapsed:?}, as if it played the tail past the stop time"
    );
}

/// When the stop time is after everything buffered -- a natural end -- the
/// whole tail still plays, exactly as with a plain stop.
#[test]
fn a_host_timed_stop_after_the_buffered_audio_plays_all_of_it() {
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
    let filled = Instant::now() + Duration::from_millis(500);
    while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
        thread::sleep(Duration::from_millis(5));
    }
    let (consuming, consumer) = spawn_real_time_consumer(token);

    runtime.stop_at(60_000).expect("stop succeeds");
    consuming.store(false, Ordering::SeqCst);
    consumer.join().expect("consumer thread ends");

    let diagnostics = runtime
        .final_diagnostics()
        .expect("diagnostics captured at stop");
    assert_eq!(diagnostics.stop_trimmed_packets, 0);
    assert_eq!(
        diagnostics.ring_queued_frames, 0,
        "the tail was not played out"
    );
}
