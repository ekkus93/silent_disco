//! CPU cost of v4 QUIC versus v3 sockets for the audio stream (remediation
//! §1). Ignored by default: it is a measurement, not a pass/fail check, and
//! its numbers depend on the machine. Run with
//! `cargo test -p silent-disco-ffi --lib transport_cpu_cost -- --ignored --nocapture`.

use super::callback_pcm_tests::{Wire, transports};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
};
use silent_disco_core::transport::{ManualTransportClock, TransportChannel, TransportEvent};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 20 s of audio in 5 ms packets of 240 stereo frames.
const PACKETS: u64 = 4_000;
const PACKET_FRAMES: u32 = 240;

/// This process's user plus system CPU time. `/proc` reports it in
/// `USER_HZ` ticks, which Linux fixes at 100 per second.
fn process_cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    // The command name is in parentheses and may contain spaces; fields
    // after it are space-separated. utime and stime are fields 14 and 15.
    let after = &stat[stat.rfind(')').expect("comm") + 2..];
    let fields: Vec<&str> = after.split(' ').collect();
    let ticks: u64 =
        fields[11].parse::<u64>().expect("utime") + fields[12].parse::<u64>().expect("stime");
    Duration::from_millis(ticks * 10)
}

/// Streams [`PACKETS`] paced at their real 5 ms cadence and returns the CPU
/// time spent and how many packets the listener received.
fn stream_cost(wire: Wire) -> (Duration, u64) {
    let session = SessionId::new("cpu-session").expect("session");
    let stream = StreamId::new("cpu-stream").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let (mut host, listener, device) = transports(wire, &session, &clock);
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session.clone(),
            device: DeviceIdentity {
                device_id: device.clone(),
                display_name: "cpu".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("join");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        } = host
            .recv_event(deadline.saturating_duration_since(Instant::now()))
            .expect("join event")
        {
            break;
        }
        assert!(Instant::now() < deadline, "no join");
    }
    host.authorize_peer(&device, routes).expect("authorize");

    // Stereo s16 PCM: 4 bytes per frame of a repeating 0..=255 pattern.
    let payload: Vec<u8> = (0..=u8::MAX)
        .cycle()
        .take(PACKET_FRAMES as usize * 4)
        .collect();
    let receiving = std::thread::spawn(move || {
        let mut received = 0_u64;
        let quiet = Duration::from_millis(500);
        while let Ok(event) = listener.recv_event(quiet) {
            if let TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                ..
            } = event
            {
                received += 1;
            }
            if received == PACKETS {
                break;
            }
        }
        received
    });
    let started = process_cpu();
    let wall = Instant::now();
    for sequence in 0..PACKETS {
        let _ = host.broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
            session_id: session.clone(),
            stream_id: stream.clone(),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: PACKET_FRAMES,
            first_sample_index: SampleIndex::new(sequence * u64::from(PACKET_FRAMES)),
            host_presentation_time_ms: MonotonicMillis::new(1_000 + sequence * 5),
            payload: payload.clone(),
        }));
        // Real-time pacing, as the host stream loop sends.
        let due = Duration::from_millis(5 * (sequence + 1));
        if let Some(wait) = due.checked_sub(wall.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    let received = receiving.join().expect("receiver");
    let cpu = process_cpu().saturating_sub(started);
    let _ = host.shutdown();
    (cpu, received)
}

#[test]
#[ignore = "measurement: reports CPU cost per audio second for v3 and v4; run on demand"]
fn transport_cpu_cost_of_v4_against_v3() {
    let audio_seconds = f64::from(u32::try_from(PACKETS * 5).expect("small")) / 1_000.0;
    for wire in [Wire::Socket, Wire::Quic] {
        let (cpu, received) = stream_cost(wire);
        println!(
            "{wire:?}: {:.1} ms CPU per audio second ({:.2}% of one core), {received}/{PACKETS} packets received",
            cpu.as_secs_f64() * 1_000.0 / audio_seconds,
            cpu.as_secs_f64() * 100.0 / audio_seconds,
        );
        assert!(
            received > PACKETS * 9 / 10,
            "{wire:?} lost too many packets to measure"
        );
    }
}
