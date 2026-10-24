//! Production source -> packetizer -> TCP/UDP -> listener scheduler -> native
//! callback proof across a paused track boundary and an admitted late joiner.
//! Physical OS audio output is intentionally not part of this software test.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, register_render_ring, registry_test_guard,
    release_render_ring, silent_disco_audio_callback_count,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_silence_filled_frames,
    silent_disco_audio_underrun_count,
};
use crate::callback_capture::CallbackCaptureTap;
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig, StreamingDecodeConfig, StreamingDecodeHandle,
};
use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, Pause, ProtocolFrame,
    StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{
    HostTransportNode, ListenerTransportConfig, ListenerTransportNode, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn recv_control(listener: &mut dyn ListenerTransportNode) -> ControlMessage {
    loop {
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } = listener.recv_event(Duration::from_secs(5)).expect("receive TCP control") {
            return message;
        }
    }
}

fn recv_audio(listener: &mut dyn ListenerTransportNode) -> AudioDatagram {
    loop {
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            frame: ProtocolFrame::Audio(packet),
            ..
        } = listener.recv_event(Duration::from_secs(5)).expect("receive UDP audio") {
            return packet;
        }
    }
}

fn authorized_late_joiner(host: &mut dyn HostTransportNode) -> Box<dyn ListenerTransportNode> {
    let session = SessionId::new("end-to-end-session").expect("session");
    let device = DeviceId::new("paused-gapless-late-joiner").expect("device");
    let clock = Arc::new(ManualTransportClock::new(1_010));
    let factory = SocketTransportFactory;
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("late listener connects");
    listener.send_control(&ControlMessage::JoinRequest(JoinRequest {
        session_id: session,
        device: DeviceIdentity {
            device_id: device.clone(),
            display_name: "Late joiner".to_owned(),
        },
        invite_code: None,
        sync_port: 0,
        audio_port: 0,
    })).expect("late join request");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "host did not observe late join");
        if let TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        } = host.recv_event(remaining).expect("host join event") {
            break;
        }
    }
    host.authorize_peer(&device, listener.local_routes()).expect("authorize late joiner");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "host did not authorize late joiner");
        if let TransportEvent::PeerAuthorized { .. } =
            host.recv_event(remaining).expect("host authorization event") {
            break;
        }
    }
    listener
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "integrated real sockets, decoded sources, pause, late join, scheduler and callback"
)]
fn paused_boundary_reconnect_preserves_canonical_pcm_and_native_output_token() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    let root = std::env::temp_dir().join(format!("sd-paused-socket-{unique}"));
    fs::create_dir_all(&root).expect("create fixture directory");
    let formats = [(48_000_u32, 2_u16, 480_usize, 1_024_i16), (44_100, 1, 441, 2_048)];
    let mut canonical = Vec::new();
    let sources: Vec<_> = formats.iter().enumerate().map(|(i, &(rate, channels, frames, sample))| {
        let path = root.join(format!("source-{i}.wav"));
        fs::write(&path, source_wav_format(sample, rate, channels, frames))
            .expect("stage source");
        let decoder = StreamingDecodeHandle::open(&path, StreamingDecodeConfig::default())
            .expect("independent canonical decode");
        loop {
            let chunk = decoder.recv_timeout(Duration::from_secs(5)).expect("decode chunk");
            let end = chunk.end_of_stream;
            canonical.extend(chunk.frames);
            if end { break; }
        }
        decoder.join().expect("join reference decoder");
        FfiPlaylistSource {
            entry_id: format!("entry-{i}"),
            display_name: format!("Track {i}"),
            staged_path: path.to_string_lossy().into_owned(),
        }
    }).collect();
    assert_eq!(canonical.len(), 960 * 2, "two canonical 480-frame tracks");

    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let playlist = FfiHostPlaylistStreamer::open(
        "end-to-end-session".to_owned(), "pause-resume-stream".to_owned(), 1_000, sources
    ).expect("production playlist");
    let start = playlist.stream_start();
    assert_eq!(start.sample_rate, 48_000);
    assert_eq!(start.channels, 2);
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("pause-resume-stream").expect("stream");
    let start_control = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: start.sample_rate,
        channels: start.channels,
        samples_per_packet: start.samples_per_packet,
        current_entry_id: start.current_entry_id,
        current_display_name: start.current_display_name,
    });
    assert_eq!(host.broadcast_control(&start_control).expect("start").report.successful_peers, 1);
    assert!(recv_control(listener.as_mut()) == start_control);

    let mut config = SchedulerConfig::new(
        session.clone(), stream.clone(), 48_000, 1_000, start.samples_per_packet, 2,
    );
    config.startup_buffer_target_ms = 0;
    let ramp = usize::try_from(
        u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000
    ).expect("startup ramp");
    let scheduler = PlaybackScheduler::new(config, 0.0).expect("scheduler");
    let (producer, token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800, target_fill_frames: 4_800,
    }).expect("native output ring");
    let mut pump = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig {
        volume: 1.0, write_lead_ms: 0, max_prefill_ms: 0, target_depth_frames: 4_800,
    }).expect("production listener pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("capture ring"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");

    let mut late = None;
    let mut audio_packets = 0_u64;
    let mut boundaries = Vec::new();
    let mut captured = Vec::<f32>::new();
    let mut callback_chunks = Vec::new();
    let mut done = false;
    for _ in 0..32 {
        match playlist.next_frame(5_000).expect("next producer event") {
            Some(FfiPlaylistFrame::Audio {
                sequence, sample_rate, channels, samples_per_packet,
                first_sample_index, host_presentation_time_ms, payload, ..
            }) => {
                assert_eq!(sequence, audio_packets);
                assert_eq!(first_sample_index, audio_packets * 480);
                assert_eq!(samples_per_packet, 480);
                let shift = if audio_packets == 0 { 0 } else { 200 };
                let packet = AudioDatagram {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    sequence: PacketSequence::new(sequence),
                    codec: AudioCodec::PcmS16Le,
                    sample_rate, channels, samples_per_packet,
                    first_sample_index: SampleIndex::new(first_sample_index),
                    host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms + shift),
                    payload,
                };
                let sent = host.broadcast_audio(&ProtocolFrame::Audio(packet.clone()))
                    .expect("broadcast source PCM via UDP");
                assert_eq!(sent.report.successful_peers, if late.is_some() { 2 } else { 1 });
                let received = recv_audio(listener.as_mut());
                assert!(received == packet, "UDP audio must preserve sample/sequence/timestamp");
                if let Some(ref mut newcomer) = late {
                    assert!(recv_audio(newcomer.as_mut()) == packet, "late joiner audio");
                }
                pump.scheduler_mut().submit_packet(received).expect("schedule UDP PCM");
                let now = host_presentation_time_ms + shift;
                for _ in 0..64 {
                    if !matches!(
                        pump.tick(now),
                        PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
                    ) { break; }
                }
                // Native callbacks do not align to source packet boundaries:
                // 241 + 239 crosses the first track's tail exactly; the
                // successor callback is split into 337 + 143 frames.
                let chunks: &[u32] = if audio_packets == 0 { &[241, 239] } else { &[337, 143] };
                for &frames in chunks {
                    let mut returned = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
                    let mut supplied = 0_u32;
                    let status = silent_disco_audio_read_interleaved_f32(
                        usize::try_from(token).expect("token") as *mut core::ffi::c_void,
                        returned.as_mut_ptr(), frames, 2, &raw mut supplied,
                    );
                    assert_eq!(status, 0, "a boundary or Resume must not underrun");
                    assert_eq!(supplied, frames, "no native inserted silence");
                    assert_eq!(
                        tap.drain(usize::try_from(frames).expect("frames")),
                        returned, "capture tap must equal native callback output"
                    );
                    captured.extend(returned);
                    callback_chunks.push(frames);
                }
                audio_packets += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index, entry_id, display_name, ..
            }) => {
                assert_eq!(first_sample_index, 480);
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(), stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id, display_name,
                });
                host.broadcast_control(&boundary).expect("boundary TCP");
                assert!(recv_control(listener.as_mut()) == boundary);
                boundaries.push(first_sample_index);

                let paused = ControlMessage::Pause(Pause {
                    session_id: session.clone(), stream_id: stream.clone(),
                    host_pause_time_ms: MonotonicMillis::new(1_010),
                });
                host.broadcast_control(&paused).expect("pause before successor");
                assert!(recv_control(listener.as_mut()) == paused);
                let count = host.counters().control_frames_sent;
                host.broadcast_control(&start_control).expect("drop stale start");
                assert_eq!(host.counters().control_frames_sent, count);

                let mut newcomer = authorized_late_joiner(host.as_mut());
                for expected in [&start_control, &boundary, &paused] {
                    assert!(recv_control(newcomer.as_mut()) == *expected, "paused reconnect snapshot");
                }
                let ControlMessage::StreamStart(mut resumed) = start_control.clone() else {
                    panic!("start control expected");
                };
                resumed.host_start_time_ms = MonotonicMillis::new(1_200);
                let resume = ControlMessage::StreamStart(resumed);
                host.broadcast_control(&resume).expect("re-anchor after Pause");
                assert!(recv_control(listener.as_mut()) == resume);
                assert!(recv_control(newcomer.as_mut()) == resume);
                pump.scheduler_mut().set_host_start_time_ms(1_200);
                late = Some(newcomer);
            }
            Some(FfiPlaylistFrame::End) => { done = true; break; }
            None => {}
        }
    }
    assert!(done, "playlist EOF");
    assert_eq!(audio_packets, 2);
    assert_eq!(boundaries, [480]);
    assert_eq!(callback_chunks, [241, 239, 337, 143]);
    assert_eq!(captured.len(), canonical.len());
    let mut expected: Vec<f32> = canonical.iter()
        .map(|sample| f32::from(*sample) / 32768.0).collect();
    for frame in 0..ramp.min(480) {
        for channel in 0..2 {
            let index = frame * 2 + channel;
            let faded = i32::from(canonical[index]) * i32::try_from(frame).expect("ramp position")
                / i32::try_from(ramp).expect("ramp length");
            expected[index] = f32::from(i16::try_from(faded).expect("ramped sample")) / 32768.0;
        }
    }
    assert_eq!(captured, expected, "canonical PCM with exactly one startup fade");
    assert_eq!(pump.diagnostics().packets_emitted, 2);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    let native = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    assert_eq!(silent_disco_audio_callback_count(native), 4);
    assert_eq!(silent_disco_audio_underrun_count(native), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(native), 0);
    assert_eq!(tap.counters(), (960, 0, 4));
    detach_callback_capture(token);
    release_render_ring(token).expect("release same native output after resume");
    playlist.cancel().expect("stop playlist");
    if let Some(mut newcomer) = late {
        newcomer.shutdown().expect("late listener shutdown");
    }
    listener.shutdown().expect("primary listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("remove generated fixtures");
}
