//! Production `UniFFI` reservation may commit a successor during a host pause.
use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_silence_filled_frames,
    silent_disco_audio_underrun_count,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::CallbackCaptureTap;
use crate::host_control::{
    FfiBridgeError, FfiCoreHandle, FfiCoreNotification, FfiCoreObserver, FfiPlatformCompletion,
    FfiPlatformEffect,
};
use silent_disco_core::domain::{AppRole, ApprovalMode, HostLifecycle, PlaybackState, StreamId};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorHandle, CoreCommand,
    CoreCommandRequest, CoreSnapshot, HostDraftPatch, InviteCodePatch, QueueEntry,
};
use std::fs;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Observer(mpsc::Sender<FfiCoreNotification>);
impl FfiCoreObserver for Observer {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.0.send(notification).expect("actor observer receiver");
        Ok(())
    }
}

fn wait_until(actor: &CoreActorHandle, ready: impl Fn(&CoreSnapshot) -> bool) -> CoreSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = actor.current_snapshot().expect("actor snapshot");
        if ready(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "actor state timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn submit(actor: &CoreActorHandle, command: CoreCommand) {
    let revision = actor.current_snapshot().expect("actor revision").revision;
    actor
        .submit_command(CoreCommandRequest::new(revision, command).expect("command"))
        .expect("submit command");
}

fn wav(frames: u32, sample: i16) -> Vec<u8> {
    let bytes = frames * 4;
    let mut data = Vec::with_capacity(44 + bytes as usize);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16_u32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.extend_from_slice(&48_000_u32.to_le_bytes());
    data.extend_from_slice(&192_000_u32.to_le_bytes());
    data.extend_from_slice(&4_u16.to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for _ in 0..frames * 2 {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one end-to-end actor and producer pause/resume test with packet-level assertions"
)]
fn authoritative_ffi_playlist_reserves_successor_while_paused() {
    let _audio_guard = registry_test_guard();
    let (tx, rx) = mpsc::channel();
    let ffi =
        FfiCoreHandle::open("ffi-paused-actor".into(), Arc::new(Observer(tx))).expect("open actor");
    let actor = ffi.actor_handle().expect("live actor");
    let descriptor = AudioSourceDescriptor::new("source-first", "First.wav", Some(1_920), None)
        .expect("first source");
    let next_source = AudioSourceDescriptor::new("source-next", "Next.wav", Some(1_920), None)
        .expect("next source");
    submit(
        &actor,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    wait_until(&actor, |s| s.selected_role == Some(AppRole::Host));
    submit(
        &actor,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Pause proof".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: Some(false),
        }),
    );
    wait_until(&actor, |s| s.host_draft.audio_source.is_some());
    submit(&actor, CoreCommand::CreateHostSession);
    let operation_id = loop {
        if let FfiCoreNotification::PlatformEffect {
            effect: FfiPlatformEffect::StartAdvertising { operation_id, .. },
        } = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("advertise effect")
        {
            break operation_id;
        }
    };
    ffi.platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("advertisement acknowledged");
    let before = wait_until(&actor, |s| {
        matches!(
            s.host_lifecycle,
            HostLifecycle::Ready | HostLifecycle::WaitingForListeners
        )
    });
    let first_id = before
        .host_queue
        .current()
        .expect("first entry")
        .entry_id
        .clone();
    submit(
        &actor,
        CoreCommand::AppendHostQueue {
            expected_queue_revision: before.host_queue.revision(),
            entries: vec![QueueEntry::new("next-entry", next_source).expect("next entry")],
        },
    );
    wait_until(&actor, |s| s.host_queue.entries().len() == 2);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Buffering))
        .expect("buffer");
    wait_until(&actor, |s| s.playback_state == PlaybackState::Buffering);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("start playback before binding its stream identity");
    wait_until(&actor, |s| s.playback_state == PlaybackState::Playing);
    let stream_id = StreamId::new("ffi-pause-stream").expect("stream ID");
    actor
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: stream_id.clone(),
            position_ms: 0,
        })
        .expect("start and bind stream");
    wait_until(&actor, |s| s.playback_state == PlaybackState::Playing);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
        .expect("pause");
    wait_until(&actor, |s| s.playback_state == PlaybackState::Paused);

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ffi-pause-reservation-{unique}"));
    fs::create_dir_all(&dir).expect("fixtures directory");
    let first_path = dir.join("first.wav");
    let next_path = dir.join("next.wav");
    fs::write(&first_path, wav(480, 3_000)).expect("first fixture");
    fs::write(&next_path, wav(480, 6_000)).expect("next fixture");
    let source = |entry_id: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: entry_id.to_owned(),
        display_name: entry_id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };
    let stream = FfiHostPlaylistStreamer::open_authoritative(
        "ffi-paused-session".into(),
        stream_id.to_string(),
        5_000,
        vec![
            source(&first_id, &first_path),
            source("next-entry", &next_path),
        ],
        Arc::clone(&ffi),
    )
    .expect("open authoritative producer");

    // The producer can prebuffer while paused, but the output callback must
    // not consume those frames until Resume. Retain one native output token
    // across the boundary and compare callback-returned PCM after resuming.
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("native render ring");
    let token = u64::try_from(output.engine_token().expect("engine token"))
        .expect("nonnegative native token");
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("capture capacity"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach capture");
    let mut samples = Vec::new();
    let mut boundaries = Vec::new();
    loop {
        match stream.next_frame(2_000).expect("paused producer frame") {
            Some(FfiPlaylistFrame::Audio {
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            }) => {
                assert_eq!(
                    payload.len(),
                    usize::try_from(samples_per_packet).expect("packet frames") * 4
                );
                let pcm = payload
                    .chunks_exact(2)
                    .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                    .collect();
                assert_eq!(
                    output.push_frames(pcm).expect("queue native PCM"),
                    samples_per_packet
                );
                samples.push(first_sample_index);
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                let snapshot = wait_until(&actor, |s| s.host_queue_next_boundary.is_some());
                assert_eq!(snapshot.playback_state, PlaybackState::Paused);
                assert_eq!(
                    snapshot.host_queue_next_boundary.as_ref().unwrap().entry_id,
                    entry_id
                );
                boundaries.push((entry_id, first_sample_index));
            }
            Some(FfiPlaylistFrame::End) => break,
            None => panic!("paused producer never reserved next track"),
        }
    }
    assert_eq!(boundaries, [("next-entry".to_owned(), 480)]);
    assert_eq!(samples, [0, 240, 480, 720]);
    actor
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("resume");
    wait_until(&actor, |s| s.playback_state == PlaybackState::Playing);
    actor
        .submit_audio_event(AudioEvent::TrackBoundaryApplied {
            entry_id: "next-entry".into(),
            first_sample_index: 480,
        })
        .expect("apply after resume");
    let after = wait_until(&actor, |s| {
        s.host_queue
            .current()
            .is_some_and(|entry| entry.entry_id == "next-entry")
    });
    assert!(after.host_queue_next_boundary.is_none());
    assert_eq!(output.queued_frames().expect("queued native PCM"), 960);
    let mut absolute_frame = 0_u32;
    // Neither callback boundary is aligned with the 480-frame track change.
    for frames in [479_u32, 2, 479] {
        let mut pcm = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
        let mut supplied = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            usize::try_from(token).expect("native token") as *mut core::ffi::c_void,
            pcm.as_mut_ptr(),
            frames,
            2,
            &raw mut supplied,
        );
        assert_eq!(status, 0, "resume must not underrun at the boundary");
        assert_eq!(supplied, frames);
        assert_eq!(
            tap.drain(usize::try_from(frames).expect("frames")),
            pcm,
            "callback capture must match native output"
        );
        for stereo in pcm.chunks_exact(2) {
            let expected = if absolute_frame < 480 {
                3_000_i16
            } else {
                6_000_i16
            };
            let expected = f32::from(expected) / 32768.0;
            assert!(
                stereo
                    .iter()
                    .all(|sample| sample.to_bits() == expected.to_bits()),
                "wrong resumed callback PCM at sample {absolute_frame}"
            );
            absolute_frame += 1;
        }
    }
    assert_eq!(absolute_frame, 960);
    let engine = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    assert_eq!(silent_disco_audio_underrun_count(engine), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    assert_eq!(tap.counters(), (960, 0, 3));
    detach_callback_capture(token);
    output.release();
    ffi.shutdown().expect("shut down host");
    fs::remove_dir_all(dir).expect("clean fixtures");
}
