//! Local desktop monitor output adapter (Block 34.1).
//!
//! Everything below the device-enumeration/stream-negotiation boundary
//! exists to keep the real-time audio callback ([`RenderCallback::write`])
//! to exactly what 34.1 allows: a bounded render-ring read and silence
//! fill, atomic telemetry only, no allocation, logging, Tauri, `SQLite`,
//! file, network, or blocking work. Device enumeration and stream
//! configuration both happen entirely outside the callback, in
//! [`AudioOutputBackend::default_output_config`]/`start`.
//!
//! [`AudioOutputBackend`] is a trait, not a direct `cpal` dependency here,
//! so tests can exercise [`RenderCallback`] and the rest of this module's
//! logic (underrun/silence-fill, device removal, wrong format, callback-
//! after-release, shutdown-under-active-callback -- 34.3) deterministically
//! with a fake backend, without needing real audio hardware, mirroring this
//! codebase's established DI pattern (`MdnsPublisher`,
//! `DesktopIdentityProvider`, `TransportFactory`).

use super::failure::DesktopPlatformFailure;
use super::render_ring::DesktopRenderConsumerLease;
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::PlatformEffectRequest;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Returns a visible failure for audio effects that require the secure source registry or
/// native output implementation delivered by later desktop blocks.
#[must_use]
pub(super) fn unsupported_effect(request: &PlatformEffectRequest) -> DesktopPlatformFailure {
    match request {
        PlatformEffectRequest::PrepareAudioSource(_) => DesktopPlatformFailure::new(
            CoreErrorCode::CapabilityUnavailable,
            "desktop audio source preparation requires the secure source-selection block",
            ErrorSeverity::Error,
            false,
        ),
        PlatformEffectRequest::StartAudioOutput(_) | PlatformEffectRequest::StopAudioOutput => {
            DesktopPlatformFailure::new(
                CoreErrorCode::AudioEngineUnavailable,
                "desktop native audio output is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::RequestCapabilities(_)
        | PlatformEffectRequest::StartAdvertising(_)
        | PlatformEffectRequest::StopAdvertising
        | PlatformEffectRequest::StartDiscovery(_)
        | PlatformEffectRequest::StopDiscovery
        | PlatformEffectRequest::EstablishNetwork(_)
        | PlatformEffectRequest::ReleaseNetwork
        | PlatformEffectRequest::ShareDiagnostics { .. } => DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "effect was routed to the wrong desktop audio adapter",
            ErrorSeverity::Fatal,
            false,
        ),
    }
}

/// Backend-agnostic negotiated output stream configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioOutputConfig {
    pub(crate) channels: u16,
    pub(crate) sample_rate_hz: u32,
}

/// Stable failure taxonomy for the local monitor output adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AudioOutputError {
    /// No default output device is available (e.g. a headless system).
    NoDefaultDevice,
    /// The device's default output configuration could not be read.
    ConfigNegotiationFailed(String),
    /// The negotiated configuration is not the project's canonical
    /// 48kHz/stereo/float32 render format, and no conversion is
    /// implemented (33.2 policy: fail closed rather than silently convert).
    UnsupportedFormat { channels: u16, sample_rate_hz: u32 },
    /// The backend rejected building the output stream.
    StreamBuildFailed(String),
    /// The backend rejected starting playback on an otherwise-built stream.
    StreamPlayFailed(String),
}

impl fmt::Display for AudioOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDefaultDevice => {
                formatter.write_str("no default audio output device is available")
            }
            Self::ConfigNegotiationFailed(message) => {
                write!(
                    formatter,
                    "could not negotiate an output configuration: {message}"
                )
            }
            Self::UnsupportedFormat {
                channels,
                sample_rate_hz,
            } => write!(
                formatter,
                "output device offers {channels}ch/{sample_rate_hz}Hz, not the required \
                 2ch/48000Hz canonical format"
            ),
            Self::StreamBuildFailed(message) => {
                write!(formatter, "could not build the output stream: {message}")
            }
            Self::StreamPlayFailed(message) => {
                write!(formatter, "could not start the output stream: {message}")
            }
        }
    }
}

impl std::error::Error for AudioOutputError {}

/// Atomic-only telemetry for one running monitor output stream (34.1
/// "atomic telemetry only").
#[derive(Debug, Default)]
pub(crate) struct AudioOutputTelemetry {
    pub(crate) callback_count: AtomicU64,
    pub(crate) frames_written: AtomicU64,
    pub(crate) frames_silence_filled: AtomicU64,
}

impl AudioOutputTelemetry {
    fn record(&self, frames_supplied: usize, frames_silence_filled: usize) {
        self.callback_count.fetch_add(1, Ordering::Relaxed);
        self.frames_written.fetch_add(
            u64::try_from(frames_supplied).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.frames_silence_filled.fetch_add(
            u64::try_from(frames_silence_filled).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }
}

/// The real-time-safe callback body itself, shared verbatim between the
/// production `cpal` backend and any test fake -- so a test exercises the
/// actual callback logic, not a reimplementation of it.
pub(crate) struct RenderCallback {
    lease: DesktopRenderConsumerLease,
    telemetry: Arc<AudioOutputTelemetry>,
}

impl RenderCallback {
    pub(crate) fn new(
        lease: DesktopRenderConsumerLease,
        telemetry: Arc<AudioOutputTelemetry>,
    ) -> Self {
        Self { lease, telemetry }
    }

    /// A bounded render-ring read and silence fill, and atomic telemetry --
    /// nothing else. No allocation, logging, Tauri, `SQLite`, file,
    /// network, or blocking work (34.1).
    pub(crate) fn write(&mut self, output: &mut [f32]) {
        let outcome = self.lease.consumer_mut().read_frames(output);
        self.telemetry
            .record(outcome.frames_supplied, outcome.frames_silence_filled);
    }
}

/// One running output stream, owned until explicitly stopped.
///
/// `stop` consumes `self` and blocks until the underlying stream is
/// quiescent -- there is no way to call the real-time callback again on a
/// stopped stream, matching 34.1's "callback is quiescent before consumer
/// release" and 34.3's "callback after release prevention".
pub(crate) trait RunningAudioOutputStream: Send {
    fn stop(self: Box<Self>);
}

/// Enumerates/negotiates/opens local audio output. Implemented by
/// [`CpalAudioOutputBackend`] in production and by a fake in tests.
pub(crate) trait AudioOutputBackend: Send + Sync {
    /// Reads the default output device's configuration.
    ///
    /// # Errors
    ///
    /// Returns [`AudioOutputError::NoDefaultDevice`] if no output device is
    /// available (e.g. a headless system -- 34.2 "no fake monitor success
    /// on headless systems"), or
    /// [`AudioOutputError::ConfigNegotiationFailed`] if the device's
    /// configuration could not be read.
    fn default_output_config(&self) -> Result<AudioOutputConfig, AudioOutputError>;

    /// Opens and starts one output stream at `config`, calling
    /// `callback.write` on the backend's own real-time thread and `on_error`
    /// (never from that real-time thread) if the stream fails after
    /// starting.
    ///
    /// # Errors
    ///
    /// Returns [`AudioOutputError::StreamBuildFailed`] or
    /// [`AudioOutputError::StreamPlayFailed`] if the backend rejects the
    /// stream.
    fn start(
        &self,
        config: AudioOutputConfig,
        callback: RenderCallback,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn RunningAudioOutputStream>, AudioOutputError>;
}

/// Production backend, selected and validated in Block 33.
pub(crate) struct CpalAudioOutputBackend;

impl AudioOutputBackend for CpalAudioOutputBackend {
    fn default_output_config(&self) -> Result<AudioOutputConfig, AudioOutputError> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioOutputError::NoDefaultDevice)?;
        let config = device
            .default_output_config()
            .map_err(|error| AudioOutputError::ConfigNegotiationFailed(error.to_string()))?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            return Err(AudioOutputError::UnsupportedFormat {
                channels: config.channels(),
                sample_rate_hz: config.sample_rate(),
            });
        }
        Ok(AudioOutputConfig {
            channels: config.channels(),
            sample_rate_hz: config.sample_rate(),
        })
    }

    fn start(
        &self,
        config: AudioOutputConfig,
        mut callback: RenderCallback,
        on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn RunningAudioOutputStream>, AudioOutputError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioOutputError::NoDefaultDevice)?;
        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: config.sample_rate_hz,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = device
            .build_output_stream(
                stream_config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| callback.write(data),
                move |error| on_error(error.to_string()),
                None,
            )
            .map_err(|error| AudioOutputError::StreamBuildFailed(error.to_string()))?;
        stream
            .play()
            .map_err(|error| AudioOutputError::StreamPlayFailed(error.to_string()))?;
        Ok(Box::new(CpalRunningStream { stream }))
    }
}

struct CpalRunningStream {
    stream: cpal::Stream,
}

impl RunningAudioOutputStream for CpalRunningStream {
    fn stop(self: Box<Self>) {
        // Block 33's real spike on this machine confirmed dropping a
        // playing cpal stream completes cleanly and quiescently within a
        // bounded join, never hangs or panics.
        drop(self.stream);
    }
}

/// Default backend: always reports no default device available, exactly
/// how a genuinely headless system behaves (34.2 "no fake monitor success
/// on headless systems"). Used as `DesktopMonitorControl`'s default so a
/// test double must be explicitly injected to ever observe a monitor
/// actually becoming active -- mirrors `NullMdnsPublisher`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NullAudioOutputBackend;

impl AudioOutputBackend for NullAudioOutputBackend {
    fn default_output_config(&self) -> Result<AudioOutputConfig, AudioOutputError> {
        Err(AudioOutputError::NoDefaultDevice)
    }

    fn start(
        &self,
        _config: AudioOutputConfig,
        _callback: RenderCallback,
        _on_error: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<Box<dyn RunningAudioOutputStream>, AudioOutputError> {
        Err(AudioOutputError::NoDefaultDevice)
    }
}

#[cfg(test)]
mod tests {
    use super::super::render_ring::DesktopRenderRingGate;
    use super::{AudioOutputTelemetry, RenderCallback};
    use silent_disco_core::audio::RenderRingConfig;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// 34.3 "underrun and silence fill": a callback reading from a ring
    /// nothing has ever been pushed into must produce silence, not garbage
    /// or a panic, and must record it as silence-filled telemetry -- never
    /// silently count it as real content delivered.
    #[test]
    fn an_empty_ring_produces_silence_and_records_it_as_such() {
        let gate = DesktopRenderRingGate::new();
        let (_producer, lease) = gate.acquire(RenderRingConfig::default()).expect("acquire");
        let telemetry = Arc::new(AudioOutputTelemetry::default());
        let mut callback = RenderCallback::new(lease, Arc::clone(&telemetry));

        let mut output = [1.0_f32, 1.0, 1.0, 1.0];
        callback.write(&mut output);

        assert_eq!(output, [0.0_f32; 4]);
        assert_eq!(telemetry.callback_count.load(Ordering::Relaxed), 1);
        assert_eq!(telemetry.frames_written.load(Ordering::Relaxed), 0);
        assert_eq!(telemetry.frames_silence_filled.load(Ordering::Relaxed), 2);
    }

    /// Frames genuinely pushed into the ring must be read back exactly, and
    /// counted as written -- not silence-filled.
    #[test]
    fn pushed_frames_are_read_back_exactly_and_counted_as_written() {
        let gate = DesktopRenderRingGate::new();
        let (producer, lease) = gate.acquire(RenderRingConfig::default()).expect("acquire");
        let telemetry = Arc::new(AudioOutputTelemetry::default());

        let pushed = [0.25_f32, -0.25, 0.5, -0.5];
        assert_eq!(producer.push_frames(&pushed), 2);

        let mut callback = RenderCallback::new(lease, Arc::clone(&telemetry));
        let mut output = [0.0_f32; 4];
        callback.write(&mut output);

        assert_eq!(output, pushed);
        assert_eq!(telemetry.frames_written.load(Ordering::Relaxed), 2);
        assert_eq!(telemetry.frames_silence_filled.load(Ordering::Relaxed), 0);
    }

    /// `callback_count` must increment exactly once per `write` call, no
    /// matter how much or how little audio that call actually supplied --
    /// it counts real-time invocations, not frames.
    #[test]
    fn callback_count_increments_once_per_write_regardless_of_content() {
        let gate = DesktopRenderRingGate::new();
        let (_producer, lease) = gate.acquire(RenderRingConfig::default()).expect("acquire");
        let telemetry = Arc::new(AudioOutputTelemetry::default());
        let mut callback = RenderCallback::new(lease, Arc::clone(&telemetry));

        let mut output = [0.0_f32; 2];
        for _ in 0..5 {
            callback.write(&mut output);
        }

        assert_eq!(telemetry.callback_count.load(Ordering::Relaxed), 5);
    }
}
