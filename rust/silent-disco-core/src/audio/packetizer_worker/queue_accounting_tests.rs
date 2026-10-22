//! Packetizer queue statistics must distinguish control boundaries from PCM datagrams.
use super::{Packetizer, SharedStatistics, send_with_backpressure};
use crate::audio::{AudioFormat, AudioSampleFormat, DecodedPcmChunk};
use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::{ControlMessage, ProtocolFrame, TrackBoundary};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;

#[test]
fn reserved_track_boundary_does_not_inflate_emitted_audio_packet_count() {
    let session_id = SessionId::new("accounting-session").expect("session");
    let stream_id = StreamId::new("accounting-stream").expect("stream");
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 1,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        session_id.clone(),
        stream_id.clone(),
        format,
        5,
        MonotonicMillis::new(1_000),
    )
    .expect("packetizer");
    let mut audio = packetizer
        .push_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: vec![7; 480],
            end_of_stream: true,
        })
        .expect("two audio datagrams")
        .frames;
    assert_eq!(audio.len(), 2);
    let boundary = ProtocolFrame::Control(ControlMessage::TrackBoundary(TrackBoundary {
        session_id,
        stream_id,
        first_sample_index: SampleIndex::new(240),
        entry_id: "next".to_owned(),
        display_name: "Next".to_owned(),
    }));
    let (sender, receiver) = sync_channel(4);
    let statistics = SharedStatistics::default();
    let cancellation = AtomicBool::new(false);
    send_with_backpressure(&sender, audio.remove(0), &cancellation, &statistics, 4)
        .expect("first audio");
    send_with_backpressure(&sender, boundary, &cancellation, &statistics, 4)
        .expect("track boundary");
    send_with_backpressure(&sender, audio.remove(0), &cancellation, &statistics, 4)
        .expect("successor audio");

    assert_eq!(statistics.queued_packets.load(Ordering::Acquire), 3);
    assert_eq!(statistics.emitted_packets.load(Ordering::Acquire), 2);
    assert_eq!(statistics.backpressure_events.load(Ordering::Acquire), 0);
    assert!(matches!(receiver.recv().expect("first"), ProtocolFrame::Audio(_)));
    assert!(matches!(
        receiver.recv().expect("boundary"),
        ProtocolFrame::Control(ControlMessage::TrackBoundary(_))
    ));
    assert!(matches!(receiver.recv().expect("last"), ProtocolFrame::Audio(_)));
}
