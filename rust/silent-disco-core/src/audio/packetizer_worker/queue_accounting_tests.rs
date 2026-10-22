//! Packetizer queue statistics must distinguish control boundaries from PCM datagrams.
use super::{Packetizer, PacketizerWorkerErrorKind, SharedStatistics, send_with_backpressure};
use crate::audio::{AudioFormat, AudioSampleFormat, DecodedPcmChunk};
use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::{ControlMessage, ProtocolFrame, TrackBoundary};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{TryRecvError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

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
    assert!(matches!(
        receiver.recv().expect("first"),
        ProtocolFrame::Audio(_)
    ));
    assert!(matches!(
        receiver.recv().expect("boundary"),
        ProtocolFrame::Control(ControlMessage::TrackBoundary(_))
    ));
    assert!(matches!(
        receiver.recv().expect("last"),
        ProtocolFrame::Audio(_)
    ));
}

#[test]
fn disconnected_receiver_does_not_count_unsent_audio_or_leak_queue_slots() {
    let session_id = SessionId::new("disconnect-session").expect("session");
    let stream_id = StreamId::new("disconnect-stream").expect("stream");
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
    let audio = packetizer
        .push_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: vec![7; 240],
            end_of_stream: true,
        })
        .expect("audio datagram")
        .frames
        .into_iter()
        .next()
        .expect("one audio datagram");
    let boundary = ProtocolFrame::Control(ControlMessage::TrackBoundary(TrackBoundary {
        session_id,
        stream_id,
        first_sample_index: SampleIndex::new(240),
        entry_id: "next".to_owned(),
        display_name: "Next".to_owned(),
    }));
    for frame in [audio, boundary] {
        let (sender, receiver) = sync_channel(1);
        drop(receiver);
        let statistics = SharedStatistics::default();
        let error = send_with_backpressure(&sender, frame, &AtomicBool::new(false), &statistics, 1)
            .expect_err("disconnected receiver must reject the frame");
        assert_eq!(error.kind, PacketizerWorkerErrorKind::Cancelled);
        assert_eq!(statistics.queued_packets.load(Ordering::Acquire), 0);
        assert_eq!(statistics.emitted_packets.load(Ordering::Acquire), 0);
        assert_eq!(statistics.backpressure_events.load(Ordering::Acquire), 0);
    }
}

#[test]
fn cancellation_while_boundary_waits_on_full_queue_preserves_audio_accounting() {
    let session_id = SessionId::new("backpressure-session").expect("session");
    let stream_id = StreamId::new("backpressure-stream").expect("stream");
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
    let audio = packetizer
        .push_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: vec![7; 240],
            end_of_stream: true,
        })
        .expect("audio datagram")
        .frames
        .into_iter()
        .next()
        .expect("one audio datagram");
    let boundary = ProtocolFrame::Control(ControlMessage::TrackBoundary(TrackBoundary {
        session_id,
        stream_id,
        first_sample_index: SampleIndex::new(240),
        entry_id: "next".to_owned(),
        display_name: "Next".to_owned(),
    }));
    let (sender, receiver) = sync_channel(1);
    let statistics = Arc::new(SharedStatistics::default());
    let cancellation = Arc::new(AtomicBool::new(false));
    send_with_backpressure(&sender, audio, &cancellation, &statistics, 1)
        .expect("first audio fills the only queue slot");

    let worker_statistics = Arc::clone(&statistics);
    let worker_cancellation = Arc::clone(&cancellation);
    let boundary_sender = sender.clone();
    let blocked_sender = thread::spawn(move || {
        send_with_backpressure(
            &boundary_sender,
            boundary,
            &worker_cancellation,
            &worker_statistics,
            1,
        )
    });
    // Observe actual bounded backpressure, not merely thread startup,
    // before cancelling a producer that has committed no boundary frame.
    let start = Instant::now();
    while statistics.backpressure_events.load(Ordering::Acquire) == 0
        && start.elapsed() < Duration::from_secs(5)
    {
        thread::sleep(Duration::from_millis(1));
    }
    cancellation.store(true, Ordering::Release);
    let result = blocked_sender.join().expect("sender thread must exit");
    assert_eq!(
        result
            .expect_err("cancellation must prevent boundary delivery")
            .kind,
        PacketizerWorkerErrorKind::Cancelled
    );
    assert!(
        statistics.backpressure_events.load(Ordering::Acquire) >= 1,
        "the boundary sender must have encountered the full queue"
    );
    assert_eq!(statistics.queued_packets.load(Ordering::Acquire), 1);
    assert_eq!(statistics.emitted_packets.load(Ordering::Acquire), 1);
    assert!(matches!(
        receiver
            .try_recv()
            .expect("only the first audio was queued"),
        ProtocolFrame::Audio(_)
    ));
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
}
