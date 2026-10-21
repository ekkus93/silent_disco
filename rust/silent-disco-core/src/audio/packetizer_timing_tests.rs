//! Regression coverage for packetizer deadlines at integer packet sizes.
use super::packetizer::Packetizer;
use super::{AudioFormat, AudioSampleFormat, DecodedPcmChunk};
use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::ProtocolFrame;

fn packetize(sample_rate_hz: u32, frames: usize) -> Vec<ProtocolFrame> {
    let format = AudioFormat {
        sample_rate_hz,
        channels: 1,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        SessionId::new("packet-time-session").expect("session"),
        StreamId::new("packet-time-stream").expect("stream"),
        format,
        5,
        MonotonicMillis::new(10_000),
    )
    .expect("valid packetizer");
    packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: vec![1; frames],
            end_of_stream: true,
        })
        .expect("valid decoded chunk")
        .frames
}

#[test]
fn non_integral_44100_hz_packets_follow_sample_clock_without_accumulated_drift() {
    let frames = packetize(44_100, 88_200);
    assert_eq!(frames.len(), 401);
    for (sequence, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(audio) = frame else {
            panic!("packetization emits only audio");
        };
        let first_sample = u64::try_from(sequence).expect("sequence") * 220;
        let exact_rounded_ms = (first_sample * 1_000 + 22_050) / 44_100;
        assert_eq!(
            audio.sequence.get(),
            u64::try_from(sequence).expect("sequence")
        );
        assert_eq!(audio.samples_per_packet, 220);
        assert_eq!(audio.first_sample_index.get(), first_sample);
        assert_eq!(
            audio.host_presentation_time_ms.get(),
            10_000 + exact_rounded_ms,
            "packet {sequence} must use the sample clock, not nominal 5ms ticks"
        );
    }
    let ProtocolFrame::Audio(last) = &frames[400] else {
        panic!("last frame is audio");
    };
    assert_eq!(last.host_presentation_time_ms.get(), 11_995);
}

#[test]
fn canonical_48000_hz_keeps_exact_five_millisecond_packet_deadlines() {
    let frames = packetize(48_000, 48_000);
    assert_eq!(frames.len(), 200);
    for (sequence, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(audio) = frame else {
            panic!("packetization emits only audio");
        };
        assert_eq!(audio.samples_per_packet, 240);
        assert_eq!(audio.first_sample_index.get(), sequence as u64 * 240);
        assert_eq!(
            audio.host_presentation_time_ms.get(),
            10_000 + sequence as u64 * 5
        );
    }
}

#[test]
fn nonzero_source_sample_index_preserves_initial_host_deadline() {
    let format = AudioFormat {
        sample_rate_hz: 1_000,
        channels: 1,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        SessionId::new("indexed-start-session").expect("session"),
        StreamId::new("indexed-start-stream").expect("stream"),
        format,
        20,
        MonotonicMillis::new(9_000),
    )
    .expect("packetizer");
    let frames = packetizer
        .push_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(400),
            frames: vec![1; 40],
            end_of_stream: true,
        })
        .expect("packetize nonzero indexed source")
        .frames;
    assert_eq!(frames.len(), 2);
    for (i, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(audio) = frame else {
            panic!("audio frame");
        };
        assert_eq!(audio.first_sample_index.get(), 400 + i as u64 * 20);
        assert_eq!(audio.host_presentation_time_ms.get(), 9_000 + i as u64 * 20);
    }
}

#[test]
fn oversized_audio_geometry_is_rejected_without_arithmetic_overflow() {
    // The sample and channel counts fit their protocol fields but their
    // product exceeds 32-bit usize. Never panic before rejecting this frame.
    let format = AudioFormat {
        sample_rate_hz: u32::MAX,
        channels: u16::MAX,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let error = Packetizer::new(
        SessionId::new("oversized-geometry-session").expect("session"),
        StreamId::new("oversized-geometry-stream").expect("stream"),
        format,
        1_000,
        MonotonicMillis::new(0),
    )
    .expect_err("oversized PCM datagram must be rejected");
    assert_eq!(error.kind, super::PacketizerErrorKind::InvalidConfiguration);
}
