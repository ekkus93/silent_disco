use super::{ProbeResult, duration_micros};
use serde::Serialize;
use silent_disco_core::audio::{PlaybackScheduler, SchedulerConfig, SchedulerPoll};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, SyncConfidence,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};
use std::time::Instant;

const PAYLOAD_BYTES: usize = 3_840;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SynchronizationMetric {
    samples: usize,
    confidence: &'static str,
    offset_ms: f64,
    round_trip_ms: f64,
    jitter_ms: f64,
    skew_ppm: f64,
    elapsed_micros: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SchedulerMetric {
    concealed_packets: u64,
    concealment_driven_rebuffers: u64,
    skipped_sequences: u64,
    elapsed_micros: u64,
}

pub(super) fn measure_synchronization() -> ProbeResult<SynchronizationMetric> {
    let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())?;
    let started = Instant::now();
    for index in 0_u64..12 {
        let t1 = 1_000_u64.saturating_add(index.saturating_mul(100));
        let correlation = SyncCorrelationId::new(index.saturating_add(1));
        estimator.begin_probe(correlation, LocalMonotonicMillis::new(t1))?;
        let observation = estimator.observe_response(
            correlation,
            LocalMonotonicMillis::new(t1),
            HostMonotonicMillis::new(t1.saturating_add(22)),
            HostMonotonicMillis::new(t1.saturating_add(24)),
            LocalMonotonicMillis::new(t1.saturating_add(22)),
        )?;
        if !observation.accepted {
            return Err("Block 45 deterministic sync sample was unexpectedly rejected".into());
        }
    }
    let elapsed = started.elapsed();
    let snapshot = estimator.snapshot();
    let confidence = match snapshot.confidence {
        SyncConfidence::Unknown => "unknown",
        SyncConfidence::Poor => "poor",
        SyncConfidence::Fair => "fair",
        SyncConfidence::Good => "good",
        SyncConfidence::Excellent => "excellent",
    };
    Ok(SynchronizationMetric {
        samples: snapshot.accepted_sample_count,
        confidence,
        offset_ms: snapshot.offset_ms,
        round_trip_ms: snapshot.round_trip_time_ms,
        jitter_ms: snapshot.jitter_ms,
        skew_ppm: snapshot.skew_ppm,
        elapsed_micros: duration_micros(elapsed)?,
    })
}

pub(super) fn measure_scheduler_concealment() -> ProbeResult<SchedulerMetric> {
    let session_id = SessionId::new("block45-scheduler")?;
    let stream_id = StreamId::new("block45-scheduler-stream")?;
    let mut config = SchedulerConfig::new(session_id.clone(), stream_id.clone(), 20, 1_000, 960, 2);
    config.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(config, 0.0)?;
    for sequence in std::iter::once(0_u64).chain(2_u64..=21) {
        scheduler.submit_packet(AudioDatagram {
            session_id: session_id.clone(),
            stream_id: stream_id.clone(),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 960,
            first_sample_index: SampleIndex::new(sequence.saturating_mul(960)),
            host_presentation_time_ms: MonotonicMillis::new(
                1_000_u64.saturating_add(sequence.saturating_mul(20)),
            ),
            payload: vec![0; PAYLOAD_BYTES],
        })?;
    }
    let started = Instant::now();
    for now in [1_000_u64, 1_020, 1_040] {
        if !matches!(scheduler.poll(now), SchedulerPoll::Frame { .. }) {
            return Err("Block 45 scheduler probe did not emit the expected frame".into());
        }
    }
    let elapsed = started.elapsed();
    let concealment = scheduler.concealment_statistics();
    let jitter = scheduler.jitter_statistics();
    if concealment.total_concealed_packets != 1 || concealment.hard_resync_signals != 0 {
        return Err(format!(
            "Block 45 scheduler concealment mismatch: concealed={}, rebuffers={}",
            concealment.total_concealed_packets, concealment.hard_resync_signals
        )
        .into());
    }
    Ok(SchedulerMetric {
        concealed_packets: concealment.total_concealed_packets,
        concealment_driven_rebuffers: concealment.hard_resync_signals,
        skipped_sequences: jitter.skipped,
        elapsed_micros: duration_micros(elapsed)?,
    })
}
