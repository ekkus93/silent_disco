//! The tick-driven pacing loop: what one [`PlaybackPump::tick`] does, the
//! stream-start alignment prefill, and pushing converted audio into the ring
//! as capacity allows.

use crate::audio::{RENDER_CHANNELS, ScheduledFrame, SchedulerPoll};

use super::pump::PlaybackPump;

/// What one [`PlaybackPump::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpTick {
    /// The scheduler is still accumulating its startup presentation buffer.
    Buffering {
        /// Buffered span accumulated so far, in milliseconds.
        buffered_ms: u64,
    },
    /// A frame was converted and queued into the render ring.
    Queued {
        /// Sequence of the frame queued.
        sequence: u64,
        /// Frames actually accepted by the ring this tick.
        frames: usize,
        /// True when the queued frame was synthesized rather than delivered.
        concealed: bool,
    },
    /// Part of an earlier frame that the ring had no room for was accepted
    /// this tick.
    FlushedPending {
        /// Frames accepted this tick.
        frames: usize,
    },
    /// The ring had no room for the frame in hand; the unwritten remainder is
    /// held and retried on the next tick rather than being dropped.
    RingFull {
        /// Frames still waiting to be accepted by the ring.
        pending_frames: usize,
    },
    /// The ring already holds the configured cushion; the pump is letting the
    /// consumer drain rather than writing further ahead.
    AtTargetDepth {
        /// Frames currently queued in the ring.
        queued_frames: usize,
    },
    /// Nothing is due for this tick.
    Waiting,
    /// No real clock-offset estimate has been applied yet, so no presentation
    /// time can be mapped and nothing may play.
    AwaitingSync,
    /// The scheduler paused and was re-armed to re-accumulate its startup
    /// buffer before playback resumes.
    AwaitingRebuffer,
    /// The scheduler has stopped and will produce no further frames.
    Stopped,
}

impl PlaybackPump {
    /// Advances playback by one tick at the given local monotonic time.
    pub fn tick(&mut self, local_now_ms: u64) -> PumpTick {
        // Nothing may play against a placeholder offset. A stream started on
        // one would map every presentation time through a meaningless
        // timeline and either dump the whole stream at once or drop all of it
        // as late — with no signal that anything is wrong.
        if !self.sync_locked {
            return PumpTick::AwaitingSync;
        }
        if !self.pending.is_empty() {
            // Never poll for new audio while an earlier frame is still
            // partly unwritten: that would reorder the stream.
            let flushed = self.flush_pending();
            return if self.pending.is_empty() {
                PumpTick::FlushedPending { frames: flushed }
            } else {
                PumpTick::RingFull {
                    pending_frames: self.pending_frames(),
                }
            };
        }

        // Hold the ring at its intended cushion. Beyond this the pump simply
        // waits: without the cap, a large startup backlog would run the ring
        // to capacity and stay there for the rest of the stream.
        let queued_frames = self.queued_frames();
        self.peak_queued_frames = self.peak_queued_frames.max(queued_frames);
        if queued_frames >= self.config.target_depth_frames {
            return PumpTick::AtTargetDepth { queued_frames };
        }

        // Release frames early by the configured lead. The ring's FIFO
        // position, not the moment of writing, decides when a frame is heard.
        let poll_time_ms = local_now_ms.saturating_add(self.config.write_lead_ms);
        match self
            .scheduler
            .poll_with_release_horizon(local_now_ms, poll_time_ms)
        {
            SchedulerPoll::Buffering { buffered_ms } => PumpTick::Buffering { buffered_ms },
            SchedulerPoll::Waiting { .. } => PumpTick::Waiting,
            SchedulerPoll::AwaitingRebuffer => {
                // Re-arm immediately: the pause exists to force a fresh
                // startup buffer, not to end playback. Without this the
                // stream would stay silent forever after one long outage.
                self.scheduler.rebuffer(self.offset_ms);
                self.awaiting_prefill = true;
                PumpTick::AwaitingRebuffer
            }
            SchedulerPoll::Stopped => PumpTick::Stopped,
            SchedulerPoll::Frame { frame, .. } => {
                let sequence = frame.sequence;
                let concealed = frame.concealed;
                self.queue_alignment_prefill(&frame, local_now_ms);
                let frames = self.enqueue_frame(&frame);
                if self.pending.is_empty() {
                    PumpTick::Queued {
                        sequence,
                        frames,
                        concealed,
                    }
                } else {
                    PumpTick::RingFull {
                        pending_frames: self.pending_frames(),
                    }
                }
            }
        }
    }

    /// Drains the scheduler's buffered tail into the ring and stops it.
    ///
    /// Returns the number of frames accepted by the ring. The ring may not
    /// have room for all of it at once; whatever does not fit is held pending
    /// and can be flushed with further [`Self::tick`] calls before the output
    /// is torn down. The held amount is bounded by the drained tail itself,
    /// which the jitter buffer already bounds.
    pub fn finish(&mut self) -> usize {
        let drained = self.scheduler.drain_remaining();
        let mut written = 0;
        for frame in &drained {
            written += self.enqueue_frame(frame);
        }
        self.scheduler.stop();
        self.finish_recording();
        written
    }

    /// Queues silence ahead of the stream's first frame so that frame is heard
    /// at its presentation deadline rather than as soon as it is written.
    ///
    /// The real-time consumer starts draining the moment the output opens, so
    /// a frame is heard once everything already queued ahead of it has played:
    /// its play time is the write time plus the current ring depth. Writing
    /// the first frame into an empty ring would therefore play it immediately,
    /// discarding the lead the write-ahead is meant to establish. Queuing
    /// exactly the remaining time until its deadline both places it correctly
    /// and seeds the ring's steady-state cushion.
    fn queue_alignment_prefill(&mut self, frame: &ScheduledFrame, local_now_ms: u64) {
        if !self.awaiting_prefill {
            return;
        }
        self.awaiting_prefill = false;
        let deadline_ms = self
            .scheduler
            .local_time_for_host_ms(frame.host_presentation_time_ms);
        // Already due or late: play it now rather than delaying it further.
        let lead_ms = deadline_ms
            .saturating_sub(local_now_ms)
            .min(self.config.max_prefill_ms);
        if lead_ms == 0 {
            return;
        }
        let frames = usize::try_from(lead_ms * u64::from(self.scheduler.sample_rate()) / 1_000)
            .unwrap_or(usize::MAX);
        self.prefill_frames = frames;
        self.pending
            .extend(core::iter::repeat_n(0.0_f32, frames * RENDER_CHANNELS));
    }

    /// Pushes as much of `pending` as the ring accepts, retaining the rest.
    pub(super) fn flush_pending(&mut self) -> usize {
        if self.pending.is_empty() {
            return 0;
        }
        let written = self.producer.push_frames(&self.pending);
        if written > 0 {
            self.pending.drain(..written * RENDER_CHANNELS);
        }
        self.peak_queued_frames = self.peak_queued_frames.max(self.queued_frames());
        written
    }
}
