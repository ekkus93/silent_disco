//! Same-stream reconnect/resume must never renegotiate the active PCM format.
use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::{ControlMessage, Pause, StreamStart, TrackBoundary};

use super::PlaybackControls;

#[test]
fn malformed_same_stream_resume_preserves_paused_reconnect_snapshot() {
    let session = SessionId::new("resume-format-session").expect("session");
    let stream = StreamId::new("resume-format-stream").expect("stream");
    let start = StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("Original title".to_owned()),
    };
    let first = ControlMessage::StreamStart(start.clone());
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second".to_owned(),
        display_name: "Second title".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session,
        stream_id: stream,
        host_pause_time_ms: MonotonicMillis::new(1_500),
    });
    let mut cache = PlaybackControls::default();
    assert!(cache.update(&first));
    assert!(cache.update(&boundary));
    assert!(cache.update(&pause));
    let held = vec![first.clone(), boundary.clone(), pause];

    for (rate, channels, packet_frames) in [(44_100, 2, 240), (48_000, 1, 240), (48_000, 2, 960)] {
        let mut invalid = start.clone();
        invalid.sample_rate = rate;
        invalid.channels = channels;
        invalid.samples_per_packet = packet_frames;
        invalid.current_display_name = Some("Stale format".to_owned());
        assert!(!cache.update(&ControlMessage::StreamStart(invalid)));
        assert!(
            cache.snapshot() == held,
            "rejected resume must not unpause or change replay"
        );
    }

    // Same-format metadata refresh remains valid, retaining the boundary
    // while withdrawing Pause for the authorized reconnect snapshot.
    let mut refreshed = start;
    refreshed.host_start_time_ms = MonotonicMillis::new(1_100);
    refreshed.current_display_name = Some("Refreshed title".to_owned());
    let resume = ControlMessage::StreamStart(refreshed);
    assert!(cache.update(&resume));
    assert!(cache.snapshot() == vec![resume, boundary]);
}

#[test]
fn delayed_same_stream_start_does_not_cancel_pause_or_overwrite_boundary_replay() {
    let session = SessionId::new("delayed-resume-session").expect("session");
    let stream = StreamId::new("delayed-resume-stream").expect("stream");
    let start = StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("First track".to_owned()),
    };
    let original = ControlMessage::StreamStart(start.clone());
    let next = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second".to_owned(),
        display_name: "Second track".to_owned(),
    });
    let mut cache = PlaybackControls::default();
    assert!(cache.update(&original));
    assert!(cache.update(&next));
    let paused = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_500),
    });
    assert!(cache.update(&paused));
    let expected_paused = vec![original.clone(), next.clone(), paused.clone()];

    assert!(!cache.update(&original), "retransmitted original start is not Resume");
    let mut duplicate_metadata = start.clone();
    duplicate_metadata.current_display_name = Some("Unverified title".to_owned());
    assert!(!cache.update(&ControlMessage::StreamStart(duplicate_metadata)));
    assert_eq!(cache.snapshot(), expected_paused);

    let mut reanchored = start;
    reanchored.host_start_time_ms = MonotonicMillis::new(1_100);
    reanchored.current_display_name = Some("Confirmed title".to_owned());
    let resumed = ControlMessage::StreamStart(reanchored.clone());
    assert!(cache.update(&resumed));
    assert_eq!(cache.snapshot(), vec![resumed.clone(), next.clone()]);
    assert!(!cache.update(&original), "old start cannot rewind reanchored stream");

    let paused_again = ControlMessage::Pause(Pause {
        session_id: session,
        stream_id: stream,
        host_pause_time_ms: MonotonicMillis::new(2_500),
    });
    assert!(cache.update(&paused_again));
    assert!(!cache.update(&resumed), "old Resume cannot clear a later Pause");
    assert_eq!(cache.snapshot(), vec![resumed, next.clone(), paused_again]);

    reanchored.host_start_time_ms = MonotonicMillis::new(1_200);
    let next_resume = ControlMessage::StreamStart(reanchored);
    assert!(cache.update(&next_resume));
    assert_eq!(cache.snapshot(), vec![next_resume, next]);
}
