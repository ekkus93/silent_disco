//! Desktop Block 45 performance/soak probe.
//!
//! This binary deliberately drives production shared-core components instead
//! of benchmark-only substitutes: `StreamingDecodeHandle`,
//! `StreamingPacketizeHandle`, `VirtualTransportFactory` (including the real
//! wire codec), and `DatabaseWorker`. It emits one JSON document and applies
//! only correctness/resource-bound invariants. Timing thresholds are
//! intentionally absent until repeated measurements justify them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use silent_disco_core::audio::{
    StreamingDecodeConfig, StreamingDecodeHandle, StreamingPacketizeConfig,
    StreamingPacketizeHandle,
};
use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, TuningSettings,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram, ProtocolFrame};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker, StoredSettings};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, TransportErrorKind, TransportEvent, TransportFactory,
    VirtualTransportFactory, VirtualTransportNetwork, VirtualUdpFaultConfig,
};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use tempfile::TempDir;

type ProbeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const CANONICAL_SAMPLE_RATE_HZ: u64 = 48_000;
const TRANSPORT_PAYLOAD_BYTES: usize = 3_840;
const DEFAULT_DECODE_ITERATIONS: u32 = 100;
const DEFAULT_PACKETIZER_SOURCE_SECONDS: u32 = 30;
const DEFAULT_TRANSPORT_PACKETS: u32 = 500;
const DEFAULT_SOAK_LISTENERS: usize = 16;
const DEFAULT_SOAK_LOSS_PERMILLE: u16 = 25;
const DEFAULT_SOAK_PACKET_CADENCE_MS: u64 = 20;
const POLL_TIMEOUT: Duration = Duration::from_millis(2);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    schema_version: u32,
    environment: EnvironmentMetric,
    decoder: Vec<DecoderMetric>,
    packetizer: PacketizerMetric,
    transport: Vec<TransportMetric>,
    reconnect: ReconnectMetric,
    database: DatabaseMetric,
    soak: Option<SoakMetric>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentMetric {
    operating_system: &'static str,
    architecture: &'static str,
    logical_cpus: usize,
    rss_kib_at_start: Option<u64>,
    high_water_rss_kib_at_start: Option<u64>,
    cpu_user_ticks_at_start: Option<u64>,
    cpu_system_ticks_at_start: Option<u64>,
    clock_ticks_per_second: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecoderMetric {
    format: String,
    iterations: u32,
    encoded_bytes: u64,
    emitted_frames: u64,
    elapsed_ms: u64,
    frames_per_second: u64,
    realtime_multiple_milli: u64,
    queue_high_water_chunks: usize,
    queue_capacity_chunks: usize,
    backpressure_events: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PacketizerMetric {
    source_seconds: u32,
    emitted_packets: u64,
    elapsed_ms: u64,
    packets_per_second: u64,
    packet_queue_high_water: usize,
    packet_queue_capacity: usize,
    packet_backpressure_events: u64,
    decode_queue_high_water: usize,
    decode_queue_capacity: usize,
    decode_backpressure_events: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransportMetric {
    listener_count: usize,
    configured_loss_permille: u16,
    packets_broadcast: u32,
    intended_peer_deliveries: u64,
    successful_peer_deliveries: u64,
    failed_peer_deliveries: u64,
    received_audio_events: u64,
    expected_audio_events_without_faults: u64,
    observed_receive_loss_permille: u64,
    encoded_bytes_sent: u64,
    send_elapsed_ms: u64,
    peer_deliveries_per_second: u64,
    transport_delivery_failures: u64,
    shutdown_elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReconnectMetric {
    listener_count: usize,
    reconnect_elapsed_ms: u64,
    post_reconnect_audio_received: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseMetric {
    read_iterations: u32,
    write_iterations: u32,
    read_elapsed_ms: u64,
    write_elapsed_ms: u64,
    average_read_latency_micros: u64,
    average_write_latency_micros: u64,
    shutdown_elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SoakMetric {
    requested_seconds: u64,
    listener_count: usize,
    configured_loss_permille: u16,
    packets_broadcast: u64,
    received_audio_events: u64,
    expected_audio_events_without_faults: u64,
    observed_receive_loss_permille: u64,
    transport_delivery_failures: u64,
    rss_kib_at_end: Option<u64>,
    high_water_rss_kib_at_end: Option<u64>,
    cpu_user_ticks_at_end: Option<u64>,
    cpu_system_ticks_at_end: Option<u64>,
    shutdown_elapsed_ms: u64,
}

fn main() -> ProbeResult<()> {
    let temp = TempDir::new()?;
    let decode_iterations = env_u32(
        "SILENT_DISCO_PERF_DECODE_ITERATIONS",
        DEFAULT_DECODE_ITERATIONS,
    );
    let packetizer_seconds = env_u32(
        "SILENT_DISCO_PERF_PACKETIZER_SOURCE_SECONDS",
        DEFAULT_PACKETIZER_SOURCE_SECONDS,
    );
    let transport_packets = env_u32(
        "SILENT_DISCO_PERF_TRANSPORT_PACKETS",
        DEFAULT_TRANSPORT_PACKETS,
    );
    let soak_seconds = env_u64("SILENT_DISCO_PERF_SOAK_SECONDS", 0);

    let environment = environment_metric();
    let decoder = ["wav", "flac", "mp3"]
        .into_iter()
        .map(|extension| measure_decoder(temp.path(), extension, decode_iterations))
        .collect::<ProbeResult<Vec<_>>>()?;
    let packetizer = measure_packetizer(temp.path(), packetizer_seconds)?;

    let mut transport = Vec::new();
    for listener_count in [1, 2, 5, 16] {
        transport.push(measure_transport(listener_count, 0, transport_packets)?);
    }
    transport.push(measure_transport(5, 50, transport_packets)?);
    transport.push(measure_transport(16, 50, transport_packets)?);

    let reconnect = measure_reconnect()?;
    let database = measure_database(temp.path())?;
    let soak = if soak_seconds == 0 {
        None
    } else {
        Some(measure_soak(soak_seconds)?)
    };

    let report = ProbeReport {
        schema_version: 1,
        environment,
        decoder,
        packetizer,
        transport,
        reconnect,
        database,
        soak,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn environment_metric() -> EnvironmentMetric {
    let (rss_kib, high_water_rss_kib) = linux_memory_kib();
    let (cpu_user_ticks, cpu_system_ticks) = linux_cpu_ticks();
    EnvironmentMetric {
        operating_system: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        logical_cpus: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        rss_kib_at_start: rss_kib,
        high_water_rss_kib_at_start: high_water_rss_kib,
        cpu_user_ticks_at_start: cpu_user_ticks,
        cpu_system_ticks_at_start: cpu_system_ticks,
        clock_ticks_per_second: std::env::var("SILENT_DISCO_PERF_CLK_TCK")
            .ok()
            .and_then(|value| value.parse().ok()),
    }
}

fn measure_decoder(root: &Path, extension: &str, iterations: u32) -> ProbeResult<DecoderMetric> {
    let bytes = fixture_bytes(extension)?;
    let path = root.join(format!("block45-source.{extension}"));
    fs::write(&path, &bytes)?;
    let started = Instant::now();
    let mut emitted_frames = 0_u64;
    let mut queue_high_water = 0_usize;
    let mut queue_capacity = 0_usize;
    let mut backpressure_events = 0_u64;

    for _ in 0..iterations {
        let handle = StreamingDecodeHandle::open(&path, StreamingDecodeConfig::default())?;
        let reader = handle.statistics_reader();
        loop {
            let chunk = handle.recv_timeout(Duration::from_secs(5))?;
            emitted_frames = emitted_frames.saturating_add(u64::try_from(chunk.frame_count())?);
            let statistics = reader.snapshot();
            queue_high_water = queue_high_water.max(statistics.queued_chunks);
            queue_capacity = statistics.queue_capacity_chunks;
            if chunk.end_of_stream {
                break;
            }
        }
        let summary = handle.join()?;
        backpressure_events = backpressure_events.saturating_add(summary.backpressure_events);
    }

    let elapsed_ms = duration_ms(started.elapsed());
    let frames_per_second = per_second(emitted_frames, elapsed_ms);
    let decoded_audio_ms = emitted_frames.saturating_mul(1_000) / CANONICAL_SAMPLE_RATE_HZ;
    let realtime_multiple_milli = decoded_audio_ms.saturating_mul(1_000) / elapsed_ms.max(1);
    Ok(DecoderMetric {
        format: extension.to_owned(),
        iterations,
        encoded_bytes: u64::try_from(bytes.len())?.saturating_mul(u64::from(iterations)),
        emitted_frames,
        elapsed_ms,
        frames_per_second,
        realtime_multiple_milli,
        queue_high_water_chunks: queue_high_water,
        queue_capacity_chunks: queue_capacity,
        backpressure_events,
    })
}

fn measure_packetizer(root: &Path, source_seconds: u32) -> ProbeResult<PacketizerMetric> {
    let source_path = root.join("block45-packetizer.wav");
    fs::write(&source_path, pcm_wav(source_seconds)?)?;
    let decoder = StreamingDecodeHandle::open(&source_path, StreamingDecodeConfig::default())?;
    let decoder_reader = decoder.statistics_reader();
    let packetizer = StreamingPacketizeHandle::spawn(
        decoder,
        SessionId::new("block45-packetizer-session")?,
        StreamId::new("block45-packetizer-stream")?,
        MonotonicMillis::new(0),
        StreamingPacketizeConfig::default(),
    )?;
    let packetizer_reader = packetizer.statistics_reader();
    let started = Instant::now();
    let mut emitted_packets = 0_u64;
    let mut packet_queue_high_water = 0_usize;
    let mut decode_queue_high_water = 0_usize;

    loop {
        match packetizer.recv_timeout(Duration::from_secs(5)) {
            Ok(ProtocolFrame::Audio(_)) => emitted_packets = emitted_packets.saturating_add(1),
            Ok(_) => return Err("packetizer emitted a non-audio frame".into()),
            Err(RecvTimeoutError::Timeout) => {
                return Err("packetizer timed out before completion".into());
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let (queued, _, _, _) = packetizer_reader.snapshot();
        packet_queue_high_water = packet_queue_high_water.max(queued);
        decode_queue_high_water =
            decode_queue_high_water.max(decoder_reader.snapshot().queued_chunks);
    }

    let elapsed_ms = duration_ms(started.elapsed());
    let packetizer_summary = packetizer.join()?;
    let (_, packet_queue_capacity, packet_backpressure_events, _) = packetizer_reader.snapshot();
    let decode_statistics = decoder_reader.snapshot();
    Ok(PacketizerMetric {
        source_seconds,
        emitted_packets,
        elapsed_ms,
        packets_per_second: per_second(emitted_packets, elapsed_ms),
        packet_queue_high_water,
        packet_queue_capacity,
        packet_backpressure_events: packetizer_summary
            .backpressure_events
            .max(packet_backpressure_events),
        decode_queue_high_water,
        decode_queue_capacity: decode_statistics.queue_capacity_chunks,
        decode_backpressure_events: decode_statistics.backpressure_events,
    })
}

fn measure_transport(
    listener_count: usize,
    loss_permille: u16,
    packets: u32,
) -> ProbeResult<TransportMetric> {
    let network = VirtualTransportNetwork::default();
    let base_factory = VirtualTransportFactory::new(network);
    let factory: Box<dyn TransportFactory> = if loss_permille == 0 {
        Box::new(base_factory)
    } else {
        Box::new(base_factory.with_udp_faults(VirtualUdpFaultConfig {
            seed: 45,
            loss_permille,
            ..VirtualUdpFaultConfig::default()
        }))
    };
    let session_id = SessionId::new(format!(
        "block45-transport-{listener_count}-{loss_permille}"
    ))?;
    let stream_id = StreamId::new("block45-transport-stream")?;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host_config = HostTransportConfig::loopback(session_id.clone());
    host_config.event_queue_capacity = (listener_count.saturating_mul(2).saturating_add(8)).max(64);
    let mut host = factory.bind_host(host_config, clock.clone())?;
    let mut listeners = connect_and_authorize_listeners(
        factory.as_ref(),
        &mut *host,
        &session_id,
        clock.clone(),
        listener_count,
        usize::try_from(packets)?.saturating_add(64),
    )?;

    let started = Instant::now();
    let mut intended = 0_u64;
    let mut successful = 0_u64;
    let mut failed = 0_u64;
    let mut bytes_sent = 0_u64;
    for sequence in 0..u64::from(packets) {
        let delivery = host.broadcast_audio(&audio_frame(&session_id, &stream_id, sequence))?;
        intended = intended.saturating_add(u64::from(delivery.report.intended_peers));
        successful = successful.saturating_add(u64::from(delivery.report.successful_peers));
        failed = failed.saturating_add(u64::from(delivery.report.failed_peers));
        bytes_sent = bytes_sent.saturating_add(delivery.bytes_sent);
    }
    let send_elapsed_ms = duration_ms(started.elapsed());
    let received_audio_events = drain_audio_events(&mut listeners)?;
    let expected = u64::from(packets).saturating_mul(u64::try_from(listener_count)?);
    if loss_permille == 0 && received_audio_events != expected {
        return Err(format!(
            "zero-fault transport lost events: expected {expected}, received {received_audio_events}"
        )
        .into());
    }
    if failed != 0 {
        return Err(format!("transport send reported {failed} failed peer deliveries").into());
    }
    let counters = host.counters();
    let shutdown_started = Instant::now();
    for (_, listener) in &mut listeners {
        listener.shutdown()?;
    }
    host.shutdown()?;
    let shutdown_elapsed_ms = duration_ms(shutdown_started.elapsed());

    Ok(TransportMetric {
        listener_count,
        configured_loss_permille: loss_permille,
        packets_broadcast: packets,
        intended_peer_deliveries: intended,
        successful_peer_deliveries: successful,
        failed_peer_deliveries: failed,
        received_audio_events,
        expected_audio_events_without_faults: expected,
        observed_receive_loss_permille: loss_permille_from_counts(expected, received_audio_events),
        encoded_bytes_sent: bytes_sent,
        send_elapsed_ms,
        peer_deliveries_per_second: per_second(successful, send_elapsed_ms),
        transport_delivery_failures: counters.delivery_failures,
        shutdown_elapsed_ms,
    })
}

fn measure_reconnect() -> ProbeResult<ReconnectMetric> {
    let listener_count = 5;
    let network = VirtualTransportNetwork::default();
    let factory = VirtualTransportFactory::new(network);
    let session_id = SessionId::new("block45-reconnect-session")?;
    let stream_id = StreamId::new("block45-reconnect-stream")?;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = factory.bind_host(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )?;
    let mut listeners = connect_and_authorize_listeners(
        &factory,
        &mut *host,
        &session_id,
        clock.clone(),
        listener_count,
        128,
    )?;
    let (device_id, mut disconnected) = listeners.remove(0);
    disconnected.shutdown()?;

    let reconnect_started = Instant::now();
    let mut config =
        ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), host.endpoint());
    config.event_queue_capacity = 128;
    let mut replacement = factory.connect_listener(config, clock)?;
    host.authorize_peer(&device_id, replacement.local_routes())?;
    let reconnect_elapsed_ms = duration_ms(reconnect_started.elapsed());
    host.broadcast_audio(&audio_frame(&session_id, &stream_id, 1))?;
    let post_reconnect_audio_received = matches!(
        replacement.recv_event(Duration::from_secs(1)),
        Ok(TransportEvent::FrameReceived {
            frame: ProtocolFrame::Audio(_),
            ..
        })
    );
    if !post_reconnect_audio_received {
        return Err("reconnected virtual listener did not receive audio".into());
    }

    replacement.shutdown()?;
    for (_, listener) in &mut listeners {
        listener.shutdown()?;
    }
    host.shutdown()?;
    Ok(ReconnectMetric {
        listener_count,
        reconnect_elapsed_ms,
        post_reconnect_audio_received,
    })
}

fn measure_database(root: &Path) -> ProbeResult<DatabaseMetric> {
    let config = DatabaseConfig::new(root.join("block45-performance.sqlite3"))?;
    let worker = DatabaseWorker::start(config)?;
    let client = worker.client();
    let read_iterations = 500_u32;
    let write_iterations = 100_u32;
    let settings = StoredSettings {
        tuning: TuningSettings::default(),
        updated_at_ms: 1,
    };
    client.save_settings(&settings)?;

    let read_started = Instant::now();
    for _ in 0..read_iterations {
        let loaded = client.load_settings()?;
        if loaded.is_none() {
            return Err("database settings disappeared during read latency probe".into());
        }
    }
    let read_elapsed = read_started.elapsed();

    let write_started = Instant::now();
    for index in 0..write_iterations {
        client.save_settings(&StoredSettings {
            tuning: TuningSettings::default(),
            updated_at_ms: u64::from(index).saturating_add(2),
        })?;
    }
    let write_elapsed = write_started.elapsed();
    let shutdown_started = Instant::now();
    worker.stop_and_join()?;

    Ok(DatabaseMetric {
        read_iterations,
        write_iterations,
        read_elapsed_ms: duration_ms(read_elapsed),
        write_elapsed_ms: duration_ms(write_elapsed),
        average_read_latency_micros: average_micros(read_elapsed, read_iterations),
        average_write_latency_micros: average_micros(write_elapsed, write_iterations),
        shutdown_elapsed_ms: duration_ms(shutdown_started.elapsed()),
    })
}

fn measure_soak(seconds: u64) -> ProbeResult<SoakMetric> {
    let network = VirtualTransportNetwork::default();
    let factory = VirtualTransportFactory::new(network).with_udp_faults(VirtualUdpFaultConfig {
        seed: 45_045,
        loss_permille: DEFAULT_SOAK_LOSS_PERMILLE,
        ..VirtualUdpFaultConfig::default()
    });
    let session_id = SessionId::new("block45-soak-session")?;
    let stream_id = StreamId::new("block45-soak-stream")?;
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host_config = HostTransportConfig::loopback(session_id.clone());
    host_config.event_queue_capacity = 128;
    let mut host = factory.bind_host(host_config, clock.clone())?;
    let mut listeners = connect_and_authorize_listeners(
        &factory,
        &mut *host,
        &session_id,
        clock.clone(),
        DEFAULT_SOAK_LISTENERS,
        128,
    )?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut sequence = 0_u64;
    let mut received = 0_u64;
    while Instant::now() < deadline {
        host.broadcast_audio(&audio_frame(&session_id, &stream_id, sequence))?;
        clock.advance(DEFAULT_SOAK_PACKET_CADENCE_MS);
        for (_, listener) in &mut listeners {
            match listener.recv_event(POLL_TIMEOUT) {
                Ok(TransportEvent::FrameReceived {
                    frame: ProtocolFrame::Audio(_),
                    ..
                }) => received = received.saturating_add(1),
                Ok(_) => {}
                Err(error) if error.kind == TransportErrorKind::Timeout => {}
                Err(error) => return Err(error.into()),
            }
        }
        sequence = sequence.saturating_add(1);
        std::thread::sleep(Duration::from_millis(DEFAULT_SOAK_PACKET_CADENCE_MS));
    }

    let counters = host.counters();
    let expected = sequence.saturating_mul(u64::try_from(DEFAULT_SOAK_LISTENERS)?);
    let shutdown_started = Instant::now();
    for (_, listener) in &mut listeners {
        listener.shutdown()?;
    }
    host.shutdown()?;
    let (rss_kib, high_water_rss_kib) = linux_memory_kib();
    let (cpu_user_ticks, cpu_system_ticks) = linux_cpu_ticks();
    Ok(SoakMetric {
        requested_seconds: seconds,
        listener_count: DEFAULT_SOAK_LISTENERS,
        configured_loss_permille: DEFAULT_SOAK_LOSS_PERMILLE,
        packets_broadcast: sequence,
        received_audio_events: received,
        expected_audio_events_without_faults: expected,
        observed_receive_loss_permille: loss_permille_from_counts(expected, received),
        transport_delivery_failures: counters.delivery_failures,
        rss_kib_at_end: rss_kib,
        high_water_rss_kib_at_end: high_water_rss_kib,
        cpu_user_ticks_at_end: cpu_user_ticks,
        cpu_system_ticks_at_end: cpu_system_ticks,
        shutdown_elapsed_ms: duration_ms(shutdown_started.elapsed()),
    })
}

fn connect_and_authorize_listeners<F: TransportFactory + ?Sized>(
    factory: &F,
    host: &mut dyn HostTransportNode,
    session_id: &SessionId,
    clock: Arc<ManualTransportClock>,
    listener_count: usize,
    event_queue_capacity: usize,
) -> ProbeResult<Vec<(DeviceId, Box<dyn ListenerTransportNode>)>> {
    let mut listeners = Vec::with_capacity(listener_count);
    for index in 0..listener_count {
        let device_id = DeviceId::new(format!("block45-listener-{index}"))?;
        let mut config = ListenerTransportConfig::loopback(
            session_id.clone(),
            device_id.clone(),
            host.endpoint(),
        );
        config.event_queue_capacity = event_queue_capacity.min(8_192).max(64);
        let listener = factory.connect_listener(config, clock.clone())?;
        host.authorize_peer(&device_id, listener.local_routes())?;
        listeners.push((device_id, listener));
    }
    Ok(listeners)
}

fn drain_audio_events(
    listeners: &mut [(DeviceId, Box<dyn ListenerTransportNode>)],
) -> ProbeResult<u64> {
    let mut received = 0_u64;
    for (_, listener) in listeners {
        loop {
            match listener.recv_event(POLL_TIMEOUT) {
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
        payload: vec![0; TRANSPORT_PAYLOAD_BYTES],
    })
}

fn fixture_bytes(extension: &str) -> ProbeResult<Vec<u8>> {
    let encoded = match extension {
        "wav" => {
            include_str!("../../../../rust/silent-disco-core/src/audio/fixtures/short.wav.b64")
        }
        "flac" => {
            include_str!("../../../../rust/silent-disco-core/src/audio/fixtures/short.flac.b64")
        }
        "mp3" => {
            include_str!("../../../../rust/silent-disco-core/src/audio/fixtures/short.mp3.b64")
        }
        _ => return Err(format!("unsupported performance fixture extension: {extension}").into()),
    };
    let compact: String = encoded
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    Ok(STANDARD.decode(compact)?)
}

fn pcm_wav(seconds: u32) -> ProbeResult<Vec<u8>> {
    let channels = 2_u32;
    let bytes_per_sample = 2_u32;
    let sample_rate = 48_000_u32;
    let frames = sample_rate
        .checked_mul(seconds)
        .ok_or("Block 45 WAV frame count overflow")?;
    let data_len = frames
        .checked_mul(channels)
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or("Block 45 WAV data length overflow")?;
    let riff_len = 36_u32
        .checked_add(data_len)
        .ok_or("Block 45 WAV RIFF length overflow")?;
    let byte_rate = sample_rate
        .checked_mul(channels)
        .and_then(|value| value.checked_mul(bytes_per_sample))
        .ok_or("Block 45 WAV byte rate overflow")?;
    let block_align = u16::try_from(channels.saturating_mul(bytes_per_sample))?;
    let capacity = usize::try_from(data_len)?.saturating_add(44);
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_len.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(channels)?.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(capacity, 0);
    Ok(bytes)
}

fn linux_memory_kib() -> (Option<u64>, Option<u64>) {
    let Ok(status) = fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    (
        proc_status_kib(&status, "VmRSS:"),
        proc_status_kib(&status, "VmHWM:"),
    )
}

fn proc_status_kib(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.trim();
        value.split_whitespace().next()?.parse().ok()
    })
}

fn linux_cpu_ticks() -> (Option<u64>, Option<u64>) {
    let Ok(stat) = fs::read_to_string("/proc/self/stat") else {
        return (None, None);
    };
    let Some(comm_end) = stat.rfind(')') else {
        return (None, None);
    };
    let fields: Vec<&str> = stat[comm_end.saturating_add(1)..]
        .split_whitespace()
        .collect();
    if fields.len() <= 12 {
        return (None, None);
    }
    (fields[11].parse().ok(), fields[12].parse().ok())
}

fn loss_permille_from_counts(expected: u64, received: u64) -> u64 {
    if expected == 0 {
        return 0;
    }
    expected.saturating_sub(received).saturating_mul(1_000) / expected
}

fn per_second(count: u64, elapsed_ms: u64) -> u64 {
    count.saturating_mul(1_000) / elapsed_ms.max(1)
}

fn average_micros(elapsed: Duration, iterations: u32) -> u64 {
    let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    micros / u64::from(iterations).max(1)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
