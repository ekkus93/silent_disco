use super::ProbeResult;
use crate::platform::audio_device::{AudioOutputTelemetry, RenderCallback};
use crate::platform::render_ring::DesktopRenderRingGate;
use serde::Serialize;
use silent_disco_core::audio::{RENDER_CHANNELS, RenderRingConfig};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const CALLBACK_FRAMES: usize = 240;
const CALLBACK_ITERATIONS: u64 = 5_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MonitorCallbackMetric {
    callback_iterations: u64,
    frames_per_callback: usize,
    average_callback_nanos: u64,
    max_callback_nanos: u64,
    callbacks_observed: u64,
    frames_written: u64,
    frames_silence_filled: u64,
    ring_underrun_callbacks: u64,
}

pub(super) fn measure_monitor_callback() -> ProbeResult<MonitorCallbackMetric> {
    let gate = DesktopRenderRingGate::new();
    let (producer, lease) = gate.acquire(RenderRingConfig::default())?;
    let telemetry = Arc::new(AudioOutputTelemetry::default());
    let mut callback = RenderCallback::new(lease, Arc::clone(&telemetry));
    let input = vec![0.125_f32; CALLBACK_FRAMES.saturating_mul(RENDER_CHANNELS)];
    let mut output = vec![0.0_f32; CALLBACK_FRAMES.saturating_mul(RENDER_CHANNELS)];
    let mut total_nanos = 0_u64;
    let mut max_nanos = 0_u64;

    for _ in 0..CALLBACK_ITERATIONS {
        let frames_written = producer.push_frames(&input);
        if frames_written != CALLBACK_FRAMES {
            return Err(format!(
                "monitor render ring accepted {frames_written} of {CALLBACK_FRAMES} frames"
            )
            .into());
        }
        let started = Instant::now();
        callback.write(&mut output);
        let elapsed = duration_nanos(started.elapsed())?;
        total_nanos = total_nanos.saturating_add(elapsed);
        max_nanos = max_nanos.max(elapsed);
    }

    callback.write(&mut output); // deliberate empty-ring underrun
    let ring = producer.telemetry();
    let callbacks_observed = telemetry.callback_count.load(Ordering::Relaxed);
    let frames_written = telemetry.frames_written.load(Ordering::Relaxed);
    let frames_silence_filled = telemetry.frames_silence_filled.load(Ordering::Relaxed);
    let expected_callbacks = CALLBACK_ITERATIONS.saturating_add(1);
    let expected_written = CALLBACK_ITERATIONS.saturating_mul(u64::try_from(CALLBACK_FRAMES)?);
    if callbacks_observed != expected_callbacks
        || frames_written != expected_written
        || frames_silence_filled != u64::try_from(CALLBACK_FRAMES)?
        || ring.underrun_callbacks != 1
    {
        return Err(format!(
            "monitor callback accounting mismatch: callbacks={callbacks_observed}/{expected_callbacks}, \
             written={frames_written}/{expected_written}, silence={frames_silence_filled}, \
             underruns={}",
            ring.underrun_callbacks
        )
        .into());
    }

    Ok(MonitorCallbackMetric {
        callback_iterations: CALLBACK_ITERATIONS,
        frames_per_callback: CALLBACK_FRAMES,
        average_callback_nanos: total_nanos / CALLBACK_ITERATIONS.max(1),
        max_callback_nanos: max_nanos,
        callbacks_observed,
        frames_written,
        frames_silence_filled,
        ring_underrun_callbacks: ring.underrun_callbacks,
    })
}

fn duration_nanos(duration: Duration) -> ProbeResult<u64> {
    Ok(u64::try_from(duration.as_nanos())?)
}
