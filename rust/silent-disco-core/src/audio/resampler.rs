use super::types::{
    CANONICAL_SAMPLE_RATE_HZ, DecodeError, DecodeErrorKind, MAX_SOURCE_SAMPLE_RATE_HZ,
    MIN_SOURCE_SAMPLE_RATE_HZ,
};

/// Incremental linear stereo resampler retaining only one prior source frame.
pub(super) struct StereoResampler {
    source_rate_hz: u32,
    next_output_frame: u64,
    source_frames_seen: u64,
    previous_frame: Option<[f32; 2]>,
}

impl StereoResampler {
    pub(super) fn new(source_rate_hz: u32) -> Result<Self, DecodeError> {
        if !(MIN_SOURCE_SAMPLE_RATE_HZ..=MAX_SOURCE_SAMPLE_RATE_HZ).contains(&source_rate_hz) {
            return Err(DecodeError::new(
                DecodeErrorKind::UnsupportedFormat,
                "source sample rate is outside the supported 8 kHz to 192 kHz range",
            ));
        }
        Ok(Self {
            source_rate_hz,
            next_output_frame: 0,
            source_frames_seen: 0,
            previous_frame: None,
        })
    }

    pub(super) fn push(
        &mut self,
        frames: &[[f32; 2]],
        end_of_stream: bool,
        mut output: impl FnMut([f32; 2]) -> Result<(), DecodeError>,
    ) -> Result<(), DecodeError> {
        let batch_start = self.source_frames_seen;
        let batch_frames = u64::try_from(frames.len()).map_err(|_| {
            DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "decoded source frame count does not fit the supported index range",
            )
        })?;
        let batch_end = batch_start.checked_add(batch_frames).ok_or_else(|| {
            DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "decoded source frame index overflowed",
            )
        })?;

        loop {
            let position_numerator = u128::from(self.next_output_frame)
                .checked_mul(u128::from(self.source_rate_hz))
                .ok_or_else(|| {
                    DecodeError::new(
                        DecodeErrorKind::ResourceLimit,
                        "resampler position arithmetic overflowed",
                    )
                })?;
            let target_rate = u128::from(CANONICAL_SAMPLE_RATE_HZ);
            let base = u64::try_from(position_numerator / target_rate).map_err(|_| {
                DecodeError::new(
                    DecodeErrorKind::ResourceLimit,
                    "resampler source position exceeded the supported range",
                )
            })?;
            let next = base.checked_add(1).ok_or_else(|| {
                DecodeError::new(
                    DecodeErrorKind::ResourceLimit,
                    "resampler interpolation index overflowed",
                )
            })?;

            if end_of_stream {
                let stream_numerator =
                    u128::from(batch_end)
                        .checked_mul(target_rate)
                        .ok_or_else(|| {
                            DecodeError::new(
                                DecodeErrorKind::ResourceLimit,
                                "resampler stream duration arithmetic overflowed",
                            )
                        })?;
                if batch_end == 0 || position_numerator >= stream_numerator {
                    break;
                }
            } else if next >= batch_end {
                break;
            }

            let base_frame = self.frame_at(base, batch_start, frames)?;
            let next_frame = if next < batch_end {
                self.frame_at(next, batch_start, frames)?
            } else {
                base_frame
            };
            #[allow(
                clippy::cast_precision_loss,
                reason = "linear interpolation intentionally converts a bounded integer remainder to f32"
            )]
            let fraction =
                (position_numerator % target_rate) as f32 / CANONICAL_SAMPLE_RATE_HZ as f32;
            output([
                base_frame[0] + (next_frame[0] - base_frame[0]) * fraction,
                base_frame[1] + (next_frame[1] - base_frame[1]) * fraction,
            ])?;
            self.next_output_frame = self.next_output_frame.checked_add(1).ok_or_else(|| {
                DecodeError::new(
                    DecodeErrorKind::ResourceLimit,
                    "canonical output frame index overflowed",
                )
            })?;
        }

        if let Some(last) = frames.last().copied() {
            self.previous_frame = Some(last);
        }
        self.source_frames_seen = batch_end;
        Ok(())
    }

    fn frame_at(
        &self,
        absolute_index: u64,
        batch_start: u64,
        frames: &[[f32; 2]],
    ) -> Result<[f32; 2], DecodeError> {
        if absolute_index < batch_start {
            if absolute_index.checked_add(1) == Some(batch_start) {
                return self.previous_frame.ok_or_else(|| {
                    DecodeError::new(
                        DecodeErrorKind::CorruptInput,
                        "resampler is missing the previous source frame",
                    )
                });
            }
            return Err(DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "resampler requested source data outside the bounded window",
            ));
        }
        let local = usize::try_from(absolute_index - batch_start).map_err(|_| {
            DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "resampler local source index exceeded the supported range",
            )
        })?;
        frames.get(local).copied().ok_or_else(|| {
            DecodeError::new(
                DecodeErrorKind::CorruptInput,
                "resampler requested a missing decoded source frame",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::StereoResampler;

    #[test]
    fn resamples_across_input_boundaries_without_losing_continuity() {
        let mut resampler = StereoResampler::new(24_000).expect("rate");
        let mut output = Vec::new();
        resampler
            .push(&[[0.0, 0.0], [0.5, 0.5]], false, |frame| {
                output.push(frame);
                Ok(())
            })
            .expect("first batch");
        resampler
            .push(&[[1.0, 1.0]], true, |frame| {
                output.push(frame);
                Ok(())
            })
            .expect("final batch");
        assert_eq!(output.len(), 6);
        assert!((output[1][0] - 0.25).abs() < 0.001);
        assert!((output[3][0] - 0.75).abs() < 0.001);
    }

    #[test]
    fn downsampling_emits_the_checked_duration() {
        let mut resampler = StereoResampler::new(96_000).expect("rate");
        let source = vec![[0.5, -0.5]; 200];
        let mut output = Vec::new();
        resampler
            .push(&source, true, |frame| {
                output.push(frame);
                Ok(())
            })
            .expect("resample");
        assert_eq!(output.len(), 100);
    }
}
