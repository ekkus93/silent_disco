//! Optional debug capture of exactly what was released toward the ring: the
//! real, concealed, and drained frames a pump believed it was playing.

use crate::audio::{DebugPcmRecorder, ScheduledFrame};

use super::pump::PlaybackPump;

impl PlaybackPump {
    /// Captures every frame released toward the ring into `recorder`.
    ///
    /// Records real, concealed, and drained frames — exactly what this pump
    /// believed it was playing — but not the stream-start alignment silence,
    /// which is ring positioning rather than stream content.
    pub fn set_recorder(&mut self, recorder: DebugPcmRecorder) {
        self.recorder = Some(recorder);
    }

    /// First failure from the debug recorder, if capture stopped early.
    #[must_use]
    pub fn recorder_error(&self) -> Option<&str> {
        self.recorder_error.as_deref()
    }

    /// Appends one released frame to the debug capture, disabling capture on
    /// the first failure and retaining it for reporting.
    pub(super) fn record_frame(&mut self, frame: &ScheduledFrame) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        if let Err(error) = recorder.append(&frame.samples) {
            self.recorder_error = Some(format!("debug capture failed: {error}"));
            self.recorder = None;
        }
    }

    /// Finalizes the debug capture, if one is active.
    pub(super) fn finish_recording(&mut self) {
        let sample_rate = self.scheduler.sample_rate();
        if let Some(recorder) = self.recorder.as_mut()
            && let Err(error) = recorder.finish(sample_rate)
        {
            self.recorder_error = Some(format!("debug capture failed to finalize: {error}"));
        }
        self.recorder = None;
    }
}
