use super::{ProbeResult, duration_micros};
use crate::platform::host_transport::{
    BROADCAST_FRAME_QUEUE_CAPACITY, DesktopHostTransportRuntime,
};
use crate::platform::host_transport_events::DesktopHostTransportEventSink;
use serde::Serialize;
use silent_disco_core::domain::{
    ApprovalMode, DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{AudioCodec, AudioDatagram, PROTOCOL_VERSION, ProtocolFrame};
use silent_disco_core::runtime::{
    AudioEvent, CoreSnapshot, SessionAdvertisement, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportConfig, ListenerTransportNode, ManualTransportClock,
    TransportErrorKind, TransportEvent, TransportFactory, VirtualTransportFactory,
    VirtualTransportNetwork,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LISTENERS: usize = 5;
const PACKETS: u64 = 200;
const LISTENER_QUEUE_CAPACITY: usize = 512;
const PAYLOAD_BYTES: usize = 3_840;
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopTransportQueueMetric {
    listener_count: usize,
    packets_enqueued: u64,
    queue_capacity: usize,
    queue_peak_depth: u64,
    queue_depth_at_end: u64,
    queue_overflows: u64,
    recipients_intended: u64,
    recipients_delivered: u64,
    received_audio_events: u64,
    delivery_severity: &'static str,
    enqueue_elapsed_micros: u64,
    drain_elapsed_micros: u64,
    shutdown_elapsed_micros: u64,
}

#[allow(
    clippy::too_many_lines,
    reason = "the probe keeps one complete production queue lifecycle auditable in one place"
)]
pub(super) fn measure_transport_queue() -> ProbeResult<DesktopTransportQueueMetric> {
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default());
    let session_id = SessionId::new("block45-desktop-transport")?;
    let stream_id = StreamId::new("block45-desktop-stream")?;
    let host_device_id = DeviceId::new("block45-desktop-host")?;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = factory.bind_host(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )?;
    let endpoint = host.endpoint();
    let mut listeners = Vec::with_capacity(LISTENERS);
    for index in 0..LISTENERS {
        let device_id = DeviceId::new(format!("block45-desktop-listener-{index}"))?;
        let mut config =
            ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint);
        config.event_queue_capacity = LISTENER_QUEUE_CAPACITY;
        let listener = factory.connect_listener(config, clock.clone())?;
        host.authorize_peer(&device_id, listener.local_routes())?;
        listeners.push(listener);
    }

    let advertisement = SessionAdvertisement::new(
        session_id.clone(),
        host_device_id,
        "Block 45 performance",
        ApprovalMode::Manual,
        PROTOCOL_VERSION,
        Some(endpoint),
    )?;
    let runtime_clock: Arc<dyn silent_disco_core::transport::TransportClock> = clock;
    let runtime = DesktopHostTransportRuntime::start(
        host,
        advertisement,
        Arc::new(NullTransportSink),
        runtime_clock,
    )?;

    let enqueue_started = Instant::now();
    for sequence in 0..PACKETS {
        runtime.broadcast_frame(audio_frame(&session_id, &stream_id, sequence))?;
    }
    let enqueue_elapsed = enqueue_started.elapsed();

    let drain_started = Instant::now();
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let status = loop {
        let status = runtime.status()?;
        if status.broadcast.frames_attempted >= PACKETS {
            break status;
        }
        if Instant::now() >= deadline {
            return Err("desktop broadcast queue did not drain within the bounded wait".into());
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let drain_elapsed = drain_started.elapsed();

    if status.broadcast.queue_depth != 0 || status.broadcast.queue_overflows != 0 {
        return Err(format!(
            "desktop broadcast queue ended at depth {} with {} overflow(s)",
            status.broadcast.queue_depth, status.broadcast.queue_overflows
        )
        .into());
    }
    let expected_recipients = PACKETS.saturating_mul(u64::try_from(LISTENERS)?);
    if status.broadcast.recipients_intended != expected_recipients
        || status.broadcast.recipients_delivered != expected_recipients
    {
        return Err(format!(
            "desktop transport delivery mismatch: intended {}, delivered {}, expected {expected_recipients}",
            status.broadcast.recipients_intended, status.broadcast.recipients_delivered
        )
        .into());
    }

    let received_audio_events = drain_listener_audio(&mut listeners)?;
    if received_audio_events != expected_recipients {
        return Err(format!(
            "desktop transport listeners received {received_audio_events}; expected {expected_recipients}"
        )
        .into());
    }

    let shutdown_started = Instant::now();
    for listener in &mut listeners {
        listener.shutdown()?;
    }
    runtime.shutdown()?;
    let shutdown_elapsed = shutdown_started.elapsed();

    Ok(DesktopTransportQueueMetric {
        listener_count: LISTENERS,
        packets_enqueued: PACKETS,
        queue_capacity: usize::from(BROADCAST_FRAME_QUEUE_CAPACITY),
        queue_peak_depth: status.broadcast.queue_peak_depth,
        queue_depth_at_end: status.broadcast.queue_depth,
        queue_overflows: status.broadcast.queue_overflows,
        recipients_intended: status.broadcast.recipients_intended,
        recipients_delivered: status.broadcast.recipients_delivered,
        received_audio_events,
        delivery_severity: "ok",
        enqueue_elapsed_micros: duration_micros(enqueue_elapsed)?,
        drain_elapsed_micros: duration_micros(drain_elapsed)?,
        shutdown_elapsed_micros: duration_micros(shutdown_elapsed)?,
    })
}

fn drain_listener_audio(listeners: &mut [Box<dyn ListenerTransportNode>]) -> ProbeResult<u64> {
    let mut received = 0_u64;
    for listener in listeners {
        loop {
            match listener.recv_event(POLL_INTERVAL) {
                Ok(TransportEvent::FrameReceived {
                    frame: ProtocolFrame::Audio(_),
                    ..
                }) => received = received.saturating_add(1),
                Ok(_) => {}
                Err(error) if error.kind == TransportErrorKind::Timeout => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(received)
}

fn audio_frame(session_id: &SessionId, stream_id: &StreamId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(960)),
        host_presentation_time_ms: MonotonicMillis::new(sequence.saturating_mul(20)),
        payload: vec![0; PAYLOAD_BYTES],
    })
}

struct NullTransportSink;

impl DesktopHostTransportEventSink for NullTransportSink {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(CoreSnapshot::default())
    }

    fn submit_transport_event(&self, _event: CoreTransportEvent) -> Result<(), CoreError> {
        Ok(())
    }

    fn submit_audio_event(&self, _event: AudioEvent) -> Result<(), CoreError> {
        Ok(())
    }
}
