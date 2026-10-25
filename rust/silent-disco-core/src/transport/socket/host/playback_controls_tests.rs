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

    // A delayed copy of the retired start cannot revive a stopped stream.
    assert!(!cached.update(&start));
    assert!(cached.snapshot().is_empty());

    // A new Play may reuse the stream ID with a strictly newer anchor,
    // without restoring the old boundary or pause.
    let ControlMessage::StreamStart(mut restarted) = start else {
        panic!("expected StreamStart");
    };
    restarted.host_start_time_ms = MonotonicMillis::new(3_000);
    let restarted = ControlMessage::StreamStart(restarted);
    assert!(cached.update(&restarted));
    assert!(cached.snapshot() == vec![restarted]);
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

    // Resume advances the presentation anchor; a delayed original start
    // cannot unpause the cached listener state.
    assert!(!cached.update(&new_start));
    let ControlMessage::StreamStart(mut resumed) = new_start else {
        panic!("expected StreamStart");
    };
    resumed.host_start_time_ms = MonotonicMillis::new(1_100);
    let resumed = ControlMessage::StreamStart(resumed);
    assert!(cached.update(&resumed));
    assert!(cached.snapshot() == vec![resumed, new_boundary]);
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

#[test]
fn late_join_replay_bounds_boundary_history_and_deduplicates_the_latest_event() {
    let session_id = SessionId::new("replay-history-session").expect("session ID");
    let stream_id = StreamId::new("replay-history-stream").expect("stream ID");
    let start = stream_start(&session_id, &stream_id);
    let mut cached = PlaybackControls::default();
    cached.update(&start);

    // A long-running session may cross more tracks than a late joiner should
    // replay. Keep the newest 100 boundaries in their original sample order.
    for index in 0..(super::MAX_CACHED_TRACK_BOUNDARIES + 2) {
        let event = ControlMessage::TrackBoundary(TrackBoundary {
            session_id: session_id.clone(),
            stream_id: stream_id.clone(),
            first_sample_index: SampleIndex::new(
                u64::try_from(index + 1).expect("bounded index") * 960,
            ),
            entry_id: format!("upcoming-{index}"),
            display_name: format!("Upcoming track {index}"),
        });
        cached.update(&event);
        if index == super::MAX_CACHED_TRACK_BOUNDARIES + 1 {
            cached.update(&event); // retransmitting the tail is idempotent
        }
    }

    let replay = cached.snapshot();
    assert_eq!(replay.len(), 1 + super::MAX_CACHED_TRACK_BOUNDARIES);
    assert!(replay.first() == Some(&start));
    let boundaries: Vec<_> = replay
        .iter()
        .skip(1)
        .map(|message| match message {
            ControlMessage::TrackBoundary(boundary) => boundary,
            _ => panic!("late-join replay must contain only track boundaries after StreamStart"),
        })
        .collect();
    assert_eq!(
        boundaries.first().expect("oldest retained").entry_id,
        "upcoming-2"
    );
    assert_eq!(
        boundaries.last().expect("newest retained").entry_id,
        "upcoming-101"
    );
    assert!(
        boundaries
            .windows(2)
            .all(|pair| { pair[0].first_sample_index.get() < pair[1].first_sample_index.get() })
    );
}

#[test]
fn late_join_replay_rejects_out_of_order_and_conflicting_boundary_retransmissions() {
    let session_id = SessionId::new("replay-ordered-session").expect("session ID");
    let stream_id = StreamId::new("replay-ordered-stream").expect("stream ID");
    let start = stream_start(&session_id, &stream_id);
    let earlier = boundary(&session_id, &stream_id);
    let later = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        first_sample_index: SampleIndex::new(96_000),
        entry_id: "later".to_owned(),
        display_name: "Later song".to_owned(),
    });
    let conflicting = ControlMessage::TrackBoundary(TrackBoundary {
        session_id,
        stream_id,
        first_sample_index: SampleIndex::new(96_000),
        entry_id: "conflicting".to_owned(),
        display_name: "Conflicting song".to_owned(),
    });
    let mut cached = PlaybackControls::default();
    cached.update(&start);
    cached.update(&earlier);
    cached.update(&later);
    cached.update(&earlier); // retransmitted older boundary
    cached.update(&conflicting); // duplicate sample time with another entry
    cached.update(&later); // exact retransmission of last boundary
    assert!(
        cached.snapshot() == vec![start, earlier, later],
        "a late joiner must receive strictly ordered, nonconflicting boundaries"
    );
}

#[test]
fn late_join_replay_does_not_regress_to_an_older_pause_in_the_same_stream() {
    let session_id = SessionId::new("replay-pause-order-session").expect("session ID");
    let stream_id = StreamId::new("replay-pause-order-stream").expect("stream ID");
    let start = stream_start(&session_id, &stream_id);
    let earlier = pause(&session_id, &stream_id);
    let later = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(2_000),
    });
    let conflicting = ControlMessage::Pause(Pause {
        session_id,
        stream_id,
        host_pause_time_ms: MonotonicMillis::new(2_000),
    });
    let mut cached = PlaybackControls::default();
    cached.update(&start);
    cached.update(&earlier);
    cached.update(&later);
    cached.update(&earlier); // delayed earlier pause
    cached.update(&conflicting); // repeated same-timestamp pause
    assert!(
        cached.snapshot() == vec![start, later],
        "late join must observe the most recent pause time, never a stale replay"
    );
}

#[test]
fn same_stream_resume_does_not_replay_delayed_pauses() {
    let session_id = SessionId::new("replay-resume-pause-session").expect("session ID");
    let stream_id = StreamId::new("replay-resume-pause-stream").expect("stream ID");
    let start = stream_start(&session_id, &stream_id);
    let first_pause = pause(&session_id, &stream_id);
    let older_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_400),
    });
    let newer_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(2_500),
    });
    let next = boundary(&session_id, &stream_id);
    let mut cached = PlaybackControls::default();

    cached.update(&start);
    cached.update(&next);
    cached.update(&first_pause);
    assert!(!cached.update(&start), "duplicate start cannot lift Pause");
    let ControlMessage::StreamStart(mut reanchored) = start.clone() else {
        panic!("expected StreamStart");
    };
    reanchored.host_start_time_ms = MonotonicMillis::new(1_100);
    let resume = ControlMessage::StreamStart(reanchored.clone());
    assert!(cached.update(&resume));
    cached.update(&older_pause); // delayed prior Pause
    cached.update(&first_pause); // duplicated prior Pause at the watermark
    assert!(
        cached.snapshot() == vec![resume.clone(), next.clone()],
        "prior pauses must not reappear in late-join replay after Resume"
    );

    cached.update(&newer_pause); // a genuinely later Pause still takes effect
    assert!(cached.snapshot() == vec![resume.clone(), next.clone(), newer_pause.clone()]);
    assert!(
        !cached.update(&resume),
        "stale resume cannot lift newer Pause"
    );
    reanchored.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume_again = ControlMessage::StreamStart(reanchored);
    assert!(cached.update(&resume_again));
    cached.update(&first_pause);
    cached.update(&newer_pause);
    assert!(cached.snapshot() == vec![resume_again, next]);
}

#[test]
fn replacement_stream_resets_pause_retransmission_fence() {
    let session_id = SessionId::new("replay-new-pause-session").expect("session ID");
    let previous_id = StreamId::new("previous-pause-stream").expect("stream ID");
    let current_id = StreamId::new("new-pause-stream").expect("stream ID");
    let previous_start = stream_start(&session_id, &previous_id);
    let current_start = stream_start(&session_id, &current_id);
    let late_previous_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: previous_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(9_000),
    });
    let current_pause = pause(&session_id, &current_id);
    let mut cached = PlaybackControls::default();
    cached.update(&previous_start);
    cached.update(&late_previous_pause);
    cached.update(&current_start);
    cached.update(&late_previous_pause);
    cached.update(&current_pause);
    assert!(
        cached.snapshot() == vec![current_start, current_pause],
        "new streams must accept their own first Pause even if its timestamp is lower"
    );
}

#[test]
fn restarted_stream_rejects_delayed_old_generation_controls() {
    let session_id = SessionId::new("replay-generation-session").expect("session ID");
    let stream_id = StreamId::new("replay-generation-stream").expect("stream ID");
    let fresh_id = StreamId::new("replay-fresh-stream").expect("new stream ID");
    let old_start = stream_start(&session_id, &stream_id);
    let old_pause = pause(&session_id, &stream_id);
    let old_stop = ControlMessage::Stop(Stop {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_000),
    });
    let mut cached = PlaybackControls::default();

    assert!(cached.update(&old_start));
    assert!(cached.update(&old_stop));
    assert!(cached.snapshot().is_empty());

    // A fresh Play uses a new stream ID. Earlier Stop/Pause/Start
    // retransmissions must never affect this newer playback generation.
    let fresh_start = ControlMessage::StreamStart(StreamStart {
        session_id: session_id.clone(),
        stream_id: fresh_id.clone(),
        host_start_time_ms: MonotonicMillis::new(5_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        current_entry_id: Some("current-again".to_owned()),
        current_display_name: Some("Restarted song".to_owned()),
    });
    let next = boundary(&session_id, &fresh_id);
    assert!(cached.update(&fresh_start));
    assert!(cached.update(&next));
    assert!(!cached.update(&old_pause));
    assert!(!cached.update(&old_stop));
    assert!(!cached.update(&old_start));
    assert!(cached.snapshot() == vec![fresh_start.clone(), next.clone()]);

    let current_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: fresh_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(6_000),
    });
    assert!(cached.update(&current_pause));
    assert!(!cached.update(&old_start));
    assert!(!cached.update(&old_stop));
    assert!(
        cached.snapshot() == vec![fresh_start, next, current_pause],
        "old-generation controls cannot unpause or retire the restarted stream"
    );

    assert!(cached.update(&ControlMessage::Stop(Stop {
        session_id,
        stream_id: fresh_id,
        host_stop_time_ms: MonotonicMillis::new(7_000),
    })));
    assert!(cached.snapshot().is_empty());
}

#[test]
fn older_stream_start_cannot_replace_the_current_reconnect_snapshot() {
    let session_id = SessionId::new("replay-switch-generation-session").expect("session ID");
    let old_id = StreamId::new("replay-switch-old-stream").expect("stream ID");
    let new_id = StreamId::new("replay-switch-new-stream").expect("stream ID");
    let old_start = stream_start(&session_id, &old_id);
    let new_start = ControlMessage::StreamStart(StreamStart {
        session_id: session_id.clone(),
        stream_id: new_id.clone(),
        host_start_time_ms: MonotonicMillis::new(5_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        current_entry_id: Some("new".to_owned()),
        current_display_name: Some("New track".to_owned()),
    });
    let new_boundary = boundary(&session_id, &new_id);
    let paused = ControlMessage::Pause(Pause {
        session_id,
        stream_id: new_id,
        host_pause_time_ms: MonotonicMillis::new(6_000),
    });
    let mut cached = PlaybackControls::default();

    assert!(cached.update(&old_start));
    assert!(cached.update(&new_start));
    assert!(cached.update(&new_boundary));
    assert!(cached.update(&paused));
    assert!(!cached.update(&old_start));
    assert!(
        cached.snapshot() == vec![new_start, new_boundary, paused],
        "a delayed earlier StreamStart must not replace a newer track or unpause it"
    );
}

#[test]
fn pause_and_stop_are_valid_before_the_first_scheduled_audio_sample() {
    let session_id = SessionId::new("replay-future-anchor-session").expect("session ID");
    let stream_id = StreamId::new("replay-future-anchor-stream").expect("stream ID");
    let future_start = ControlMessage::StreamStart(StreamStart {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_start_time_ms: MonotonicMillis::new(10_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("First track".to_owned()),
    });
    // The desktop host advertises a start 400 ms ahead; a user can pause
    // or stop immediately after Play, while every audio sample is still
    // scheduled for the future. Both controls must reach live listeners.
    let early_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(9_650),
    });
    let early_stop = ControlMessage::Stop(Stop {
        session_id,
        stream_id,
        host_stop_time_ms: MonotonicMillis::new(9_700),
    });
    let mut cached = PlaybackControls::default();
    assert!(cached.update(&future_start));
    assert!(
        cached.update(&early_pause),
        "Pause cannot be gated by future audio time"
    );
    assert!(cached.snapshot() == vec![future_start, early_pause]);
    assert!(
        cached.update(&early_stop),
        "Stop must interrupt startup send-ahead"
    );
    assert!(
        cached.snapshot().is_empty(),
        "Stop must clear future playback from late-join replay"
    );
}

#[test]
fn stopped_generation_rejects_delayed_stream_starts_even_with_another_stream_id() {
    let session = SessionId::new("stop-start-fence-session").expect("session");
    let old_id = StreamId::new("stop-start-fence-old").expect("old stream");
    let start = stream_start(&session, &old_id);
    let mut cached = PlaybackControls::default();
    assert!(cached.update(&start));
    assert!(cached.update(&ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: old_id,
        host_stop_time_ms: MonotonicMillis::new(2_000),
    })));

    let ControlMessage::StreamStart(mut delayed) = start.clone() else {
        panic!("expected StreamStart");
    };
    delayed.stream_id = StreamId::new("stop-start-fence-other").expect("other stream");
    assert!(!cached.update(&start));
    assert!(!cached.update(&ControlMessage::StreamStart(delayed.clone())));
    // A delayed Play can have an anchor later than the stopped stream's
    // original anchor yet still precede Stop's actual control timestamp.
    delayed.host_start_time_ms = MonotonicMillis::new(1_500);
    assert!(!cached.update(&ControlMessage::StreamStart(delayed.clone())));
    assert!(cached.snapshot().is_empty());

    // A genuinely newer Play in the same session is accepted; a delayed
    // start from before Stop must not replace its reconnect snapshot.
    delayed.stream_id = StreamId::new("stop-start-fence-new").expect("new stream");
    delayed.host_start_time_ms = MonotonicMillis::new(3_000);
    let fresh = ControlMessage::StreamStart(delayed);
    assert!(cached.update(&fresh));
    assert!(!cached.update(&start));
    assert!(cached.snapshot() == vec![fresh]);
}

#[test]
fn replay_with_reused_stream_id_rejects_retired_pause_and_stop() {
    let session = SessionId::new("reused-stream-control-fence").expect("session");
    let stream = StreamId::new("reused-stream-control-fence").expect("stream");
    let original = stream_start(&session, &stream);
    let old_pause = pause(&session, &stream);
    let old_stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_000),
    });
    let mut cache = PlaybackControls::default();
    assert!(cache.update(&original));
    assert!(cache.update(&old_pause));
    assert!(cache.update(&old_stop));

    let ControlMessage::StreamStart(mut restarted) = original else {
        panic!("expected start");
    };
    restarted.host_start_time_ms = MonotonicMillis::new(3_000);
    let replay = ControlMessage::StreamStart(restarted);
    let next = boundary(&session, &stream);
    assert!(cache.update(&replay));
    assert!(cache.update(&next));
    assert!(!cache.update(&old_pause), "old Pause must not pause replay");
    assert!(!cache.update(&old_stop), "old Stop must not end replay");
    assert!(cache.snapshot() == vec![replay.clone(), next.clone()]);

    let fresh_pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(3_500),
    });
    assert!(cache.update(&fresh_pause));
    assert!(cache.snapshot() == vec![replay, next, fresh_pause]);
    let fresh_stop = ControlMessage::Stop(Stop {
        session_id: session,
        stream_id: stream,
        host_stop_time_ms: MonotonicMillis::new(3_600),
    });
    assert!(cache.update(&fresh_stop));
    assert!(
        !cache.update(&old_stop),
        "retired idle Stop is not broadcast"
    );
    assert!(cache.snapshot().is_empty());
}

#[test]
fn replay_control_fence_allows_pause_before_new_future_audio_anchor() {
    let session = SessionId::new("future-replay-control-fence").expect("session");
    let stream = StreamId::new("future-replay-control-fence").expect("stream");
    let ControlMessage::StreamStart(mut original) = stream_start(&session, &stream) else {
        panic!("expected start");
    };
    original.host_start_time_ms = MonotonicMillis::new(10_000);
    let mut cache = PlaybackControls::default();
    assert!(cache.update(&ControlMessage::StreamStart(original.clone())));
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(9_700),
    });
    assert!(cache.update(&stop));

    original.host_start_time_ms = MonotonicMillis::new(10_300);
    let replay = ControlMessage::StreamStart(original);
    assert!(cache.update(&replay));
    let old_pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(9_650),
    });
    assert!(!cache.update(&old_pause));
    assert!(!cache.update(&stop));
    let early_new_pause = ControlMessage::Pause(Pause {
        session_id: session,
        stream_id: stream,
        host_pause_time_ms: MonotonicMillis::new(9_800),
    });
    assert!(cache.update(&early_new_pause));
    assert!(cache.snapshot() == vec![replay, early_new_pause]);
}
