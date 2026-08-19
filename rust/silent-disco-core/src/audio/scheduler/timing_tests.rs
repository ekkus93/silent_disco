//! Exact sample-geometry presentation-time regressions.

use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram};

use super::{PlaybackScheduler, SchedulerConfig, SchedulerPoll};

#[test]
fn non_integer_packet_duration_does_not_accumulate_millisecond_truncation() {
    const SAMPLE_RATE: u32 = 48_000;
    const SAMPLES_PER_PACKET: u32 = 1_024;
    const HOST_START_MS: u64 = 10_000;
    const SEQUENCE: u64 = 47;

    let session_id = SessionId::new("timing-session").expect("session id");
    let stream_id = StreamId::new("timing-stream").expect("stream id");
    let mut config = SchedulerConfig::new(
        session_id.clone(),
        stream_id.clone(),
        SAMPLE_RATE,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    config.startup_buffer_target_ms = 0;
    let mut scheduler = PlaybackScheduler::new(config, 0.0).expect("valid scheduler");

    let first_sample_index = SEQUENCE * u64::from(SAMPLES_PER_PACKET);
    let exact_elapsed_ms = first_sample_index * 1_000 / u64::from(SAMPLE_RATE);
    assert_eq!(exact_elapsed_ms, 1_002);
    scheduler
        .submit_packet(AudioDatagram {
            session_id,
            stream_id,
            sequence: PacketSequence::new(SEQUENCE),
            codec: AudioCodec::PcmS16Le,
            sample_rate: SAMPLE_RATE,
            channels: 2,
            samples_per_packet: SAMPLES_PER_PACKET,
            first_sample_index: SampleIndex::new(first_sample_index),
            host_presentation_time_ms: MonotonicMillis::new(HOST_START_MS + exact_elapsed_ms),
            payload: vec![0; usize::try_from(SAMPLES_PER_PACKET).expect("fits usize") * 4],
        })
        .expect("packet accepted");

    // A rounded 21ms packet duration would incorrectly release sequence 47
    // at +987ms. Exact 1024/48000 geometry keeps it waiting at +1000ms.
    assert!(matches!(
        scheduler.poll(HOST_START_MS + 1_000),
        SchedulerPoll::Waiting { .. }
    ));
    match scheduler.poll(HOST_START_MS + 1_002) {
        SchedulerPoll::Frame { frame, .. } => assert_eq!(frame.sequence, SEQUENCE),
        other => panic!("expected exact-timeline frame, got {other:?}"),
    }
}
