//! Real production socket -> FFI -> scheduler -> native callback PCM proof.
//! Software-only: does not claim Oboe/DAC or physical output continuity.
use super::{FfiListenerTransportHandle, Inner};
use crate::audio_abi::{
    registry_test_guard, silent_disco_audio_callback_count, silent_disco_audio_frames_rendered,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_underrun_count,
};
use crate::listener_playback::{FfiListenerPlaybackConfig, FfiListenerPlaybackHandle};
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
const START_MS: u64 = 400;

#[test]
fn socket_pcm_callback_is_sample_exact_across_track_boundary_and_engine_retirement() {
    let _guard = registry_test_guard();
    let session = SessionId::new("callback-session").expect("session");
    let stream = StreamId::new("callback-stream").expect("stream");
    let device = DeviceId::new("callback-listener").expect("device");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketTransportFactory
        .bind_host(HostTransportConfig::loopback(session.clone()), clock.clone())
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
    host.authorize_peer(&device, routes).expect("authorize listener");

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
        ring_target_fill_frames: 40_000,
        write_lead_ms: 800,
        max_prefill_ms: 800,
        volume: 1.0,
    })
    .expect("open native callback runtime");
    runtime.lock_same_process_host_clock(0).expect("lock local clock");
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
            host.broadcast_audio(&packet).expect("send UDP PCM").report.successful_peers,
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
        assert!(Instant::now() < deadline, "PCM never reached ring: {state:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(diagnostics.sequences_skipped, 0);
    assert_eq!(diagnostics.concealed_packets, 0);
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
                engine, output.as_mut_ptr(), frames, 2, &raw mut supplied,
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
                let sequence = (index - diagnostics.prefill_frames) / u64::from(PACKET_FRAMES);
                (i16::try_from(sequence).expect("bounded") + 1) * 4_096
            };
            let left = f32::from(amplitude) / 32_768.0;
            assert_eq!(stereo[0].to_bits(), left.to_bits(), "left sample {index}");
            assert_eq!(stereo[1].to_bits(), (-left).to_bits(), "right sample {index}");
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
            engine, output.as_mut_ptr(), 1, 2, &raw mut supplied,
        ),
        2,
        "retired engine must not reopen"
    );
    assert_eq!(supplied, 0);
    assert_eq!(output, [0.0, 0.0]);
    handle.shutdown().expect("close listener");
    host.shutdown().expect("close host");
}
