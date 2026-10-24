//! Repeated Stop/replay must not accumulate retired output registrations.
#![allow(clippy::float_cmp)]

use super::{
    AudioAbiError, audio_engine_registry, registry_test_guard, release_render_ring,
    silent_disco_audio_read_interleaved_f32,
};
use crate::FfiAudioOutputHandle;

#[test]
fn repeated_stop_replay_keeps_registry_bounded_and_old_callbacks_silent() {
    let _guard = registry_test_guard();
    let baseline = audio_engine_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entries
        .len();
    let mut retired = Vec::new();

    for generation in 0_u16..64 {
        let output = FfiAudioOutputHandle::open(4_800, 1).expect("new output");
        let token = u64::try_from(output.engine_token().expect("token")).expect("valid token");
        let amplitude = (f32::from(generation) + 1.0) / 128.0;
        let pcm = [amplitude, -amplitude];
        assert_eq!(output.push_frames(pcm.to_vec()).expect("queued"), 1);

        // Delayed callbacks from retired outputs must never consume replay PCM.
        for &old_token in &retired {
            let old_engine =
                usize::try_from(old_token).expect("old token") as *mut core::ffi::c_void;
            let mut stale_pcm = [f32::NAN; 2];
            let mut supplied = u32::MAX;
            assert_eq!(
                silent_disco_audio_read_interleaved_f32(
                    old_engine,
                    stale_pcm.as_mut_ptr(),
                    1,
                    2,
                    &raw mut supplied,
                ),
                2,
                "retired generation must report Stopping"
            );
            assert_eq!(supplied, 0);
            assert_eq!(stale_pcm, [0.0, 0.0]);
        }
        assert_eq!(output.queued_frames().expect("unconsumed live PCM"), 1);

        let engine = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
        let mut actual = [f32::NAN; 2];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                actual.as_mut_ptr(),
                1,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, 1);
        assert_eq!(actual, pcm);

        output.release();
        assert_eq!(release_render_ring(token), Err(AudioAbiError::AlreadyReleased));
        retired.push(token);
        assert_eq!(
            audio_engine_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entries
                .len(),
            baseline,
            "released generations must not leave registry tombstones"
        );
    }
}
