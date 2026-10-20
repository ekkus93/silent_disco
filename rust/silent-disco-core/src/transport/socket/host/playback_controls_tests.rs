//! Regression locks for the control-state snapshot replayed to an authorized late joiner.
//! Stale boundaries and pauses must never cross a stopped or replaced stream.
use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::{ControlMessage, Pause, Stop, StreamStart, TrackBoundary};

use super::PlaybackControls;

fn stream_start(session_id: &SessionId, stream_id: &StreamId) -> ControlMessage {
    ControlMessage::StreamStart(StreamStart {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        current_entry_id: Some("current".to_owned()),
        current_display_name: Some("Current song".to_owned()),
    })
}

fn boundary(session_id: &SessionId, stream_id: &StreamId) -> ControlMessage {
    ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "next".to_owned(),
        display_name: "Next song".to_owned(),
    })
}

fn pause(session_id: &SessionId, stream_id: &StreamId) -> ControlMessage {
    ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_500),
    })
}

#[test]
fn stop_clears_cached_boundaries_and_pause_before_late_join_or_replay() {
    let session_id = SessionId::new("replay-stop-session").expect("session ID");
    let stream_id = StreamId::new("replay-stop-stream").expect("stream ID");
    let start = stream_start(&session_id, &stream_id);
    let next = boundary(&session_id, &stream_id);
    let held = pause(&session_id, &stream_id);
    let mut cached = PlaybackControls::default();
    for message in [&start, &next, &held] {
        cached.update(message);
    }
    assert!(cached.snapshot() == vec![start.clone(), next.clone(), held]);

    cached.update(&ControlMessage::Stop(Stop {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_000),
    }));
    assert!(
        cached.snapshot().is_empty(),
        "a listener admitted after Stop must not replay a retired stream"
    );

    // A delayed boundary from the old stream cannot revive a retired cache.
    cached.update(&next);
    assert!(cached.snapshot().is_empty());

    // An explicit new Play/StreamStart may reuse the stream ID, but the
    // previously cached boundary and pause must still be absent.
    cached.update(&start);
    assert!(cached.snapshot() == vec![start]);
}

#[test]
fn new_stream_drops_old_boundaries_and_rejects_stale_control_events() {
    let session_id = SessionId::new("replay-switch-session").expect("session ID");
    let old_id = StreamId::new("replay-old-stream").expect("old stream ID");
    let new_id = StreamId::new("replay-new-stream").expect("new stream ID");
    let old_start = stream_start(&session_id, &old_id);
    let old_boundary = boundary(&session_id, &old_id);
    let new_start = stream_start(&session_id, &new_id);
    let new_boundary = boundary(&session_id, &new_id);
    let mut cached = PlaybackControls::default();

    cached.update(&old_start);
    cached.update(&old_boundary);
    cached.update(&new_start);
    cached.update(&old_boundary);
    cached.update(&pause(&session_id, &old_id));
    cached.update(&ControlMessage::Stop(Stop {
        session_id: session_id.clone(),
        stream_id: old_id,
        host_stop_time_ms: MonotonicMillis::new(2_000),
    }));
    assert!(
        cached.snapshot() == vec![new_start.clone()],
        "late controls from a retired stream cannot change new-stream replay"
    );

    cached.update(&new_boundary);
    cached.update(&pause(&session_id, &new_id));
    assert_eq!(cached.snapshot().len(), 3);

    // A same-stream StreamStart is a resume: retain queued boundaries but
    // withdraw paused state before replaying to a newly admitted listener.
    cached.update(&new_start);
    assert!(cached.snapshot() == vec![new_start, new_boundary]);
}

#[test]
fn same_stream_id_in_new_session_rejects_stale_controls_and_drops_old_cache() {
    let old_session = SessionId::new("replay-old-session").expect("old session ID");
    let new_session = SessionId::new("replay-new-session").expect("new session ID");
    let reused_stream = StreamId::new("replay-reused-stream").expect("stream ID");
    let old_start = stream_start(&old_session, &reused_stream);
    let old_boundary = boundary(&old_session, &reused_stream);
    let old_pause = pause(&old_session, &reused_stream);
    let new_start = stream_start(&new_session, &reused_stream);
    let new_boundary = boundary(&new_session, &reused_stream);
    let new_pause = pause(&new_session, &reused_stream);
    let mut cached = PlaybackControls::default();

    cached.update(&old_start);
    cached.update(&old_boundary);
    cached.update(&old_pause);
    cached.update(&new_start);
    assert!(
        cached.snapshot() == vec![new_start.clone()],
        "a reused stream ID must not carry boundaries or pause into a new session"
    );

    cached.update(&old_boundary);
    cached.update(&old_pause);
    cached.update(&ControlMessage::Stop(Stop {
        session_id: old_session,
        stream_id: reused_stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_000),
    }));
    assert!(
        cached.snapshot() == vec![new_start.clone()],
        "delayed old-session controls must not mutate the new session"
    );

    cached.update(&new_boundary);
    cached.update(&new_pause);
    assert!(cached.snapshot() == vec![new_start.clone(), new_boundary, new_pause]);

    cached.update(&ControlMessage::Stop(Stop {
        session_id: new_session,
        stream_id: reused_stream,
        host_stop_time_ms: MonotonicMillis::new(3_000),
    }));
    assert!(
        cached.snapshot().is_empty(),
        "the active session's Stop must clear replay"
    );
}
