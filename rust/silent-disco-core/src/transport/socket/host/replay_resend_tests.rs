//! A replayed stream's re-sent opening (device matrix, 2026-11-20).
//!
//! The desktop host sends each packet of a stream's first 200 ms twice. After
//! a Stop, the next stream on the session is fenced to strictly contiguous
//! packets, which dropped every copy: on the A54 only a session's first stream
//! had its opening covered. The fence now also admits an exact copy of a
//! packet this generation already sent, and nothing else it used to refuse.

use crate::protocol::{AudioDatagram, ProtocolFrame};
use crate::transport::test_support::{audio_frame_at, id_session, stream_start, stream_stop};

use super::{PlaybackControls, REPLAY_RESEND_WINDOW_PACKETS};

fn packet(sequence: u64, presentation_ms: u64) -> AudioDatagram {
    match audio_frame_at(&id_session("replay-resend"), sequence, presentation_ms) {
        ProtocolFrame::Audio(packet) => packet,
        _ => unreachable!("audio_frame_at always constructs audio"),
    }
}

/// A first stream played and stopped, then a second stream started at 1 000.
fn replaying() -> PlaybackControls {
    let session = id_session("replay-resend");
    let mut controls = PlaybackControls::default();
    assert!(controls.update(&stream_start(&session, 100)));
    for sequence in 0..30 {
        assert!(controls.permits_audio(&packet(sequence, 100 + sequence)));
    }
    assert!(controls.update(&stream_stop(&session, 200)));
    assert!(controls.update(&stream_start(&session, 1_000)));
    controls
}

#[test]
fn a_replayed_stream_admits_a_copy_of_a_packet_it_already_sent() {
    let mut controls = replaying();
    for sequence in 0..=20 {
        assert!(controls.permits_audio(&packet(sequence, 1_000 + sequence)));
    }
    // The copies the desktop sends 20 packets later.
    for sequence in 0..=5 {
        assert!(
            controls.permits_audio(&packet(sequence, 1_000 + sequence)),
            "copy of packet {sequence}"
        );
    }
    // Copies do not move the contiguity fence.
    assert!(controls.permits_audio(&packet(21, 1_021)));
    assert!(
        !controls.permits_audio(&packet(23, 1_023)),
        "a gap is still refused"
    );
}

#[test]
fn the_fence_still_refuses_what_is_not_an_exact_copy() {
    let mut controls = replaying();
    for sequence in 0..=20 {
        assert!(controls.permits_audio(&packet(sequence, 1_000 + sequence)));
    }
    assert!(
        !controls.permits_audio(&packet(3, 103)),
        "a late packet from the stopped stream"
    );
    assert!(
        !controls.permits_audio(&packet(3, 1_500)),
        "a sent sequence on another presentation time"
    );
    assert!(
        !controls.permits_audio(&packet(25, 1_025)),
        "a packet this stream never sent"
    );
}

#[test]
fn only_recently_sent_packets_are_remembered() {
    let mut controls = replaying();
    let sent = REPLAY_RESEND_WINDOW_PACKETS as u64 + 10;
    for sequence in 0..sent {
        assert!(controls.permits_audio(&packet(sequence, 1_000 + sequence)));
    }
    assert!(
        !controls.permits_audio(&packet(0, 1_000)),
        "too old to be a re-send"
    );
    let recent = sent - 1;
    assert!(controls.permits_audio(&packet(recent, 1_000 + recent)));
}

#[test]
fn a_new_stop_forgets_the_previous_generation_packets() {
    let session = id_session("replay-resend");
    let mut controls = replaying();
    for sequence in 0..=20 {
        assert!(controls.permits_audio(&packet(sequence, 1_000 + sequence)));
    }
    assert!(controls.update(&stream_stop(&session, 2_000)));
    assert!(controls.update(&stream_start(&session, 3_000)));
    assert!(
        !controls.permits_audio(&packet(5, 1_005)),
        "a copy from the previous generation"
    );
    assert!(controls.permits_audio(&packet(0, 3_000)));
}
