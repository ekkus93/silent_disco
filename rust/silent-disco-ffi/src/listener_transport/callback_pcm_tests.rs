//! Real production socket -> FFI -> scheduler -> native callback PCM proof.
//! Software-only: does not claim Oboe/DAC or physical output continuity.
use super::{FfiListenerTransportHandle, Inner};
use crate::audio_abi::{
    registry_test_guard, silent_disco_audio_callback_count, silent_disco_audio_frames_rendered,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_underrun_count,
};
use crate::listener_playback::{FfiListenerPlaybackConfig, FfiListenerPlaybackHandle};
use silent_disco_core::audio::DEFAULT_CONCEALMENT_RAMP_MS;
use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, SocketTransportFactory,
    TransportChannel, TransportClock, TransportEvent, TransportFactory,
};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const PACKET_FRAMES: u32 = 240;
const START_MS: u64 = 2_000;

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "single end-to-end TCP/UDP through native callback scenario keeps sample and lifecycle assertions together"
)]
fn socket_pcm_callback_is_sample_exact_across_track_boundary_and_engine_retirement() {
    let _guard = registry_test_guard();
    let session = SessionId::new("callback-session").expect("session");
    let stream = StreamId::new("callback-stream").expect("stream");
    let device = DeviceId::new("callback-listener").expect("device");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketTransportFactory
        .bind_host(
            HostTransportConfig::loopback(session.clone()),
            clock.clone(),
        )
        .expect("production TCP/UDP host");
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("production TCP/UDP listener");
    let routes = listener.local_routes();
    let transport_clock: Arc<dyn TransportClock> = clock;
    let handle = FfiListenerTransportHandle {
        inner: RwLock::new(Some(Inner {
            transport: listener,
            session_id: session.clone(),
            device_id: device.clone(),
            clock: transport_clock,
        })),
        playback: Mutex::new(None),
        forwarded_audio: AtomicU64::new(0),
        closed: std::sync::atomic::AtomicBool::new(false),
    };
    handle
        .send_join_request("Callback listener".to_owned(), None)
        .expect("send join");
    let join_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = join_deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for join");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            peer,
            ..
        } = host.recv_event(remaining).expect("host join event")
            && peer.device_id.as_ref() == Some(&device)
        {
            break;
        }
    }
    host.authorize_peer(&device, routes)
        .expect("authorize listener");

    // A future host start makes delivery independent of runner scheduling.
    // Account for real scheduler alignment silence rather than ignoring it.
    let runtime = FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
        session_id: session.as_str().to_owned(),
        stream_id: stream.as_str().to_owned(),
        sample_rate: 48_000,
        host_start_time_ms: START_MS,
        samples_per_packet: PACKET_FRAMES,
        channels: 2,
        startup_buffer_target_ms: 0,
        rebuffer_target_ms: 0,
        ring_capacity_frames: 48_000,
        ring_target_fill_frames: 39_360,
        write_lead_ms: 800,
        max_prefill_ms: 800,
        volume: 1.0,
    })
    .expect("open native callback runtime");
    runtime
        .lock_same_process_host_clock(0)
        .expect("lock local clock");
    let token = runtime.engine_token();
    assert!(token > 0);
    handle.attach_playback(Arc::clone(&runtime));

    host.broadcast_control(&ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(START_MS),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: PACKET_FRAMES,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("First".to_owned()),
    }))
    .expect("start stream");
    assert!(matches!(
        handle.poll_event(1_000).expect("start poll"),
        Some(super::FfiListenerTransportEvent::StreamStarted { .. })
    ));
    host.broadcast_control(&ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second".to_owned(),
        display_name: "Second".to_owned(),
    }))
    .expect("boundary");
    assert!(matches!(
        handle.poll_event(1_000).expect("boundary poll"),
        Some(super::FfiListenerTransportEvent::TrackBoundary {
            first_sample_index: 480,
            ..
        })
    ));

    for sequence in 0_u64..4 {
        let amplitude = (i16::try_from(sequence).expect("bounded") + 1) * 4_096;
        let payload = [amplitude, -amplitude]
            .into_iter()
            .cycle()
            .take(PACKET_FRAMES as usize * 2)
            .flat_map(i16::to_le_bytes)
            .collect();
        let packet = ProtocolFrame::Audio(AudioDatagram {
            session_id: session.clone(),
            stream_id: stream.clone(),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: PACKET_FRAMES,
            first_sample_index: SampleIndex::new(sequence * u64::from(PACKET_FRAMES)),
            host_presentation_time_ms: MonotonicMillis::new(START_MS + sequence * 5),
            payload,
        });
        assert_eq!(
            host.broadcast_audio(&packet)
                .expect("send UDP PCM")
                .report
                .successful_peers,
            1
        );
    }
    assert!(handle.poll_event(500).expect("forward audio").is_none());
    assert_eq!(handle.forwarded_audio_count(), 4);

    let deadline = Instant::now() + Duration::from_secs(5);
    let diagnostics = loop {
        let state = runtime.diagnostics();
        let expected = state.prefill_frames + u64::from(PACKET_FRAMES) * 4;
        if state.packets_emitted == 4 && state.ring_queued_frames >= expected {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "PCM never reached ring: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    // Only the four transmitted packets are compared; the pump may subsequently
    // conceal absent future packets if no native callback is draining the ring.
    let expected_frames = diagnostics.prefill_frames + u64::from(PACKET_FRAMES) * 4;
    let engine = usize::try_from(token).expect("token fits pointer") as *mut core::ffi::c_void;
    let original_token = runtime.engine_token();
    let mut consumed = 0_u64;
    let sizes = [137_u32, 311, 19, 493];
    let mut calls = 0_u64;
    while consumed < expected_frames {
        let size = sizes[usize::try_from(calls).expect("bounded") % sizes.len()];
        let frames = u32::try_from((expected_frames - consumed).min(u64::from(size)))
            .expect("bounded callback size");
        let mut output = vec![f32::NAN; frames as usize * 2];
        let mut supplied = 0_u32;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                output.as_mut_ptr(),
                frames,
                2,
                &raw mut supplied,
            ),
            0,
            "callback must not conceal missing PCM"
        );
        assert_eq!(supplied, frames);
        for (offset, stereo) in output.chunks_exact(2).enumerate() {
            let index = consumed + u64::try_from(offset).expect("bounded");
            let amplitude = if index < diagnostics.prefill_frames {
                0_i16
            } else {
                let pcm_index = index - diagnostics.prefill_frames;
                let sequence = pcm_index / u64::from(PACKET_FRAMES);
                let raw = (i16::try_from(sequence).expect("bounded") + 1) * 4_096;
                // The production scheduler fades the first real packet in
                // from silence to avoid a click. Compare the actual shaped
                // PCM16 samples, including integer truncation, rather than
                // incorrectly expecting an unmodified first sample.
                let ramp_frames = u64::from(DEFAULT_CONCEALMENT_RAMP_MS) * 48;
                if sequence == 0 && pcm_index < ramp_frames {
                    let shaped = i64::from(raw) * i64::try_from(pcm_index).expect("bounded")
                        / i64::try_from(ramp_frames).expect("bounded");
                    i16::try_from(shaped).expect("ramped sample fits")
                } else {
                    raw
                }
            };
            let left = f32::from(amplitude) / 32_768.0;
            let right = f32::from(-amplitude) / 32_768.0;
            assert_eq!(stereo[0].to_bits(), left.to_bits(), "left sample {index}");
            assert_eq!(stereo[1].to_bits(), right.to_bits(), "right sample {index}");
        }
        consumed += u64::from(frames);
        calls += 1;
    }
    assert_eq!(silent_disco_audio_frames_rendered(engine), expected_frames);
    assert_eq!(silent_disco_audio_underrun_count(engine), 0);
    assert_eq!(silent_disco_audio_callback_count(engine), calls);
    assert_eq!(runtime.engine_token(), original_token);

    handle.detach_playback();
    runtime.stop().expect("stop runtime");
    let mut output = [1.0_f32; 2];
    let mut supplied = u32::MAX;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            engine,
            output.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        ),
        2,
        "retired engine must not reopen"
    );
    assert_eq!(supplied, 0);
    assert_eq!(output.map(f32::to_bits), [0_u32; 2]);

    // Stop/replay on the SAME authorized TCP/UDP listener must use a fresh
    // native engine without resurrecting the retired ring or mixing old PCM.
    // This exercises production socket ingress and the real C callback again,
    // rather than merely checking that a new scheduler can be constructed.
    host.broadcast_control(&ControlMessage::Stop(silent_disco_core::protocol::Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(START_MS + 20),
    }))
    .expect("broadcast stop");
    assert!(matches!(
        handle.poll_event(1_000).expect("stop poll"),
        Some(super::FfiListenerTransportEvent::Stopped { .. })
    ));
    let replay_stream = StreamId::new("callback-replay").expect("replay stream");
    let replay = FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
        session_id: session.as_str().to_owned(),
        stream_id: replay_stream.as_str().to_owned(),
        sample_rate: 48_000,
        host_start_time_ms: START_MS,
        samples_per_packet: PACKET_FRAMES,
        channels: 2,
        startup_buffer_target_ms: 0,
        rebuffer_target_ms: 0,
        ring_capacity_frames: 48_000,
        ring_target_fill_frames: 39_360,
        write_lead_ms: 800,
        max_prefill_ms: 800,
        volume: 1.0,
    })
    .expect("open replay callback runtime");
    replay
        .lock_same_process_host_clock(0)
        .expect("lock replay clock");
    let replay_token = replay.engine_token();
    assert_ne!(
        replay_token, original_token,
        "replay must have a new engine"
    );
    handle.attach_playback(Arc::clone(&replay));
    let replay_start_delivery = host.broadcast_control(&ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: replay_stream.clone(),
        host_start_time_ms: MonotonicMillis::new(START_MS),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: PACKET_FRAMES,
        current_entry_id: Some("replay".to_owned()),
        current_display_name: Some("Replay".to_owned()),
    }))
    .expect("broadcast replay start");
    assert_eq!(replay_start_delivery.report.successful_peers, 1);
    let replay_start_event = handle.poll_event(1_000).expect("replay start poll");
    assert!(matches!(
        replay_start_event,
        Some(super::FfiListenerTransportEvent::StreamStarted { stream_id, .. })
            if stream_id == replay_stream.as_str()
    ), "unexpected replay control event: {replay_start_event:?}");
    let replay_pcm = [12_288_i16, -12_288_i16]
        .into_iter()
        .cycle()
        .take(PACKET_FRAMES as usize * 2)
        .flat_map(i16::to_le_bytes)
        .collect();
    host.broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
        session_id: session.clone(),
        stream_id: replay_stream,
        sequence: PacketSequence::new(0),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: PACKET_FRAMES,
        first_sample_index: SampleIndex::new(0),
        host_presentation_time_ms: MonotonicMillis::new(START_MS),
        payload: replay_pcm,
    }))
    .expect("broadcast replay PCM");
    assert!(
        handle
            .poll_event(500)
            .expect("forward replay PCM")
            .is_none()
    );
    assert_eq!(handle.forwarded_audio_count(), 5);
    let deadline = Instant::now() + Duration::from_secs(5);
    let replay_prefill = loop {
        let state = replay.diagnostics();
        if state.packets_emitted == 1
            && state.ring_queued_frames >= state.prefill_frames + u64::from(PACKET_FRAMES)
        {
            break state.prefill_frames;
        }
        assert!(
            Instant::now() < deadline,
            "replay PCM did not reach native ring: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let replay_engine =
        usize::try_from(replay_token).expect("replay token fits pointer") as *mut core::ffi::c_void;
    let mut prefill = vec![f32::NAN; usize::try_from(replay_prefill).expect("bounded prefill") * 2];
    let mut prefill_supplied = 0;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            replay_engine,
            prefill.as_mut_ptr(),
            u32::try_from(replay_prefill).expect("bounded prefill"),
            2,
            &raw mut prefill_supplied,
        ),
        0
    );
    assert_eq!(u64::from(prefill_supplied), replay_prefill);
    assert!(prefill.iter().all(|sample| *sample == 0.0));
    let mut replay_output = vec![f32::NAN; PACKET_FRAMES as usize * 2];
    let mut replay_supplied = 0;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            replay_engine,
            replay_output.as_mut_ptr(),
            PACKET_FRAMES,
            2,
            &raw mut replay_supplied,
        ),
        0
    );
    assert_eq!(replay_supplied, PACKET_FRAMES);
    for (frame, stereo) in replay_output.chunks_exact(2).enumerate() {
        let shaped = if frame < DEFAULT_CONCEALMENT_RAMP_MS as usize * 48 {
            12_288_i64 * i64::try_from(frame).expect("bounded frame")
                / (i64::from(DEFAULT_CONCEALMENT_RAMP_MS) * 48)
        } else {
            12_288
        };
        let shaped = i16::try_from(shaped).expect("replay PCM16 amplitude");
        let expected = f32::from(shaped) / 32_768.0;
        assert_eq!(
            stereo[0].to_bits(),
            expected.to_bits(),
            "replay left {frame}"
        );
        assert_eq!(
            stereo[1].to_bits(),
            (f32::from(-shaped) / 32_768.0).to_bits(),
            "replay right {frame}"
        );
    }
    // The first token remains permanently retired even while the replacement
    // is actively returning PCM to a native callback.
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            engine,
            output.as_mut_ptr(),
            1,
            2,
            &raw mut supplied,
        ),
        2
    );
    assert_eq!(supplied, 0);
    assert_eq!(replay.engine_token(), replay_token);
    handle.detach_playback();
    replay.stop().expect("stop replay");
    handle.shutdown().expect("close listener");
    host.shutdown().expect("close host");
}
