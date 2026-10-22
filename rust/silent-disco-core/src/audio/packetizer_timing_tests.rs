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

#[test]
fn three_non_aligned_44100_hz_tracks_preserve_pcm_and_sample_clock() {
    // All three decoders report source-local sample zero. Neither boundary
    // aligns with a 220-frame packet, and the final packet needs padding.
    // This catches premature carry flushing, inserted silence, duplicate
    // samples, and nominal-5ms timestamp drift across multiple tracks.
    let format = AudioFormat {
        sample_rate_hz: 44_100,
        channels: 1,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        SessionId::new("three-track-timing-session").expect("session"),
        StreamId::new("three-track-timing-stream").expect("stream"),
        format,
        5,
        MonotonicMillis::new(10_000),
    )
    .expect("valid packetizer");
    let lengths = [219_usize, 44_101, 44_101];
    let values = [101_i16, 202, 303];
    let mut frames = Vec::new();
    let mut boundary_indices = Vec::new();

    for (index, (&length, &value)) in lengths.iter().zip(values.iter()).enumerate() {
        if index > 0 {
            boundary_indices.push(packetizer.next_contiguous_sample_index());
        }
        let outcome = packetizer
            .push_contiguous_chunk(DecodedPcmChunk {
                format,
                first_sample_index: SampleIndex::new(0),
                frames: vec![value; length],
                end_of_stream: index == lengths.len() - 1,
            })
            .expect("contiguous track");
        assert_eq!(outcome.end_of_stream, index == lengths.len() - 1);
        frames.extend(outcome.frames);
    }

    assert_eq!(boundary_indices, [219, 44_320]);
    assert_eq!(packetizer.next_contiguous_sample_index(), 88_421);
    assert_eq!(frames.len(), 402);
    for (sequence, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(audio) = frame else {
            panic!("only audio packets expected");
        };
        let first = sequence as u64 * 220;
        let rounded_ms = (first * 1_000 + 22_050) / 44_100;
        assert_eq!(audio.sequence.get(), sequence as u64);
        assert_eq!(audio.first_sample_index.get(), first);
        assert_eq!(audio.host_presentation_time_ms.get(), 10_000 + rounded_ms);
        assert_eq!(audio.payload.len(), 440);

        for (offset, sample) in audio.payload.chunks_exact(2).enumerate() {
            let absolute = usize::try_from(first).expect("fixture index fits usize") + offset;
            let expected = if absolute < 219 {
                values[0]
            } else if absolute < 44_320 {
                values[1]
            } else if absolute < 88_421 {
                values[2]
            } else {
                0 // Only the last packet may contain end-of-queue padding.
            };
            assert_eq!(
                i16::from_le_bytes([sample[0], sample[1]]),
                expected,
                "unexpected PCM at stream sample {absolute}"
            );
        }
    }
}

#[test]
fn stereo_split_tracks_and_empty_intermediate_entry_keep_one_pcm_timeline() {
    // A 241-frame track crosses a packet boundary; an empty track must not
    // flush the carry. The final track arrives in three arbitrarily sized
    // decoder chunks, and only the final packet may contain silence padding.
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        SessionId::new("split-stereo-session").expect("session"),
        StreamId::new("split-stereo-stream").expect("stream"),
        format,
        5,
        MonotonicMillis::new(10_000),
    )
    .expect("valid packetizer");

    let chunks = [
        (100_usize, 11_i16, false),
        (141, 11, false),
        (0, 22, false), // Empty middle track, with its own entry identity.
        (1, 33, false),
        (239, 33, false),
        (2, 33, true),
    ];
    let mut frames = Vec::new();
    let mut boundaries = Vec::new();
    for (index, (frame_count, value, end_of_stream)) in chunks.into_iter().enumerate() {
        if index == 2 || index == 3 {
            boundaries.push(packetizer.next_contiguous_sample_index());
        }
        let pcm = (0..frame_count)
            .flat_map(|_| [value, -value])
            .collect();
        let outcome = packetizer
            .push_contiguous_chunk(DecodedPcmChunk {
                format,
                first_sample_index: SampleIndex::new(0),
                frames: pcm,
                end_of_stream,
            })
            .expect("adjacent decoded chunk");
        assert_eq!(outcome.end_of_stream, end_of_stream);
        frames.extend(outcome.frames);
    }

    assert_eq!(boundaries, [241, 241]);
    assert_eq!(packetizer.next_contiguous_sample_index(), 483);
    assert!(packetizer.is_finished());
    assert_eq!(frames.len(), 3);
    for (sequence, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(audio) = frame else {
            panic!("only audio packets expected");
        };
        assert_eq!(audio.sequence.get(), sequence as u64);
        assert_eq!(audio.first_sample_index.get(), sequence as u64 * 240);
        assert_eq!(audio.host_presentation_time_ms.get(), 10_000 + sequence as u64 * 5);
        assert_eq!(audio.samples_per_packet, 240);
        assert_eq!(audio.payload.len(), 960);
        for (offset, stereo) in audio.payload.chunks_exact(4).enumerate() {
            let absolute = sequence * 240 + offset;
            let expected: i16 = if absolute < 241 {
                11
            } else if absolute < 483 {
                33
            } else {
                0
            };
            assert_eq!(
                i16::from_le_bytes([stereo[0], stereo[1]]),
                expected,
                "left sample {absolute}"
            );
            assert_eq!(
                i16::from_le_bytes([stereo[2], stereo[3]]),
                -expected,
                "right sample {absolute}"
            );
        }
    }
}

#[test]
fn empty_final_track_flushes_only_the_prior_tracks_unpadded_carry() {
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 1,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        SessionId::new("empty-final-session").expect("session"),
        StreamId::new("empty-final-stream").expect("stream"),
        format,
        5,
        MonotonicMillis::new(1_000),
    )
    .expect("valid packetizer");
    let first = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: vec![7; 241],
            end_of_stream: false,
        })
        .expect("first track");
    assert_eq!(first.frames.len(), 1);
    assert!(!first.end_of_stream);
    let final_track = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: Vec::new(),
            end_of_stream: true,
        })
        .expect("empty final track");
    assert!(final_track.end_of_stream);
    assert_eq!(packetizer.next_contiguous_sample_index(), 241);
    assert_eq!(final_track.frames.len(), 1);
    let ProtocolFrame::Audio(last) = &final_track.frames[0] else {
        panic!("last packet is audio");
    };
    assert_eq!(last.sequence.get(), 1);
    assert_eq!(last.first_sample_index.get(), 240);
    assert_eq!(last.host_presentation_time_ms.get(), 1_005);
    assert_eq!(&last.payload[..2], &7_i16.to_le_bytes());
    assert!(last.payload[2..].iter().all(|byte| *byte == 0));
}
