use super::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use super::network::{AddressRecord, DesktopHostNetworkControl, InterfaceRecord, TestHostPorts};
use super::start_playback;
use silent_disco_core::domain::{AppRole, ApprovalMode, DeviceId, MonotonicMillis};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame, SyncRequest, SyncResponse,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, HostDraftPatch, InviteCodePatch,
    PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion, SnapshotRevision,
    TransportEffect,
};
use silent_disco_core::transport::{
    ListenerTransportConfig, ListenerTransportNode, SystemTransportClock, TransportChannel,
    TransportEvent, TransportFactory, production_transport_factory,
};
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn desktop_host_streams_real_audio_and_answers_sync_requests() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; streaming playback coverage remains deterministic"
        );
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );

    start_playback::start(&handle, &network, &registry).expect("start playback");

    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });
    let first_audio = wait_for_audio(&mut *listener);
    assert_eq!(first_audio.session_id, advertisement.session_id);
    assert!(!first_audio.payload.is_empty());

    let correlation_id = 7;
    listener
        .send_sync_request(&SyncRequest {
            session_id: advertisement.session_id.clone(),
            correlation_id,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(0),
        })
        .expect("send sync request");
    let sync_response = wait_for_sync_response(&mut *listener, correlation_id);
    assert_eq!(sync_response.session_id, advertisement.session_id);
    assert!(
        sync_response.t3_host_send_elapsed_ms.get()
            >= sync_response.t2_host_receive_elapsed_ms.get()
    );

    network.stop_playback().expect("stop playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::Stop(_))
    });
    // Stopping is only genuinely done once the actor has left `Playing`.
    // `stop_playback` used to return `Ok` without this ever happening.
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Guards the send-ahead horizon fix in `playback_streamer.rs`: the pump
/// used to pace strictly one packet per `packet_duration_ms` real
/// milliseconds, so a whole short source's worth of packets took roughly
/// `(packet_count - 1) * packet_duration_ms` of real time to arrive. Since
/// `pcm_wav()` is only 100ms (5 packets at 20ms each), the old pacing
/// guaranteed at least 80ms between the first and last packet. The fix lets
/// the pump burst out everything already within the send-ahead horizon
/// immediately, so all 5 packets of this short source should arrive far
/// faster than that.
#[test]
fn desktop_host_bursts_a_short_source_instead_of_pacing_one_packet_per_tick() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; streaming playback coverage remains deterministic"
        );
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );

    start_playback::start(&handle, &network, &registry).expect("start playback");

    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });

    let first_audio = wait_for_audio(&mut *listener);
    let burst_start = Instant::now();
    let mut last_sequence = first_audio.sequence.get();
    let mut packet_count = 1;
    let mut last_packet_at = burst_start;
    // This 100ms/5-packet source reaches natural end-of-file almost
    // immediately once burst-sent, so the real `Stop` broadcast (from the
    // pump's own natural-EOF exit path, not from an explicit stop_playback()
    // call below) can arrive interleaved with these remaining audio frames --
    // watch for it here instead of discarding it, or a later explicit wait
    // for it would time out waiting for a second one that never comes. Audio
    // and control are separate channels with no cross-channel ordering
    // guarantee, so keep draining both until quiescent rather than stopping
    // as soon as `Stop` is seen, which could cut off a still-in-flight
    // audio frame.
    let mut saw_stop = false;
    loop {
        match listener.recv_event(Duration::from_millis(60)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(datagram),
                ..
            }) => {
                assert!(
                    datagram.sequence.get() > last_sequence,
                    "audio sequence must strictly increase"
                );
                last_sequence = datagram.sequence.get();
                packet_count += 1;
                last_packet_at = Instant::now();
            }
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Stop(_)),
                ..
            }) => {
                saw_stop = true;
            }
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout =>
            {
                break;
            }
            Err(error) => panic!("listener transport failed: {error}"),
        }
    }
    let burst_elapsed = last_packet_at - burst_start;
    assert!(
        packet_count >= 5,
        "expected all 5 packets of the 100ms test source, got {packet_count}"
    );
    assert!(
        burst_elapsed < Duration::from_millis(60),
        "remaining packets after the first took {burst_elapsed:?}; the old \
         one-packet-per-tick pacing would need at least ~80ms (4 gaps * 20ms) \
         for this 100ms source -- the send-ahead horizon should burst them \
         out far faster than that"
    );

    // The pump may have already exited and broadcast `Stop` on its own
    // (natural EOF, caught above) before this call ever runs; `stop_playback`
    // is still safe and necessary to clear the network layer's playback slot
    // either way, but only wait for a fresh `Stop` if we haven't seen one yet.
    network.stop_playback().expect("stop playback");
    if !saw_stop {
        wait_for_control(&mut *listener, |message| {
            matches!(message, ControlMessage::Stop(_))
        });
    }

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// `stop_playback` must report a pump that could not finish stopping, rather
/// than returning `Ok` regardless.
///
/// The pump's exit is what broadcasts `Stop` and transitions the actor to
/// `Stopped`, and all three of its shutdown steps -- plus a panicking pump
/// thread -- were discarded (`drop(pump.join())`, `drop(handle.submit_audio_event(..))`).
/// A caller could therefore be told the stream stopped while the session was
/// still `Playing`, with nothing anywhere reporting otherwise.
#[test]
fn stop_playback_reports_a_pump_that_could_not_complete_its_shutdown() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });

    // Take the actor away, so the pump's closing `Stopped` transition cannot
    // be delivered. Stopping must surface that instead of claiming success.
    actor.shutdown().expect("actor shutdown");

    let error = network
        .stop_playback()
        .expect_err("stopping must report that the pump could not finish");
    // Which layer reports it (the actor, here) matters less than that the
    // failure is structured and reaches the caller at all.
    assert!(!error.code.is_empty(), "the failure must carry a stable code");
    assert!(
        !error.message.is_empty(),
        "the reported failure must say what went wrong"
    );

    listener.shutdown().expect("listener shutdown");
    // The actor was deliberately taken away, so the host shutdown reports that
    // too rather than claiming success -- the same property, one layer up.
    network
        .shutdown()
        .expect_err("shutdown must report the missing actor");
}

fn stage_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-source", pcm_wav())
}

/// Comfortably longer than [`SEND_AHEAD_HORIZON_MS`]-worth of playback (via
/// `playback_streamer::SEND_AHEAD_HORIZON_MS`, not directly importable from
/// this test module) so a mid-stream check (a sync request/response
/// round trip, an explicit `stop_playback()`) genuinely happens before
/// natural end-of-file, unlike the short `stage_source` fixture, which now
/// bursts out entirely and reaches natural EOF almost immediately.
fn stage_long_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-long-source", long_pcm_wav())
}

fn stage_wav_source(
    temp: &TempDir,
    source_id: &str,
    wav_bytes: Vec<u8>,
) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, wav_bytes).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(source_id, "source.wav", Some(byte_length), None)
        .expect("descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    (descriptor, registry)
}

/// Drives a real actor through role selection, host draft, and session
/// creation, then binds a real desktop host transport on the given local
/// interface address and completes the advertising handshake.
#[allow(clippy::type_complexity)]
fn start_host_session(
    descriptor: AudioSourceDescriptor,
    interface_name: String,
    interface_index: u32,
    address: Ipv4Addr,
) -> (
    CoreActorRuntime,
    silent_disco_core::runtime::CoreActorHandle,
    Receiver<CoreNotification>,
    silent_disco_core::runtime::SessionAdvertisement,
    Arc<DesktopHostNetworkControl>,
    silent_disco_core::runtime::NetworkEndpoint,
) {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-playback-host").expect("host id")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    next_snapshot(&receiver, 0);
    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Streaming playback host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);
    submit(&handle, 2, CoreCommand::CreateHostSession);
    next_snapshot(&receiver, 3);
    let advertisement_effect = next_effect(&receiver);
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request
    else {
        panic!("expected start advertising effect");
    };

    let network = Arc::new(DesktopHostNetworkControl::with_components(
        Arc::new(FixedInterfaceProvider::new(
            interface_name,
            interface_index,
            address,
        )),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    ));
    let endpoint = network
        .start_host(&advertisement, handle.clone())
        .expect("start desktop host transport");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertisement_effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    next_snapshot(&receiver, 4);
    (actor, handle, receiver, advertisement, network, endpoint)
}

/// Connects a real loopback-bound listener, joins, and drives the approval
/// through a real `CoreCommand::ApproveJoin` -> `TransportEffect` ->
/// `authorize_peer_ports` round trip -- exercising the same wiring a real
/// listener's ports depend on before any sync/audio frame can reach it.
fn join_and_approve_listener(
    address: Ipv4Addr,
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    network: &DesktopHostNetworkControl,
) -> Box<dyn ListenerTransportNode> {
    let listener_id = DeviceId::new("desktop-playback-listener").expect("listener id");
    let mut listener = production_transport_factory()
        .connect_listener(
            ListenerTransportConfig {
                local_address: IpAddr::V4(address),
                ..ListenerTransportConfig::loopback(
                    advertisement.session_id.clone(),
                    listener_id.clone(),
                    endpoint,
                )
            },
            Arc::new(SystemTransportClock::default()),
        )
        .expect("listener connects to desktop host");
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Streaming listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("send join request");
    wait_for_hello(&mut *listener);

    let joined = wait_snapshot(handle, |snapshot| {
        !snapshot.pending_join_requests.is_empty()
    });
    let request_id = joined.pending_join_requests[0].request_id.clone();
    submit(
        handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    listener
}

/// Not part of the automated suite: binds a real desktop host on this
/// machine's real LAN address, prints a real connection payload, and waits
/// for an actual external listener (e.g. a phone on the same Wi-Fi network,
/// pasting the printed payload into the app's "Connect manually" screen) to
/// join before streaming a first long "song" (a 300Hz tone), then switching
/// mid-session to a second, audibly distinct "song" (a 900Hz tone) -- the
/// same stop -> update draft -> start sequence a real user changing tracks
/// would trigger, including a fresh stream ID for the second song. Run
/// explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_a_song_change() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor_a = stage_melody_source(&temp, &registry, "song-a", &C_MAJOR_SCALE_HZ, 1.0, 40);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor_a, interface_name, interface_index, address);

    eprintln!(
        "=== paste this connection payload into the Android app's Connect manually screen ==="
    );
    eprintln!(
        "{{\"hostAddress\":\"{address}\",\"controlPort\":{},\"syncPort\":{},\"audioPort\":{},\"sessionId\":\"{}\",\"protocolVersion\":{},\"inviteCodeRequired\":false,\"expiresAtMs\":null}}",
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        advertisement.session_id.as_str(),
        advertisement.protocol_version,
    );
    eprintln!("waiting up to 8 minutes for a real join request...");

    let deadline = Instant::now() + Duration::from_mins(8);
    let joined = loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if !snapshot.pending_join_requests.is_empty() {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a real join request"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let request = &joined.pending_join_requests[0];
    eprintln!(
        "real join request received from device_id={} display_name={}",
        request.device_id.as_str(),
        request.display_name
    );
    let request_id = request.request_id.clone();
    submit(
        &handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(&receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");
    eprintln!("approved and authorized.");

    eprintln!(
        "=== song 1/2: \"song-a\", an ascending C major scale (do re mi fa so la ti do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-a playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));

    eprintln!("=== switching songs: stopping song-a ===");
    network.stop_playback().expect("stop playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });

    let descending_scale: Vec<f64> = C_MAJOR_SCALE_HZ.iter().rev().copied().collect();
    let descriptor_b = stage_melody_source(&temp, &registry, "song-b", &descending_scale, 1.0, 40);
    let current = handle.current_snapshot().expect("current snapshot");
    submit(
        &handle,
        current.revision.get(),
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor_b.clone()),
            remember_approved_devices: None,
        }),
    );
    wait_snapshot(&handle, |snapshot| {
        snapshot
            .host_draft
            .audio_source
            .as_ref()
            .is_some_and(|source| source.source_id == descriptor_b.source_id)
    });

    eprintln!(
        "=== song 2/2: \"song-b\", a descending C major scale (do ti la so fa mi re do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-b playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

/// The eight notes of an ascending C major scale ("do re mi fa so la ti
/// do"), a sequence a listener can recognize and judge for smoothness by
/// ear far more easily than one sustained tone -- a dropped, repeated, or
/// glitched note is unmistakable, where a gap in a continuous tone can
/// blend into the tone's own texture.
/// Linear fade-in/fade-out applied at each note boundary in
/// [`melody_pcm_wav`], long enough to remove the phase-reset amplitude
/// discontinuity without being long enough to noticeably shorten the note.
const NOTE_FADE_SECONDS: f64 = 0.005;

const C_MAJOR_SCALE_HZ: [f64; 8] = [
    261.63, // C4
    293.66, // D4
    329.63, // E4
    349.23, // F4
    392.00, // G4
    440.00, // A4
    493.88, // B4
    523.25, // C5
];

fn stage_melody_source(
    temp: &TempDir,
    registry: &SelectedSourceRegistry,
    source_id: &str,
    notes_hz: &[f64],
    note_seconds: f64,
    total_seconds: u32,
) -> AudioSourceDescriptor {
    let source_path = temp.path().join(format!("{source_id}.wav"));
    fs::write(
        &source_path,
        melody_pcm_wav(notes_hz, note_seconds, total_seconds),
    )
    .expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        format!("desktop-block-playback-manual-{source_id}"),
        format!("{source_id}.wav"),
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    descriptor
}

/// Cycles through `notes_hz`, holding each for `note_seconds`, until
/// `total_seconds` of audio have been generated. Each note restarts its sine
/// wave at phase zero rather than gliding from the previous note, so a
/// listener hears a clean, deliberate transition, not a bend.
fn melody_pcm_wav(notes_hz: &[f64], note_seconds: f64, total_seconds: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = sample_rate * total_seconds;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames_per_note = (f64::from(sample_rate) * note_seconds) as u32;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    // Each note restarts its sine phase at 0, and the previous note almost
    // never ends on a zero-crossing -- without a fade, that's a real,
    // audible click at every note boundary baked into this fixture's own
    // audio, indistinguishable from a genuine playback-pipeline defect.
    // A short linear fade-in/fade-out at each note's edges removes that
    // amplitude discontinuity so any clicks heard on a real device are
    // attributable to the playback pipeline, not this synthetic source.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fade_frames = (f64::from(sample_rate) * NOTE_FADE_SECONDS) as u32;
    for index in 0..frame_count {
        let note_index =
            usize::try_from(index / frames_per_note).expect("note index") % notes_hz.len();
        let frequency_hz = notes_hz[note_index];
        let index_within_note = index % frames_per_note;
        let time_within_note = f64::from(index_within_note) / f64::from(sample_rate);
        let envelope = if index_within_note < fade_frames {
            f64::from(index_within_note) / f64::from(fade_frames)
        } else if index_within_note >= frames_per_note - fade_frames {
            f64::from(frames_per_note - index_within_note) / f64::from(fade_frames)
        } else {
            1.0
        };
        let sample =
            (time_within_note * frequency_hz * std::f64::consts::TAU).sin() * 12_000.0 * envelope;
        #[allow(clippy::cast_possible_truncation)]
        let sample = sample as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(4_410)
}

/// 3 real seconds -- comfortably longer than the playback pump's send-ahead
/// horizon, so playback is still genuinely running (not yet at natural
/// end-of-file) by the time a mid-stream check runs.
fn long_pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(44_100 * 3)
}

fn square_wave_pcm_wav(frame_count: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frame_count {
        let sample = if index % 64 < 32 {
            8_000_i16
        } else {
            -8_000_i16
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Finds a real, currently-active private-LAN IPv4 interface on this machine,
/// mirroring `network_tests.rs`'s bind-conflict test -- a real production
/// socket bind requires an address genuinely assigned to a local interface,
/// so this cannot be faked the way `network_tests.rs`'s simulated-transport
/// tests fake interface records.
fn real_private_lan_address() -> Option<(String, u32, Ipv4Addr)> {
    let interfaces = netdev::get_interfaces();
    let system_interface = interfaces.into_iter().find(|interface| {
        interface.is_up()
            && (interface.is_running() || interface.is_oper_up())
            && !interface.is_loopback()
            && !interface.is_tun()
            && !interface.is_point_to_point()
            && interface.default
            && interface.ipv4_addrs().iter().any(Ipv4Addr::is_private)
    })?;
    let address = system_interface
        .ipv4_addrs()
        .into_iter()
        .find(Ipv4Addr::is_private)?;
    Some((system_interface.name, system_interface.index, address))
}

struct FixedInterfaceProvider {
    record: InterfaceRecord,
}

impl FixedInterfaceProvider {
    fn new(name: String, index: u32, address: Ipv4Addr) -> Self {
        Self {
            record: InterfaceRecord {
                name,
                index,
                up: true,
                running: true,
                oper_up: true,
                loopback: false,
                point_to_point: false,
                tun: false,
                physical: true,
                default_route: false,
                addresses: vec![AddressRecord {
                    address: IpAddr::V4(address),
                    prefix_length: 24,
                }],
            },
        }
    }
}

impl super::network::NetworkInterfaceProvider for FixedInterfaceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, super::network::DesktopNetworkError> {
        Ok(vec![self.record.clone()])
    }
}

fn submit(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    revision: u64,
    command: CoreCommand,
) {
    handle
        .submit_command(
            CoreCommandRequest::new(SnapshotRevision::new(revision), command).expect("command"),
        )
        .expect("submit command");
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot)) if snapshot.revision.get() >= minimum => {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for snapshot"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::PlatformEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for transport effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn wait_snapshot(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for actor state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_hello(listener: &mut dyn ListenerTransportNode) {
    wait_for_control(listener, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
}

fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    matches_expected: impl Fn(&ControlMessage) -> bool,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) if matches_expected(&message) => return,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for expected control message"
        );
    }
}

fn wait_for_audio(
    listener: &mut dyn ListenerTransportNode,
) -> silent_disco_core::protocol::AudioDatagram {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(datagram),
                ..
            }) => return datagram,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for an audio datagram"
        );
    }
}

fn wait_for_sync_response(
    listener: &mut dyn ListenerTransportNode,
    correlation_id: u64,
) -> SyncResponse {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncResponse(response),
                ..
            }) if response.correlation_id == correlation_id => return response,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a sync response"
        );
    }
}
