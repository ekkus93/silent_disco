//! Preallocated SPSC PCM tap for the real-time Rust native callback.
//!
//! The callback does no I/O, allocation, waiting or locking for this buffer.
//! Only one native callback can enter under the registry's try-lock at a time,
//! and only one caller drains a given tap; indices publish complete frames.
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

const CHANNELS: usize = 2;
pub(crate) const MIN_CAPTURE_FRAMES: usize = 4_800;
pub(crate) const MAX_CAPTURE_FRAMES: usize = 480_000;

#[derive(Debug)]
pub(crate) struct CallbackCaptureTap {
    slots: Box<[AtomicU32]>,
    capacity: usize,
    written: AtomicUsize,
    read: AtomicUsize,
    lost: AtomicU64,
    callbacks: AtomicU64,
}

impl CallbackCaptureTap {
    pub(crate) fn new(capacity: usize) -> Option<Self> {
        if !(MIN_CAPTURE_FRAMES..=MAX_CAPTURE_FRAMES).contains(&capacity) {
            return None;
        }
        Some(Self {
            slots: (0..capacity * CHANNELS)
                .map(|_| AtomicU32::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            capacity,
            written: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            lost: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
        })
    }

    /// Called only from the serialized native callback after the render read.
    pub(crate) fn record_callback(&self, samples: &[f32]) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        let frames = samples.len() / CHANNELS;
        let write = self.written.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let available = self.capacity.saturating_sub(write.saturating_sub(read));
        let accepted = frames.min(available);
        for frame in 0..accepted {
            let slot = (write + frame) % self.capacity * CHANNELS;
            for channel in 0..CHANNELS {
                self.slots[slot + channel].store(
                    samples[frame * CHANNELS + channel].to_bits(),
                    Ordering::Relaxed,
                );
            }
        }
        self.written.store(write + accepted, Ordering::Release);
        self.lost.fetch_add(
            u64::try_from(frames - accepted).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// Non-real-time control thread drains at most `max_frames` frames.
    pub(crate) fn drain(&self, max_frames: usize) -> Vec<f32> {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.written.load(Ordering::Acquire);
        let frames = (write - read).min(max_frames);
        let mut result = Vec::with_capacity(frames * CHANNELS);
        for frame in 0..frames {
            let slot = (read + frame) % self.capacity * CHANNELS;
            for channel in 0..CHANNELS {
                result.push(f32::from_bits(
                    self.slots[slot + channel].load(Ordering::Relaxed),
                ));
            }
        }
        self.read.store(read + frames, Ordering::Release);
        result
    }

    pub(crate) fn counters(&self) -> (u64, u64, u64) {
        (
            u64::try_from(self.written.load(Ordering::Acquire)).unwrap_or(u64::MAX),
            self.lost.load(Ordering::Relaxed),
            self.callbacks.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_captures_wraparound_and_reports_explicit_loss() {
        let tap = CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("valid capacity");
        let initial = vec![0.25; MIN_CAPTURE_FRAMES * CHANNELS];
        tap.record_callback(&initial);
        assert_eq!(tap.counters(), (4_800, 0, 1));
        assert_eq!(tap.drain(4_799), initial[..4_799 * CHANNELS]);
        let next = vec![0.75; 100 * CHANNELS];
        tap.record_callback(&next);
        assert_eq!(tap.counters(), (4_900, 0, 2));
        assert_eq!(
            tap.drain(101),
            [&initial[4_799 * CHANNELS..], &next].concat()
        );

        tap.record_callback(&vec![0.5; (MIN_CAPTURE_FRAMES + 3) * CHANNELS]);
        assert_eq!(tap.counters(), (9_700, 3, 3));
        assert_eq!(tap.drain(4_800).len(), 4_800 * CHANNELS);
        assert!(tap.drain(1).is_empty());
    }

    #[test]
    fn concurrent_audio_callback_and_background_drain_preserve_exact_frame_order() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        const FRAMES: usize = 24_000;
        const BATCH: usize = 512;
        let tap = Arc::new(CallbackCaptureTap::new(MAX_CAPTURE_FRAMES).expect("capacity"));
        let original: Vec<f32> = (0..FRAMES)
            .flat_map(|frame| {
                std::iter::repeat_n(
                    f32::from(u16::try_from(frame).expect("bounded frame")),
                    CHANNELS,
                )
            })
            .collect();
        let writer = Arc::clone(&tap);
        let packets = original.clone();
        let producer = std::thread::spawn(move || {
            for chunk in packets.chunks(BATCH * CHANNELS) {
                // The actual C-ABI audio callback has one serialized producer.
                // Every published frame is atomically visible before the
                // background consumer observes the incremented write cursor.
                writer.record_callback(chunk);
                std::thread::yield_now();
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::with_capacity(original.len());
        while received.len() < original.len() {
            received.extend(tap.drain(73));
            assert!(Instant::now() < deadline, "bounded background drain stalled");
            if received.len() < original.len() {
                std::thread::yield_now();
            }
        }
        producer.join().expect("audio writer must finish");
        assert_eq!(received, original, "no missing or duplicated callback PCM");
        assert_eq!(tap.counters(), (24_000, 0, 47));
        assert!(tap.drain(1).is_empty());
    }

    #[test]
    fn rejects_unbounded_capture_configuration() {
        assert!(CallbackCaptureTap::new(0).is_none());
        assert!(CallbackCaptureTap::new(MAX_CAPTURE_FRAMES + 1).is_none());
    }
}
