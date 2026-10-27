//! A second independent production listener scheduler, render ring, and native
//! callback in the real TCP/UDP integration suite. Each socket must render
//! the same PCM without sharing a native output token or concealing a gap.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, register_render_ring, release_render_ring,
    silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
    silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
};
use crate::callback_capture::CallbackCaptureTap;
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{SessionId, StreamId};
use silent_disco_core::protocol::AudioDatagram;
use std::sync::Arc;

pub(super) struct SecondaryPcmOutput {
    pump: PlaybackPump,
    token: u64,
    tap: Arc<CallbackCaptureTap>,
}

// A production-path test can enqueue a full 40-packet current track before
// invoking callbacks. Both independent listener rings must hold that entire
// deterministic generation, not silently cap the second output at 100 ms.
const TEST_OUTPUT_FRAMES: usize = 16_384;

impl SecondaryPcmOutput {
    pub(super) fn new(
        session: &SessionId,
        stream: &StreamId,
        start_ms: u64,
        samples_per_packet: u32,
    ) -> Self {
        let mut config = SchedulerConfig::new(
            session.clone(),
            stream.clone(),
            48_000,
            start_ms,
            samples_per_packet,
            2,
        );
        config.startup_buffer_target_ms = 0;
        let (producer, token) = register_render_ring(RenderRingConfig {
            capacity_frames: TEST_OUTPUT_FRAMES,
            target_fill_frames: TEST_OUTPUT_FRAMES,
        })
        .expect("second listener native output");
        let mut pump = PlaybackPump::new(
            PlaybackScheduler::new(config, 0.0).expect("second listener scheduler"),
            producer,
            PlaybackPumpConfig {
                volume: 1.0,
                write_lead_ms: 0,
                max_prefill_ms: 0,
                target_depth_frames: TEST_OUTPUT_FRAMES,
            },
        )
        .expect("second listener pump");
        pump.apply_sync_offset(0.0);
        let tap = Arc::new(CallbackCaptureTap::new(TEST_OUTPUT_FRAMES).expect("second capture"));
        attach_callback_capture(token, Arc::clone(&tap)).expect("attach second capture");
        Self { pump, token, tap }
    }

    pub(super) fn token(&self) -> u64 {
        self.token
    }

    pub(super) fn submit(&mut self, packet: AudioDatagram, host_time_ms: u64) {
        self.pump
            .scheduler_mut()
            .submit_packet(packet)
            .expect("second listener schedule PCM");
        for _ in 0..64 {
            if !matches!(
                self.pump.tick(host_time_ms),
                PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
            ) {
                break;
            }
        }
    }

    pub(super) fn reanchor(&mut self, start_ms: u64) {
        self.pump.scheduler_mut().set_host_start_time_ms(start_ms);
    }

    pub(super) fn assert_and_release(self, chunks: &[u32], expected: &[f32], packets: u64) {
        let native = usize::try_from(self.token).expect("second token") as *mut core::ffi::c_void;
        let mut captured = Vec::new();
        for &frames in chunks {
            let mut samples = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
            let mut supplied = 0_u32;
            assert_eq!(
                silent_disco_audio_read_interleaved_f32(
                    native,
                    samples.as_mut_ptr(),
                    frames,
                    2,
                    &raw mut supplied,
                ),
                0,
                "second listener native callback must not underrun"
            );
            assert_eq!(supplied, frames);
            assert_eq!(
                self.tap.drain(usize::try_from(frames).expect("frames")),
                samples,
                "second callback tap must match its own native output"
            );
            captured.extend(samples);
        }
        assert_eq!(
            captured, expected,
            "independent TCP listener must render exactly the first listener's PCM"
        );
        assert_eq!(self.pump.diagnostics().packets_emitted, packets);
        assert_eq!(self.pump.diagnostics().concealed_packets, 0);
        assert_eq!(
            silent_disco_audio_callback_count(native.cast_const()),
            u64::try_from(chunks.len()).expect("callback count")
        );
        assert_eq!(silent_disco_audio_underrun_count(native.cast_const()), 0);
        assert_eq!(
            silent_disco_audio_silence_filled_frames(native.cast_const()),
            0
        );
        assert_eq!(
            self.tap.counters(),
            (
                u64::try_from(expected.len() / 2).expect("frame count"),
                0,
                chunks.len() as u64,
            )
        );
        detach_callback_capture(self.token);
        release_render_ring(self.token).expect("release second listener native output");
    }
}
