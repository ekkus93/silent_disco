//! PCM16-to-float32 conversion: normalizing one scheduled frame into the
//! render ring's interleaved float32 format.

use crate::audio::ScheduledFrame;

use super::pump::PlaybackPump;

/// Full-scale magnitude of a 16-bit PCM sample, used to normalize into the
/// render ring's float32 representation.
const PCM16_FULL_SCALE: f32 = 32_768.0;

impl PlaybackPump {
    /// Converts one frame and appends it to the pending queue, then pushes as
    /// much of that queue as the ring will take. Appending rather than
    /// replacing keeps playback order intact when an earlier frame was only
    /// partly accepted.
    pub(super) fn enqueue_frame(&mut self, frame: &ScheduledFrame) -> usize {
        self.record_frame(frame);
        self.pending.reserve(frame.samples.len());
        for &sample in &frame.samples {
            self.pending
                .push(f32::from(sample) / PCM16_FULL_SCALE * self.config.volume);
        }
        self.flush_pending()
    }
}
