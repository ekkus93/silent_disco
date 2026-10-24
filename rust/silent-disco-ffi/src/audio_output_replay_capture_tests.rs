//! Cross-generation native callback capture isolation for Stop/replay.

#![allow(clippy::float_cmp)]

use std::sync::Arc;

use super::FfiAudioOutputHandle;
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_read_interleaved_f32,
};
use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};

#[test]
fn retired_capture_cleanup_cannot_detach_or_pollute_replay_capture() {
    let _guard = registry_test_guard();
    let old = FfiAudioOutputHandle::open(4_800, 1).expect("old output");
    let old_token = u64::try_from(old.engine_token().expect("old token")).expect("old token");
    let old_tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("old tap"));
    attach_callback_capture(old_token, Arc::clone(&old_tap)).expect("old capture");
    let old_pcm = [0.125_f32, -0.125];
    assert_eq!(old.push_frames(old_pcm.to_vec()).expect("old PCM"), 1);
    let old_engine = usize::try_from(old_token).expect("old token") as *mut core::ffi::c_void;
    let mut old_output = [f32::NAN; 2];
    let mut supplied = u32::MAX;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            old_engine,
            old_output.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        ),
        0
    );
    assert_eq!(old_output, old_pcm);
    assert_eq!(old_tap.drain(1), old_pcm);
    old.release();

    let replay = FfiAudioOutputHandle::open(4_800, 1).expect("replay output");
    let replay_token =
        u64::try_from(replay.engine_token().expect("replay token")).expect("replay token");
    assert_ne!(old_token, replay_token);
    let replay_tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("replay tap"));
    assert!(attach_callback_capture(old_token, Arc::clone(&replay_tap)).is_err());
    attach_callback_capture(replay_token, Arc::clone(&replay_tap)).expect("replay capture");
    detach_callback_capture(old_token);

    let replay_pcm = [0.875_f32, -0.875];
    assert_eq!(
        replay.push_frames(replay_pcm.to_vec()).expect("replay PCM"),
        1
    );
    // A delayed callback from the retired output must not consume replay PCM,
    // record it in either diagnostic tap, or advance the new ring's cursor.
    old_output.fill(f32::NAN);
    supplied = u32::MAX;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            old_engine,
            old_output.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        ),
        2 // Stopping: the old token remains retired across replay.
    );
    assert_eq!(supplied, 0);
    assert_eq!(old_output, [0.0, 0.0]);
    assert_eq!(replay.queued_frames().expect("replay PCM untouched"), 1);
    assert_eq!(old_tap.counters(), (1, 0, 1));
    assert_eq!(replay_tap.counters(), (0, 0, 0));
    assert!(replay_tap.drain(1).is_empty());

    let replay_engine =
        usize::try_from(replay_token).expect("replay token") as *mut core::ffi::c_void;
    let mut replay_output = [f32::NAN; 2];
    supplied = u32::MAX;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            replay_engine,
            replay_output.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        ),
        0
    );
    assert_eq!(supplied, 1);
    assert_eq!(replay_output, replay_pcm);
    assert_eq!(replay_tap.drain(1), replay_pcm);
    assert_eq!(replay_tap.counters(), (1, 0, 1));
    assert_eq!(old_tap.counters(), (1, 0, 1));
    assert_eq!(replay.queued_frames().expect("drained ring"), 0);
    detach_callback_capture(replay_token);
    replay.release();
}
