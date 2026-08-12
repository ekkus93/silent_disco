//! Coordinates the desktop host's optional local monitor: an explicit
//! on/off preference (Block 34.2 "transmit-only default" -- off unless
//! turned on), the render-ring consumer gate (Block 32), the presentation-
//! time-scheduled monitor pump (`monitor_pump.rs`), and the real audio
//! output stream (`audio_device.rs`).
//!
//! Nothing in this module is authoritative session state: monitor on/off
//! affects only what the person at this desktop machine hears, never what
//! any listener receives, so -- exactly like mDNS publication status
//! (`mdns.rs`) and unlike `PlaybackState` -- it stays a desktop-platform-
//! local concern and never becomes `CoreCommand`/`AudioEvent`/`CoreSnapshot`
//! domain state.
//!
//! **Lifecycle policy, recorded here because it is the one real
//! simplification this block makes deliberately**: enabling the monitor
//! only takes effect the next time a stream starts (it does not reach back
//! into a song already playing); disabling it takes effect immediately,
//! tearing down any active monitor stream right away. This keeps the
//! monitor's own lifecycle tied entirely to the playback stream's lifecycle
//! it is monitoring, with no reattachment/rebind machinery to build for a
//! Phase-9-optional feature.

use super::audio_device::{
    AudioOutputBackend, AudioOutputConfig, AudioOutputTelemetry, RenderCallback,
    RunningAudioOutputStream,
};
use super::monitor_pump::DesktopMonitorPump;
use super::render_ring::DesktopRenderRingGate;
use silent_disco_core::audio::{
    CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ, RenderRingConfig, SchedulerConfig,
};
use silent_disco_core::domain::{SessionId, StreamId};
use silent_disco_core::protocol::AudioDatagram;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};

/// Bounded so a stalled/slow monitor can never grow unbounded memory --
/// matches the render ring's own one-second-ish scale of slack. A full
/// channel simply drops the newest datagram (34.2: a struggling monitor
/// must never affect host transmission, which never touches this channel).
const MONITOR_TAP_CAPACITY: usize = 64;

/// Current desktop monitor status, safe to surface to the frontend as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorStatus {
    pub(crate) enabled: bool,
    pub(crate) active: bool,
    pub(crate) failure_reason: Option<String>,
    /// Live render-callback telemetry, present only while `active` --
    /// Block 35.1 "local monitor and render counters" diagnostics.
    pub(crate) telemetry: Option<MonitorTelemetrySnapshot>,
}

/// A point-in-time read of one active monitor stream's atomic telemetry
/// (`audio_device::AudioOutputTelemetry`), safe to surface as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MonitorTelemetrySnapshot {
    pub(crate) callback_count: u64,
    pub(crate) frames_written: u64,
    pub(crate) frames_silence_filled: u64,
}

struct ActiveMonitorStream {
    pump: DesktopMonitorPump,
    output: Box<dyn RunningAudioOutputStream>,
    /// Retained so `status()` can report live counters -- `start_stream`
    /// used to create this and hand it only to the render callback, leaving
    /// no way to ever read it back (a real gap this block closes: without
    /// this, "local monitor and render counters" diagnostics would be
    /// permanently unreachable, not merely unbuilt).
    telemetry: Arc<AudioOutputTelemetry>,
    /// The audio backend's error callback cannot lock `MonitorState`: its
    /// calling context is backend-owned and must never be allowed to block
    /// the host's monitor/status mutex. A write-once cell retains the first
    /// runtime failure so ordinary status readers can surface it later.
    runtime_failure: Arc<OnceLock<String>>,
}

impl ActiveMonitorStream {
    fn runtime_failure(&self) -> Option<String> {
        self.runtime_failure.get().cloned()
    }

    fn stop(self) -> Option<String> {
        let runtime_failure = self.runtime_failure();
        let Self {
            pump,
            output,
            telemetry: _,
            runtime_failure: _,
        } = self;
        let pump_failure = pump.stop().err();
        output.stop();
        match (runtime_failure, pump_failure) {
            (Some(runtime), Some(cleanup)) => {
                Some(format!("{runtime}; monitor cleanup failed: {cleanup}"))
            }
            (Some(runtime), None) => Some(runtime),
            (None, Some(cleanup)) => Some(cleanup),
            (None, None) => None,
        }
    }
}

struct MonitorState {
    enabled: bool,
    active: Option<ActiveMonitorStream>,
    failure_reason: Option<String>,
}

pub(crate) struct DesktopMonitorControl {
    gate: Arc<DesktopRenderRingGate>,
    backend: Arc<dyn AudioOutputBackend>,
    state: Mutex<MonitorState>,
}

impl DesktopMonitorControl {
    #[must_use]
    pub(crate) fn new(backend: Arc<dyn AudioOutputBackend>) -> Arc<Self> {
        Arc::new(Self {
            gate: DesktopRenderRingGate::new(),
            backend,
            state: Mutex::new(MonitorState {
                enabled: false,
                active: None,
                failure_reason: None,
            }),
        })
    }

    /// Sets the user's monitor preference. Disabling takes effect
    /// immediately (tears down any active stream); enabling only arms the
    /// preference for the next stream start -- see the module doc comment.
    pub(crate) fn set_enabled(&self, enabled: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.enabled = enabled;
        if !enabled {
            if let Some(active) = state.active.take() {
                drop(state);
                let failure = active.stop();
                if let Ok(mut state) = self.state.lock() {
                    state.failure_reason = failure;
                }
            } else {
                state.failure_reason = None;
            }
        }
    }

    #[must_use]
    pub(crate) fn status(&self) -> MonitorStatus {
        let Ok(state) = self.state.lock() else {
            return MonitorStatus {
                enabled: false,
                active: false,
                failure_reason: Some("monitor state is unavailable".to_owned()),
                telemetry: None,
            };
        };
        let runtime_failure = state
            .active
            .as_ref()
            .and_then(ActiveMonitorStream::runtime_failure);
        let active = state.active.is_some() && runtime_failure.is_none();
        MonitorStatus {
            enabled: state.enabled,
            active,
            failure_reason: runtime_failure.or_else(|| state.failure_reason.clone()),
            telemetry: if active {
                state.active.as_ref().map(|active| MonitorTelemetrySnapshot {
                    callback_count: active.telemetry.callback_count.load(Ordering::Relaxed),
                    frames_written: active.telemetry.frames_written.load(Ordering::Relaxed),
                    frames_silence_filled: active
                        .telemetry
                        .frames_silence_filled
                        .load(Ordering::Relaxed),
                })
            } else {
                None
            },
        }
    }

    /// Called once per stream start. If the monitor is enabled, attempts to
    /// stand up a full monitor stream (render-ring lease, output device,
    /// scheduled pump) for it and returns a tap the caller's playback pump
    /// can forward decoded audio through. Any failure at any step is
    /// recorded and reported via [`Self::status`] -- never propagated to
    /// the caller, which must keep transmitting to listeners regardless
    /// (34.2 policy).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_stream_started(
        &self,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        now_ms: impl Fn() -> Option<u64> + Send + 'static,
    ) -> Option<SyncSender<AudioDatagram>> {
        let mut state = self.state.lock().ok()?;
        if !state.enabled {
            return None;
        }
        // A previous stream's monitor must already be gone by the time a
        // new one starts (playback itself guarantees streams do not
        // overlap), but tear down defensively rather than leaking a lease.
        if let Some(previous) = state.active.take() {
            drop(state);
            let cleanup_failure = previous.stop();
            state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return None,
            };
            if let Some(failure) = cleanup_failure {
                state.failure_reason = Some(failure);
                return None;
            }
        }

        match self.start_stream(
            session_id,
            stream_id,
            host_start_time_ms,
            sample_rate,
            channels,
            samples_per_packet,
            now_ms,
        ) {
            Ok((active, tap)) => {
                state.active = Some(active);
                state.failure_reason = None;
                Some(tap)
            }
            Err(reason) => {
                state.failure_reason = Some(reason);
                None
            }
        }
    }

    /// Called once per stream stop, unconditionally. Tears down the active
    /// monitor stream if there is one; a no-op otherwise. Blocks until the
    /// monitor pump and output stream are both quiescent before returning
    /// (34.1 "callback is quiescent before consumer release").
    pub(crate) fn on_stream_stopped(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let Some(active) = state.active.take() else {
            return;
        };
        drop(state);
        let failure = active.stop();
        if let Ok(mut state) = self.state.lock() {
            state.failure_reason = failure;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_stream(
        &self,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        now_ms: impl Fn() -> Option<u64> + Send + 'static,
    ) -> Result<(ActiveMonitorStream, SyncSender<AudioDatagram>), String> {
        let device_config = self
            .backend
            .default_output_config()
            .map_err(|error| error.to_string())?;
        // 33.2 policy: require a device config that already matches the
        // canonical render format; fail closed rather than silently
        // resampling. No resampler exists for this path today.
        if device_config != canonical_output_config() {
            return Err(format!(
                "output device offers {}ch/{}Hz, not the required {}ch/{}Hz canonical format",
                device_config.channels,
                device_config.sample_rate_hz,
                CANONICAL_CHANNELS,
                CANONICAL_SAMPLE_RATE_HZ
            ));
        }
        if sample_rate != CANONICAL_SAMPLE_RATE_HZ || channels != CANONICAL_CHANNELS {
            return Err(format!(
                "stream is {channels}ch/{sample_rate}Hz, not the required \
                 {CANONICAL_CHANNELS}ch/{CANONICAL_SAMPLE_RATE_HZ}Hz canonical format"
            ));
        }
        let packet_duration_ms = if sample_rate == 0 {
            0
        } else {
            u32::try_from(
                u64::from(samples_per_packet).saturating_mul(1_000) / u64::from(sample_rate),
            )
            .unwrap_or(u32::MAX)
        };
        let scheduler_config = SchedulerConfig::new(
            session_id,
            stream_id,
            packet_duration_ms,
            host_start_time_ms,
            samples_per_packet,
            channels,
        );

        let (producer, lease) = self
            .gate
            .acquire(RenderRingConfig::default())
            .map_err(|error| error.to_string())?;

        let (sender, receiver) = sync_channel::<AudioDatagram>(MONITOR_TAP_CAPACITY);
        let pump = match DesktopMonitorPump::start(scheduler_config, producer, receiver, now_ms) {
            Ok(pump) => pump,
            Err(error) => {
                drop(lease);
                return Err(error.to_string());
            }
        };

        let telemetry = Arc::new(AudioOutputTelemetry::default());
        let callback = RenderCallback::new(lease, Arc::clone(&telemetry));
        let runtime_failure = Arc::new(OnceLock::new());
        let runtime_failure_for_callback = Arc::clone(&runtime_failure);
        let output = match self.backend.start(
            device_config,
            callback,
            Box::new(move |message| {
                // This is the backend's non-real-time error callback, not
                // `RenderCallback::write`. Never take the monitor mutex here;
                // retain the first actionable runtime cause in a write-once
                // cell and let ordinary status readers expose it.
                drop(runtime_failure_for_callback.set(format!(
                    "local monitor audio device failed: {message}"
                )));
            }),
        ) {
            Ok(output) => output,
            Err(error) => {
                let primary = error.to_string();
                return Err(match pump.stop() {
                    Ok(()) => primary,
                    Err(cleanup) => format!("{primary}; monitor cleanup failed: {cleanup}"),
                });
            }
        };

        Ok((
            ActiveMonitorStream {
                pump,
                output,
                telemetry,
                runtime_failure,
            },
            sender,
        ))
    }
}

fn canonical_output_config() -> AudioOutputConfig {
    AudioOutputConfig {
        channels: CANONICAL_CHANNELS,
        sample_rate_hz: CANONICAL_SAMPLE_RATE_HZ,
    }
}
