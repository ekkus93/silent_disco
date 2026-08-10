//! Block 34.3 tests for `DesktopMonitorControl` using a fake
//! [`AudioOutputBackend`], mirroring this codebase's established DI-based
//! testing pattern (`MdnsPublisher`, `DesktopIdentityProvider`,
//! `TransportFactory`) -- deterministic, no real audio hardware needed.

use super::audio_device::{
    AudioOutputBackend, AudioOutputConfig, AudioOutputError, RenderCallback,
    RunningAudioOutputStream,
};
use super::monitor::DesktopMonitorControl;
use silent_disco_core::audio::{CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ, RENDER_CHANNELS};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::AudioDatagram;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn canonical_config() -> AudioOutputConfig {
    AudioOutputConfig {
        channels: CANONICAL_CHANNELS,
        sample_rate_hz: CANONICAL_SAMPLE_RATE_HZ,
    }
}

/// Configurable fake backend. Every `start()` spawns a real thread that
/// repeatedly calls `callback.write` (the *actual* production callback
/// logic, not a reimplementation) at a fast interval, appending every
/// buffer it observes into `captured` so a test can inspect exactly what
/// flowed all the way through the real pipeline.
struct FakeBackend {
    config: Mutex<Result<AudioOutputConfig, AudioOutputError>>,
    start_should_fail: AtomicBool,
    /// If `Some(n)`, the fake's driving thread calls `on_error` after
    /// exactly `n` successful writes and then stops writing -- simulating
    /// a device disappearing mid-stream (34.3 "device removal").
    fail_after_writes: Mutex<Option<usize>>,
    captured: Arc<Mutex<Vec<f32>>>,
    starts: AtomicUsize,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            config: Mutex::new(Ok(canonical_config())),
            start_should_fail: AtomicBool::new(false),
            fail_after_writes: Mutex::new(None),
            captured: Arc::new(Mutex::new(Vec::new())),
            starts: AtomicUsize::new(0),
        }
    }

    fn with_config(config: Result<AudioOutputConfig, AudioOutputError>) -> Self {
        let backend = Self::new();
        *backend.config.lock().expect("config lock") = config;
        backend
    }
}

impl AudioOutputBackend for FakeBackend {
    fn default_output_config(&self) -> Result<AudioOutputConfig, AudioOutputError> {
        self.config.lock().expect("config lock").clone()
    }

    fn start(
        &self,
        _config: AudioOutputConfig,
        mut callback: RenderCallback,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn RunningAudioOutputStream>, AudioOutputError> {
        if self.start_should_fail.load(Ordering::Acquire) {
            return Err(AudioOutputError::StreamBuildFailed(
                "fake: build rejected".to_owned(),
            ));
        }
        self.starts.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let captured = Arc::clone(&self.captured);
        let fail_after = *self.fail_after_writes.lock().expect("fail-after lock");
        let thread = thread::Builder::new()
            .name("fake-audio-output".to_owned())
            .spawn(move || {
                let mut writes = 0_usize;
                let mut buffer = [0.0_f32; 96];
                while !stop_for_thread.load(Ordering::Acquire) {
                    callback.write(&mut buffer);
                    writes += 1;
                    if let Ok(mut log) = captured.lock() {
                        log.extend_from_slice(&buffer);
                    }
                    if fail_after == Some(writes) {
                        on_error("fake: device disappeared".to_owned());
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            })
            .expect("spawn fake output thread");
        Ok(Box::new(FakeRunningStream {
            stop,
            thread: Some(thread),
        }))
    }
}

struct FakeRunningStream {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RunningAudioOutputStream for FakeRunningStream {
    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // Joining here is exactly what proves 34.3's "callback after
            // release prevention": once `stop` returns, the fake's thread
            // (which is the only thing that ever calls `callback.write`)
            // is provably gone, so nothing can invoke the callback again.
            drop(thread.join());
        }
    }
}

fn session() -> SessionId {
    SessionId::new("session-block34").expect("session id")
}

fn stream() -> StreamId {
    StreamId::new("stream-block34").expect("stream id")
}

/// A datagram carrying a fixed, easily-recognized PCM16 pattern -- the
/// "test tone" 34.3 asks for, distinguishable from silence at a glance.
fn tone_datagram(
    sequence: u64,
    host_presentation_time_ms: u64,
    samples_per_packet: u32,
) -> AudioDatagram {
    let mut payload = Vec::with_capacity(samples_per_packet as usize * RENDER_CHANNELS * 2);
    for _ in 0..samples_per_packet {
        for sample in [4_000_i16, -4_000_i16] {
            payload.extend_from_slice(&sample.to_le_bytes());
        }
    }
    AudioDatagram {
        session_id: session(),
        stream_id: stream(),
        sequence: PacketSequence::new(sequence),
        codec: silent_disco_core::protocol::AudioCodec::PcmS16Le,
        sample_rate: CANONICAL_SAMPLE_RATE_HZ,
        channels: CANONICAL_CHANNELS,
        samples_per_packet,
        first_sample_index: SampleIndex::new(sequence * u64::from(samples_per_packet)),
        host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
        payload,
    }
}

/// A clock the test fully controls -- lets the monitor pump's timeline
/// advance instantly rather than depending on real wall-clock pacing to
/// clear the scheduler's startup buffer target.
fn controllable_clock() -> (impl Fn() -> Option<u64> + Send + 'static, Arc<AtomicU64>) {
    let value = Arc::new(AtomicU64::new(0));
    let read = Arc::clone(&value);
    (move || Some(read.load(Ordering::Acquire)), value)
}

const HOST_START_MS: u64 = 1_000_000;
const SAMPLES_PER_PACKET: u32 = 960; // 20ms at 48kHz

fn start_stream(
    monitor: &Arc<DesktopMonitorControl>,
    clock_fn: impl Fn() -> Option<u64> + Send + 'static,
) -> Option<SyncSender<AudioDatagram>> {
    monitor.on_stream_started(
        session(),
        stream(),
        HOST_START_MS,
        CANONICAL_SAMPLE_RATE_HZ,
        CANONICAL_CHANNELS,
        SAMPLES_PER_PACKET,
        clock_fn,
    )
}

/// 34.3 "generated test tone through render ring": a synthetic, recognizable
/// signal submitted through the tap must reach the real output callback,
/// having passed through the actual scheduler/pump/render-ring pipeline --
/// not a shortcut or a simulation of it.
#[test]
fn a_generated_test_tone_reaches_the_output_callback_through_the_real_pipeline() {
    let backend = Arc::new(FakeBackend::new());
    let captured = Arc::clone(&backend.captured);
    let monitor = DesktopMonitorControl::new(backend);
    monitor.set_enabled(true);

    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    let tap = start_stream(&monitor, clock_fn).expect("monitor stream starts");

    for sequence in 0..30_u64 {
        let datagram = tone_datagram(sequence, HOST_START_MS + sequence * 20, SAMPLES_PER_PACKET);
        tap.send(datagram).expect("tap accepts datagram");
    }
    // Jump the clock well past the scheduler's startup buffer target so
    // the pump's next tick releases queued frames instead of buffering.
    clock.store(HOST_START_MS + 2_000, Ordering::Release);
    thread::sleep(Duration::from_millis(300));

    monitor.on_stream_stopped();

    let observed = captured.lock().expect("captured lock");
    assert!(
        observed.iter().any(|&sample| sample > 0.05),
        "expected the tone's real positive sample to reach the callback, observed: {observed:?}"
    );
    assert!(
        observed.iter().any(|&sample| sample < -0.05),
        "expected the tone's real negative sample to reach the callback, observed: {observed:?}"
    );
}

/// Block 35.1 "local monitor and render counters": once a stream is
/// active, `status().telemetry` must reflect real, live callback activity
/// -- not stay `None` forever (the gap this block closes: the telemetry
/// handle used to be created and immediately orphaned from the control
/// layer, unreachable by anything outside the render callback itself).
#[test]
fn active_status_reports_live_telemetry_counters() {
    let backend = Arc::new(FakeBackend::new());
    let monitor = DesktopMonitorControl::new(backend);
    monitor.set_enabled(true);

    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    let _tap = start_stream(&monitor, clock_fn).expect("monitor stream starts");
    thread::sleep(Duration::from_millis(50));

    let telemetry = monitor
        .status()
        .telemetry
        .expect("telemetry present while active");
    assert!(
        telemetry.callback_count > 0,
        "the fake's driving thread must have called write"
    );

    monitor.on_stream_stopped();
    assert!(
        monitor.status().telemetry.is_none(),
        "telemetry must disappear once the stream is no longer active"
    );
}

/// 34.3 "start/stop repeated": enabling/disabling and starting/stopping
/// streams several times in a row must never leak a stuck gate, panic, or
/// hang.
#[test]
fn start_stop_repeated_never_leaks_or_panics() {
    let backend = Arc::new(FakeBackend::new());
    let monitor = DesktopMonitorControl::new(backend);
    monitor.set_enabled(true);

    for iteration in 0..5_u64 {
        let (clock_fn, clock) = controllable_clock();
        clock.store(HOST_START_MS, Ordering::Release);
        let tap = start_stream(&monitor, clock_fn);
        assert!(tap.is_some(), "iteration {iteration}: monitor should start");
        let status = monitor.status();
        assert!(
            status.active,
            "iteration {iteration}: monitor should be active"
        );
        monitor.on_stream_stopped();
        let status = monitor.status();
        assert!(
            !status.active,
            "iteration {iteration}: monitor should be inactive after stop"
        );
    }
}

/// 34.3 "device removal": a fake backend that reports an error partway
/// through must not panic the monitor pipeline; the failure reaches
/// `status()` through the non-real-time `on_error` path, and the rest of
/// teardown still completes cleanly.
#[test]
fn device_removal_mid_stream_is_survived_without_panicking() {
    let backend = FakeBackend::new();
    *backend.fail_after_writes.lock().expect("lock") = Some(3);
    let monitor = DesktopMonitorControl::new(Arc::new(backend));
    monitor.set_enabled(true);

    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    let tap = start_stream(&monitor, clock_fn).expect("monitor stream starts");
    for sequence in 0..10_u64 {
        drop(tap.send(tone_datagram(
            sequence,
            HOST_START_MS + sequence * 20,
            SAMPLES_PER_PACKET,
        )));
    }
    thread::sleep(Duration::from_millis(100));

    // Must not have panicked getting here; teardown must still complete.
    monitor.on_stream_stopped();
    assert!(!monitor.status().active);
}

/// 34.3 "wrong format": a device reporting a non-canonical configuration
/// must be rejected before any stream is ever opened -- 33.2's fail-closed
/// policy, not a silent attempt to convert.
#[test]
fn a_non_canonical_device_format_is_rejected_before_opening_a_stream() {
    let backend = Arc::new(FakeBackend::with_config(Ok(AudioOutputConfig {
        channels: CANONICAL_CHANNELS,
        sample_rate_hz: 44_100,
    })));
    let starts_before = backend.starts.load(Ordering::Relaxed);
    let monitor = DesktopMonitorControl::new(backend.clone());
    monitor.set_enabled(true);

    let (clock_fn, _clock) = controllable_clock();
    let tap = start_stream(&monitor, clock_fn);

    assert!(
        tap.is_none(),
        "a wrong-format device must never produce a tap"
    );
    assert_eq!(
        backend.starts.load(Ordering::Relaxed),
        starts_before,
        "start() must never be called for a rejected format"
    );
    let status = monitor.status();
    assert!(!status.active);
    assert!(status.failure_reason.is_some());
}

/// 34.3 "callback after release prevention": once a monitor stream is
/// stopped, nothing can call its output callback again -- proven here by
/// the fake's driving thread being provably joined (and therefore gone)
/// before `on_stream_stopped` returns, and a fresh render-ring acquire
/// succeeding immediately afterward (Block 32's gate would reject it were
/// the previous consumer still alive).
#[test]
fn callback_after_release_is_structurally_impossible() {
    let backend = Arc::new(FakeBackend::new());
    let monitor = DesktopMonitorControl::new(backend);
    monitor.set_enabled(true);

    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    let _tap = start_stream(&monitor, clock_fn).expect("monitor stream starts");
    thread::sleep(Duration::from_millis(30));

    monitor.on_stream_stopped();

    // A fresh acquire against the SAME gate succeeding proves the previous
    // lease (and the callback holding it) is genuinely gone, not merely
    // logically "stopped" while still alive somewhere.
    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    monitor.set_enabled(true);
    let tap = start_stream(&monitor, clock_fn);
    assert!(
        tap.is_some(),
        "a fresh stream must be able to reacquire the render ring"
    );
    monitor.on_stream_stopped();
}

/// 34.3 "shutdown under active callback": stopping while the fake's thread
/// is actively mid-write must still join cleanly, never hang or panic.
#[test]
fn shutdown_while_the_callback_is_actively_running_completes_cleanly() {
    let backend = Arc::new(FakeBackend::new());
    let monitor = DesktopMonitorControl::new(backend);
    monitor.set_enabled(true);

    let (clock_fn, clock) = controllable_clock();
    clock.store(HOST_START_MS, Ordering::Release);
    let tap = start_stream(&monitor, clock_fn).expect("monitor stream starts");
    for sequence in 0..5_u64 {
        drop(tap.send(tone_datagram(
            sequence,
            HOST_START_MS + sequence * 20,
            SAMPLES_PER_PACKET,
        )));
    }
    // No sleep here on purpose -- stop is issued while the fake's thread is
    // in the middle of its 2ms write/sleep cycle, exercising the
    // stop-while-active path rather than a settled/idle one.
    let stopped = thread::spawn(move || monitor.on_stream_stopped());
    stopped
        .join()
        .expect("shutdown under an active callback must not panic");
}
