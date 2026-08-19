use super::{
    JitterBuffer, JitterBufferConfig, JitterBufferConfigErrorKind, JitterBufferRejectionKind,
    MAX_BUFFERED_DURATION_LIMIT_MS, MAX_REORDER_WINDOW_LIMIT,
};
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram};

fn session() -> SessionId {
    SessionId::new("session-jitter").expect("session id")
}

fn stream() -> StreamId {
    StreamId::new("stream-jitter").expect("stream id")
}

fn datagram(
    session_id: SessionId,
    stream_id: StreamId,
    sequence: u64,
    time_ms: u64,
) -> AudioDatagram {
    AudioDatagram {
        session_id,
        stream_id,
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        first_sample_index: SampleIndex::new(sequence * 960),
        host_presentation_time_ms: MonotonicMillis::new(time_ms),
        payload: vec![0; 960 * 2 * 2],
    }
}

fn packet(sequence: u64, time_ms: u64) -> AudioDatagram {
    datagram(session(), stream(), sequence, time_ms)
}

fn buffer() -> JitterBuffer {
    JitterBuffer::new(JitterBufferConfig::new(session(), stream())).expect("valid config")
}

#[test]
fn emits_out_of_order_arrivals_in_sequence_order() {
    let mut buffer = buffer();
    buffer.accept(packet(2, 40)).expect("accepted");
    buffer.accept(packet(0, 0)).expect("accepted");
    buffer.accept(packet(1, 20)).expect("accepted");

    assert_eq!(buffer.pop_in_order().expect("seq 0").sequence.get(), 0);
    assert_eq!(buffer.pop_in_order().expect("seq 1").sequence.get(), 1);
    assert_eq!(buffer.pop_in_order().expect("seq 2").sequence.get(), 2);
    assert!(buffer.is_empty());
    assert_eq!(buffer.statistics().accepted, 3);
    assert_eq!(buffer.statistics().emitted, 3);
}

#[test]
fn pop_in_order_returns_none_while_the_next_sequence_is_missing() {
    let mut buffer = buffer();
    buffer.accept(packet(1, 20)).expect("accepted");

    assert!(buffer.pop_in_order().is_none());
    assert_eq!(buffer.missing_sequence_count(), 1);
    assert_eq!(buffer.next_expected_sequence(), 0);
}

#[test]
fn rejects_a_duplicate_of_an_already_buffered_sequence() {
    let mut buffer = buffer();
    buffer.accept(packet(0, 0)).expect("accepted");

    let error = buffer
        .accept(packet(0, 0))
        .expect_err("duplicate must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::Duplicate);
    assert_eq!(buffer.statistics().duplicate_rejections, 1);
    assert_eq!(buffer.len(), 1);
}

#[test]
fn rejects_a_sequence_already_emitted_as_too_late() {
    let mut buffer = buffer();
    buffer.accept(packet(0, 0)).expect("accepted");
    assert!(buffer.pop_in_order().is_some());

    let error = buffer
        .accept(packet(0, 0))
        .expect_err("late arrival must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::AlreadyEmitted);
    assert_eq!(buffer.statistics().late_rejections, 1);
}

#[test]
fn rejects_a_sequence_beyond_the_reorder_window() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_reorder_window = 4;
    let mut buffer = JitterBuffer::new(config).expect("valid config");

    let error = buffer
        .accept(packet(5, 100))
        .expect_err("too-far-future sequence must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::ReorderWindowExceeded);
    assert_eq!(buffer.statistics().reorder_window_rejections, 1);
    assert!(buffer.is_empty());
}

#[test]
fn accepts_exactly_at_the_reorder_window_boundary() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_reorder_window = 4;
    let mut buffer = JitterBuffer::new(config).expect("valid config");

    buffer
        .accept(packet(4, 80))
        .expect("sequence exactly at the window boundary must be accepted");
    assert_eq!(buffer.len(), 1);
}

#[test]
fn rejects_a_packet_that_would_exceed_the_buffered_duration() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_buffered_duration_ms = 100;
    let mut buffer = JitterBuffer::new(config).expect("valid config");

    buffer.accept(packet(0, 0)).expect("accepted");
    let error = buffer
        .accept(packet(10, 500))
        .expect_err("packet spanning past the buffered duration must be rejected");
    assert_eq!(
        error.kind,
        JitterBufferRejectionKind::BufferedDurationExceeded
    );
    assert_eq!(buffer.statistics().buffered_duration_rejections, 1);
    assert_eq!(buffer.len(), 1);
}

#[test]
fn rejects_a_packet_from_a_different_session() {
    let mut buffer = buffer();
    let wrong_session = SessionId::new("session-other").expect("session id");

    let error = buffer
        .accept(datagram(wrong_session, stream(), 0, 0))
        .expect_err("wrong session must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::WrongSession);
    assert_eq!(buffer.statistics().wrong_session_rejections, 1);
}

#[test]
fn rejects_a_packet_from_a_stale_or_unrelated_stream() {
    let mut buffer = buffer();
    let stale_stream = StreamId::new("stream-previous-generation").expect("stream id");

    let error = buffer
        .accept(datagram(session(), stale_stream, 0, 0))
        .expect_err("stale stream must be rejected");
    assert_eq!(error.kind, JitterBufferRejectionKind::WrongStream);
    assert_eq!(buffer.statistics().wrong_stream_rejections, 1);
}

#[test]
fn missing_sequence_count_reflects_the_current_gap_and_closes_once_filled() {
    let mut buffer = buffer();
    buffer.accept(packet(3, 60)).expect("accepted");
    assert_eq!(buffer.missing_sequence_count(), 3);

    buffer.accept(packet(0, 0)).expect("accepted");
    assert_eq!(buffer.pop_in_order().expect("seq 0").sequence.get(), 0);
    assert_eq!(buffer.missing_sequence_count(), 2);
}

#[test]
fn skip_expected_sequence_advances_past_a_missing_packet_without_emitting_one() {
    let mut buffer = buffer();
    buffer.accept(packet(1, 20)).expect("accepted");

    assert!(buffer.pop_in_order().is_none());
    buffer.skip_expected_sequence();
    assert_eq!(buffer.next_expected_sequence(), 1);
    assert_eq!(buffer.pop_in_order().expect("seq 1").sequence.get(), 1);
}

#[test]
fn discard_in_order_advances_without_falsely_counting_a_packet_as_emitted() {
    let mut buffer = buffer();
    buffer.accept(packet(0, 0)).expect("accepted");
    buffer.accept(packet(1, 20)).expect("accepted");

    assert!(buffer.discard_in_order());
    assert_eq!(buffer.next_expected_sequence(), 1);
    assert_eq!(buffer.statistics().emitted, 0);
    assert_eq!(buffer.statistics().skipped, 1);
    assert_eq!(buffer.pop_in_order().expect("seq 1").sequence.get(), 1);
    assert_eq!(buffer.statistics().emitted, 1);
}

#[test]
fn buffered_span_ms_reflects_the_earliest_and_latest_buffered_presentation_times() {
    let mut buffer = buffer();
    assert_eq!(buffer.buffered_span_ms(), 0);

    buffer.accept(packet(0, 100)).expect("accepted");
    assert_eq!(buffer.buffered_span_ms(), 0);

    buffer.accept(packet(1, 120)).expect("accepted");
    buffer.accept(packet(2, 140)).expect("accepted");
    assert_eq!(buffer.buffered_span_ms(), 40);
}

#[test]
fn rejects_configuration_with_an_oversized_reorder_window() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_reorder_window = MAX_REORDER_WINDOW_LIMIT + 1;

    let error = JitterBuffer::new(config).expect_err("oversized reorder window must be rejected");
    assert_eq!(
        error.kind,
        JitterBufferConfigErrorKind::ReorderWindowTooLarge
    );
}

#[test]
fn rejects_configuration_with_zero_buffered_duration() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_buffered_duration_ms = 0;

    let error = JitterBuffer::new(config).expect_err("zero buffered duration must be rejected");
    assert_eq!(
        error.kind,
        JitterBufferConfigErrorKind::BufferedDurationOutOfRange
    );
}

#[test]
fn rejects_configuration_with_an_oversized_buffered_duration() {
    let mut config = JitterBufferConfig::new(session(), stream());
    config.max_buffered_duration_ms = MAX_BUFFERED_DURATION_LIMIT_MS + 1;

    let error =
        JitterBuffer::new(config).expect_err("oversized buffered duration must be rejected");
    assert_eq!(
        error.kind,
        JitterBufferConfigErrorKind::BufferedDurationOutOfRange
    );
}
