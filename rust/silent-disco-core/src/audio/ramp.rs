//! Shared integer-PCM amplitude shaping for interleaved frames.
//!
//! Concealment and frame delivery both need to start or end a buffer at a
//! known amplitude so that splicing two independently produced buffers never
//! introduces a waveform discontinuity. A hard splice is audible as a click;
//! ramping over a few milliseconds keeps the seam inaudible without smearing
//! real content.

/// Frame counts are bounded by validated packet geometry (samples per packet
/// and channel count), orders of magnitude below `i64::MAX`.
#[allow(
    clippy::cast_possible_wrap,
    reason = "frame counts are bounded by validated packet geometry"
)]
const fn to_i64(value: usize) -> i64 {
    value as i64
}

fn clamp_to_i16(value: i64) -> i16 {
    if value > i64::from(i16::MAX) {
        i16::MAX
    } else if value < i64::from(i16::MIN) {
        i16::MIN
    } else {
        // Bounded by the two comparisons above.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "value is clamped into i16 range immediately above"
        )]
        {
            value as i16
        }
    }
}

/// Scales `sample` by `numerator / denominator`, saturating into `i16`.
/// A zero denominator yields zero rather than dividing.
pub(super) fn scale_sample(sample: i16, numerator: usize, denominator: usize) -> i16 {
    if denominator == 0 {
        return 0;
    }
    clamp_to_i16(i64::from(sample) * to_i64(numerator) / to_i64(denominator))
}

/// Linear blend between two sample values across a ramp: at `position` 0 the
/// result is `from`, at `position == ramp` it is `toward`.
pub(super) fn blend_sample(toward: i16, from: i16, position: usize, ramp: usize) -> i16 {
    if ramp == 0 {
        return toward;
    }
    let weighted =
        i64::from(toward) * to_i64(position) + i64::from(from) * to_i64(ramp - position.min(ramp));
    clamp_to_i16(weighted / to_i64(ramp))
}

/// Per-channel sample values of the final complete frame in `samples`, or
/// `None` when `samples` holds no complete frame.
pub(super) fn last_frame(samples: &[i16], channels: usize) -> Option<&[i16]> {
    if channels == 0 {
        return None;
    }
    let frames = samples.len() / channels;
    if frames == 0 {
        return None;
    }
    Some(&samples[(frames - 1) * channels..frames * channels])
}

/// Linearly fades the first `ramp_frames` frames of `samples` up from zero,
/// leaving later frames untouched. Used when real audio resumes after a gap
/// (or starts mid-waveform), so the resume seam is a short ramp rather than a
/// step.
pub(super) fn apply_fade_in(samples: &mut [i16], channels: usize, ramp_frames: usize) {
    if channels == 0 {
        return;
    }
    let total_frames = samples.len() / channels;
    if total_frames == 0 {
        return;
    }
    let ramp = ramp_frames.clamp(1, total_frames);
    for frame in 0..ramp {
        for channel in 0..channels {
            let index = frame * channels + channel;
            samples[index] = scale_sample(samples[index], frame, ramp);
        }
    }
}

/// Linearly fades the final `ramp_frames` frames of `samples` down to zero,
/// leaving earlier frames untouched. The ramp is clamped to the buffer's own
/// length, so a buffer shorter than the requested ramp simply fades across
/// its whole span.
pub(super) fn apply_fade_out_tail(samples: &mut [i16], channels: usize, ramp_frames: usize) {
    if channels == 0 {
        return;
    }
    let total_frames = samples.len() / channels;
    if total_frames == 0 {
        return;
    }
    let ramp = ramp_frames.clamp(1, total_frames);
    for frame in (total_frames - ramp)..total_frames {
        let remaining = total_frames - 1 - frame;
        for channel in 0..channels {
            let index = frame * channels + channel;
            samples[index] = scale_sample(samples[index], remaining, ramp);
        }
    }
}
