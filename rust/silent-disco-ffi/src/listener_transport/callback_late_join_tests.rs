//! Two independent production TCP/UDP listeners: cached late-join timeline
//! and exact native callback PCM, without physical audio output claims.
use super::{FfiListenerTransportHandle, Inner};
use crate::audio_abi::{
    registry_test_guard, silent_disco_audio_frames_rendered,
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
    reason = "two independent production sockets and native callback engines are verified together"
)]
fn late_join_replays_multitrack_controls_and_delivers_exact_pcm_to_two_native_callbacks() {
    let _guard = registry_test_guard();
    let session = SessionId::new("callback-late-join-session").expect("session");
    let stream = StreamId::new("callback-late-join-stream").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketTransportFactory
        .bind_host(
            HostTransportConfig::loopback(session.clone()),
            clock.clone(),
        )
        .expect("production TCP/UDP host");
    let start = StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(START_MS),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: PACKET_FRAMES,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("First".to_owned()),
    };
    let boundary = TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(u64::from(PACKET_FRAMES)),
        entry_id: "second".to_owned(),
        display_name: "Second".to_owned(),
    };
    let mut listeners = Vec::new();
    for (index, name) in ["callback-early", "callback-late"].iter().enumerate() {
        let device = DeviceId::new(*name).expect("device");
        let listener = SocketTransportFactory
            .connect_listener(
                ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
                clock.clone(),
            )
            .expect("production listener TCP/UDP");
        let routes = listener.local_routes();
        let transport_clock: Arc<dyn TransportClock> = clock.clone();
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
            .send_join_request((*name).to_owned(), None)
            .expect("send real TCP join");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for listener join");
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
            .expect("authorize real TCP generation and UDP routes");
        if index == 0 {
            assert_eq!(
                host.broadcast_control(&ControlMessage::StreamStart(start.clone()))
                    .expect("live Start")
                    .report
                    .successful_peers,
                1
            );
            assert_eq!(
                host.broadcast_control(&ControlMessage::TrackBoundary(boundary.clone()))
                    .expect("live boundary")
                    .report
                    .successful_peers,
                1
            );
        }
        // The second listener receives these controls from the host's
        // authorization-time cache, not from either live broadcast above.
        assert!(matches!(
            handle.poll_event(1_000).expect("Start event"),
            Some(super::FfiListenerTransportEvent::StreamStarted {
                stream_id,
                host_start_time_ms,
                ..
            }) if stream_id == stream.as_str() && host_start_time_ms == START_MS
        ));
        assert!(matches!(
            handle.poll_event(1_000).expect("boundary event"),
            Some(super::FfiListenerTransportEvent::TrackBoundary {
                first_sample_index,
                ..
            }) if first_sample_index == u64::from(PACKET_FRAMES)
        ));
        listeners.push(handle);
    }

    let mut runtimes = Vec::new();
    for handle in &listeners {
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
        .expect("native callback runtime");
        runtime
            .lock_same_process_host_clock(0)
            .expect("same-process host clock");
        handle.attach_playback(Arc::clone(&runtime));
        runtimes.push(runtime);
    }
    let tokens: Vec<i64> = runtimes
        .iter()
        .map(|runtime| runtime.engine_token())
        .collect();
    assert_ne!(tokens[0], tokens[1], "each listener owns its native output");

    for sequence in 0..2_u64 {
        if sequence == 1 {
            // Pause after the first packet has reached BOTH independent
            // native engines. Resume the same stream on both existing TCP
            // connections; a boundary must not allocate a replacement sink.
            let pause_ms = START_MS + 5;
            assert_eq!(
                host.broadcast_control(&ControlMessage::Pause(
                    silent_disco_core::protocol::Pause {
                        session_id: session.clone(),
                        stream_id: stream.clone(),
                        host_pause_time_ms: MonotonicMillis::new(pause_ms),
                    }
                ))
                .expect("broadcast two-listener Pause")
                .report
                .successful_peers,
                2
            );
            for (index, handle) in listeners.iter().enumerate() {
                assert!(
                    matches!(
                        handle.poll_event(1_000).expect("listener Pause"),
                        Some(super::FfiListenerTransportEvent::Paused {
                            stream_id,
                            host_pause_time_ms,
                        }) if stream_id == stream.as_str() && host_pause_time_ms == pause_ms
                    ),
                    "listener {index} must receive authoritative Pause"
                );
                assert_eq!(handle.forwarded_audio_count(), 1);
                assert_eq!(runtimes[index].engine_token(), tokens[index]);
            }

            let resumed_start_ms = START_MS + 100;
            assert_eq!(
                host.broadcast_control(&ControlMessage::StreamStart(StreamStart {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    host_start_time_ms: MonotonicMillis::new(resumed_start_ms),
                    sample_rate: 48_000,
                    channels: 2,
                    samples_per_packet: PACKET_FRAMES,
                    current_entry_id: Some("second".to_owned()),
                    current_display_name: Some("Second".to_owned()),
                }))
                .expect("broadcast two-listener Resume")
                .report
                .successful_peers,
                2
            );
            for (index, handle) in listeners.iter().enumerate() {
                assert!(
                    matches!(
                        handle.poll_event(1_000).expect("listener Resume"),
                        Some(super::FfiListenerTransportEvent::StreamStarted {
                            stream_id,
                            host_start_time_ms,
                            ..
                        }) if stream_id == stream.as_str()
                            && host_start_time_ms == resumed_start_ms
                    ),
                    "listener {index} must receive the new host anchor"
                );
                runtimes[index]
                    .reanchor_presentation_time(resumed_start_ms)
                    .expect("reanchor existing native output");
                assert_eq!(
                    runtimes[index].engine_token(),
                    tokens[index],
                    "Resume must not reopen listener {index} output"
                );
            }
        }
        let amplitude = if sequence == 0 { 8_192_i16 } else { 16_384_i16 };
        let payload = [amplitude, -amplitude]
            .into_iter()
            .cycle()
            .take(PACKET_FRAMES as usize * 2)
            .flat_map(i16::to_le_bytes)
            .collect();
        assert_eq!(
            host.broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
                session_id: session.clone(),
                stream_id: stream.clone(),
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: PACKET_FRAMES,
                first_sample_index: SampleIndex::new(sequence * u64::from(PACKET_FRAMES)),
                host_presentation_time_ms: MonotonicMillis::new(
                    START_MS + sequence * 5 + if sequence == 1 { 100 } else { 0 },
                ),
                payload,
            }))
            .expect("broadcast production UDP PCM")
            .report
            .successful_peers,
            2
        );
        // Polling an attached PCM packet returns None: a 500 ms poll for
        // each of two listeners would artificially advance the real-time
        // scheduler by two seconds across this tiny two-packet stream.
        // Use short polls with a shared deadline, not a fixed idle wait per
        // peer. This exercises actual UDP forwarding without making the
        // second listener's PCM late merely because the first was polled.
        let deadline = Instant::now() + Duration::from_secs(1);
        for (index, handle) in listeners.iter().enumerate() {
            while handle.forwarded_audio_count() < sequence + 1 {
                assert!(
                    Instant::now() < deadline,
                    "listener {index} never forwarded UDP packet {sequence}"
                );
                assert!(
                    handle.poll_event(20).expect("forward PCM").is_none(),
                    "attached PCM must not enter the foreign event queue"
                );
            }
            assert_eq!(handle.forwarded_audio_count(), sequence + 1);
            assert_eq!(runtimes[index].engine_token(), tokens[index]);
        }
    }

    // Both independent render pumps must be ready under one deadline.
    // Waiting up to five seconds for each in series could let the other
    // pump pass its sample deadline and synthesize concealment before its
    // callback is drained.
    let deadline = Instant::now() + Duration::from_secs(5);
    let prefills = loop {
        let states: Vec<_> = runtimes
            .iter()
            .map(|runtime| runtime.diagnostics())
            .collect();
        if states.iter().all(|state| {
            state.packets_emitted == 2
                && state.ring_queued_frames >= state.prefill_frames + u64::from(PACKET_FRAMES) * 2
        }) {
            break states
                .iter()
                .map(|state| state.prefill_frames)
                .collect::<Vec<_>>();
        }
        assert!(
            Instant::now() < deadline,
            "both listener PCM rings: {states:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for (index, runtime) in runtimes.iter().enumerate() {
        let prefill = prefills[index];
        let engine =
            usize::try_from(tokens[index]).expect("token fits pointer") as *mut core::ffi::c_void;
        let mut silence = vec![f32::NAN; usize::try_from(prefill).expect("prefill fits") * 2];
        let mut supplied = 0_u32;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                silence.as_mut_ptr(),
                u32::try_from(prefill).expect("prefill fits"),
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(u64::from(supplied), prefill);
        assert!(silence.iter().all(|sample| *sample == 0.0));

        let mut pcm = vec![f32::NAN; PACKET_FRAMES as usize * 4];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                pcm.as_mut_ptr(),
                PACKET_FRAMES * 2,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, PACKET_FRAMES * 2);
        for (frame, stereo) in pcm.chunks_exact(2).enumerate() {
            let amplitude = if frame < PACKET_FRAMES as usize {
                8_192_i64
            } else {
                16_384_i64
            };
            let shaped = if frame < DEFAULT_CONCEALMENT_RAMP_MS as usize * 48 {
                amplitude * i64::try_from(frame).expect("bounded frame")
                    / (i64::from(DEFAULT_CONCEALMENT_RAMP_MS) * 48)
            } else {
                amplitude
            };
            let shaped = i16::try_from(shaped).expect("PCM16");
            let expected_left = f32::from(shaped) / 32_768.0;
            let expected_right = f32::from(-shaped) / 32_768.0;
            assert_eq!(
                stereo[0].to_bits(),
                expected_left.to_bits(),
                "listener {index} left {frame}"
            );
            assert_eq!(
                stereo[1].to_bits(),
                expected_right.to_bits(),
                "listener {index} right {frame}"
            );
        }
        assert_eq!(
            silent_disco_audio_frames_rendered(engine),
            prefill + u64::from(PACKET_FRAMES) * 2
        );
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(runtime.engine_token(), tokens[index]);
    }

    // Stop both listeners on their original authorized sockets, then replay
    // through those SAME sockets with fresh native engines. This tests a
    // multi-peer Stop/replay transition beyond cached late-join controls.
    assert_eq!(
        host.broadcast_control(&ControlMessage::Stop(silent_disco_core::protocol::Stop {
            session_id: session.clone(),
            stream_id: stream.clone(),
            host_stop_time_ms: MonotonicMillis::new(START_MS + 20),
        }))
        .expect("broadcast authoritative Stop")
        .report
        .successful_peers,
        2
    );
    for (index, (handle, runtime)) in listeners.iter().zip(&runtimes).enumerate() {
        assert!(
            matches!(
                handle.poll_event(1_000).expect("Stop event"),
                Some(super::FfiListenerTransportEvent::Stopped { stream_id, .. })
                    if stream_id == stream.as_str()
            ),
            "listener {index} must receive Stop"
        );
        handle.detach_playback();
        runtime.stop().expect("retire old output");
        let old_engine =
            usize::try_from(tokens[index]).expect("token fits pointer") as *mut core::ffi::c_void;
        let mut output = [f32::NAN; 2];
        let mut supplied = u32::MAX;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                old_engine,
                output.as_mut_ptr(),
                1,
                2,
                &raw mut supplied,
            ),
            2,
            "listener {index} old output must remain retired"
        );
        assert_eq!(supplied, 0);
    }

    let replay_stream = StreamId::new("callback-late-join-replay").expect("replay stream");
    let replay_start_ms = START_MS + 200;
    let mut replay_runtimes = Vec::new();
    for (index, handle) in listeners.iter().enumerate() {
        let replay = FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
            session_id: session.as_str().to_owned(),
            stream_id: replay_stream.as_str().to_owned(),
            sample_rate: 48_000,
            host_start_time_ms: replay_start_ms,
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
        .expect("fresh replay output");
        replay
            .lock_same_process_host_clock(0)
            .expect("replay clock");
        assert_ne!(replay.engine_token(), tokens[index]);
        handle.attach_playback(Arc::clone(&replay));
        replay_runtimes.push(replay);
    }
    assert_eq!(
        host.broadcast_control(&ControlMessage::StreamStart(StreamStart {
            session_id: session.clone(),
            stream_id: replay_stream.clone(),
            host_start_time_ms: MonotonicMillis::new(replay_start_ms),
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: PACKET_FRAMES,
            current_entry_id: Some("replay-first".to_owned()),
            current_display_name: Some("Replay First".to_owned()),
        }))
        .expect("replay Start")
        .report
        .successful_peers,
        2
    );
    for handle in &listeners {
        assert!(matches!(
            handle.poll_event(1_000).expect("replay Start event"),
            Some(super::FfiListenerTransportEvent::StreamStarted {
                stream_id, host_start_time_ms, ..
            }) if stream_id == replay_stream.as_str() && host_start_time_ms == replay_start_ms
        ));
    }
    assert_eq!(
        host.broadcast_control(&ControlMessage::TrackBoundary(TrackBoundary {
            session_id: session.clone(),
            stream_id: replay_stream.clone(),
            first_sample_index: SampleIndex::new(u64::from(PACKET_FRAMES)),
            entry_id: "replay-second".to_owned(),
            display_name: "Replay Second".to_owned(),
        }))
        .expect("replay boundary")
        .report
        .successful_peers,
        2
    );
    for handle in &listeners {
        assert!(matches!(
            handle.poll_event(1_000).expect("replay boundary event"),
            Some(super::FfiListenerTransportEvent::TrackBoundary {
                stream_id, first_sample_index, ..
            }) if stream_id == replay_stream.as_str()
                && first_sample_index == u64::from(PACKET_FRAMES)
        ));
    }

    // A delayed datagram from the retired stream can arrive on the same
    // authorized UDP route after Stop/replay. The newly attached native
    // engine must reject its old stream identity, not play it as replay PCM.
    let stale_payload = [30_000_i16, -30_000_i16]
        .into_iter()
        .cycle()
        .take(PACKET_FRAMES as usize * 2)
        .flat_map(i16::to_le_bytes)
        .collect();
    assert_eq!(
        host.broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
            session_id: session.clone(),
            stream_id: stream.clone(),
            sequence: PacketSequence::new(2),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: PACKET_FRAMES,
            first_sample_index: SampleIndex::new(u64::from(PACKET_FRAMES) * 2),
            host_presentation_time_ms: MonotonicMillis::new(START_MS + 10),
            payload: stale_payload,
        }))
        .expect("delayed retired-stream UDP")
        .report
        .successful_peers,
        2
    );
    let stale_deadline = Instant::now() + Duration::from_secs(1);
    for (index, handle) in listeners.iter().enumerate() {
        while handle.forwarded_audio_count() < 3 {
            assert!(
                Instant::now() < stale_deadline,
                "listener {index} did not inspect retired-stream UDP"
            );
            assert!(
                handle
                    .poll_event(20)
                    .expect("inspect retired PCM")
                    .is_none()
            );
        }
    }
    assert!(
        replay_runtimes
            .iter()
            .all(|runtime| runtime.diagnostics().packets_emitted == 0),
        "retired-stream UDP must not schedule replay audio"
    );

    for sequence in 0..2_u64 {
        let amplitude = if sequence == 0 { 12_288_i16 } else { 8_192_i16 };
        let payload = [amplitude, -amplitude]
            .into_iter()
            .cycle()
            .take(PACKET_FRAMES as usize * 2)
            .flat_map(i16::to_le_bytes)
            .collect();
        assert_eq!(
            host.broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
                session_id: session.clone(),
                stream_id: replay_stream.clone(),
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: PACKET_FRAMES,
                first_sample_index: SampleIndex::new(sequence * u64::from(PACKET_FRAMES)),
                host_presentation_time_ms: MonotonicMillis::new(replay_start_ms + sequence * 5),
                payload,
            }))
            .expect("replay UDP PCM")
            .report
            .successful_peers,
            2
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        for (index, handle) in listeners.iter().enumerate() {
            while handle.forwarded_audio_count() < sequence + 4 {
                assert!(
                    Instant::now() < deadline,
                    "listener {index} missed replay packet {sequence}"
                );
                assert!(handle.poll_event(20).expect("forward replay PCM").is_none());
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let replay_prefills = loop {
        let states: Vec<_> = replay_runtimes
            .iter()
            .map(|runtime| runtime.diagnostics())
            .collect();
        if states.iter().all(|state| {
            state.packets_emitted == 2
                && state.ring_queued_frames >= state.prefill_frames + u64::from(PACKET_FRAMES) * 2
        }) {
            break states
                .iter()
                .map(|state| state.prefill_frames)
                .collect::<Vec<_>>();
        }
        assert!(Instant::now() < deadline, "both replay rings: {states:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    for (index, replay) in replay_runtimes.iter().enumerate() {
        let replay_token = replay.engine_token();
        let engine =
            usize::try_from(replay_token).expect("token fits pointer") as *mut core::ffi::c_void;
        let prefill = replay_prefills[index];
        let mut silence = vec![f32::NAN; usize::try_from(prefill).expect("prefill fits") * 2];
        let mut supplied = 0_u32;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                silence.as_mut_ptr(),
                u32::try_from(prefill).expect("prefill fits"),
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(u64::from(supplied), prefill);
        assert!(silence.iter().all(|sample| *sample == 0.0));
        let mut pcm = vec![f32::NAN; PACKET_FRAMES as usize * 4];
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                engine,
                pcm.as_mut_ptr(),
                PACKET_FRAMES * 2,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, PACKET_FRAMES * 2);
        for (frame, stereo) in pcm.chunks_exact(2).enumerate() {
            let amplitude = if frame < PACKET_FRAMES as usize {
                12_288_i64
            } else {
                8_192_i64
            };
            let shaped = if frame < DEFAULT_CONCEALMENT_RAMP_MS as usize * 48 {
                amplitude * i64::try_from(frame).expect("bounded frame")
                    / (i64::from(DEFAULT_CONCEALMENT_RAMP_MS) * 48)
            } else {
                amplitude
            };
            let shaped = i16::try_from(shaped).expect("PCM16");
            assert_eq!(
                stereo[0].to_bits(),
                (f32::from(shaped) / 32_768.0).to_bits(),
                "listener {index} replay left {frame}"
            );
            assert_eq!(
                stereo[1].to_bits(),
                (f32::from(-shaped) / 32_768.0).to_bits(),
                "listener {index} replay right {frame}"
            );
        }
        assert_eq!(
            silent_disco_audio_frames_rendered(engine),
            prefill + u64::from(PACKET_FRAMES) * 2
        );
        assert_eq!(silent_disco_audio_underrun_count(engine), 0);
        assert_eq!(
            replay.engine_token(),
            replay_token,
            "boundary cannot reopen output"
        );
        listeners[index].detach_playback();
        replay.stop().expect("stop replay output");
    }

    for (handle, runtime) in listeners.iter().zip(&runtimes) {
        handle.detach_playback();
        runtime.stop().expect("stop native runtime");
        handle.shutdown().expect("close TCP/UDP listener");
    }
    host.shutdown().expect("close TCP/UDP host");
}
