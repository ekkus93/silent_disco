//! A session's clock lock before its first stream (startup-counter
//! investigation, 2026-11-19): the first stream opened from a locked
//! [`ListenerSyncSession`] starts locked and plays its first packet, where it
//! used to drop the opening while sync was acquired after `StreamStart`.

use super::ffi_handle::{FfiListenerPlaybackHandle, FfiListenerSyncSession};
use super::ffi_types::FfiListenerPlaybackConfig;
use super::runtime::ListenerPlaybackRuntime;
use super::sync_session::ListenerSyncSession;
use crate::audio_abi::registry_test_guard;
use silent_disco_core::audio::{PlaybackPumpConfig, RenderRingConfig, SchedulerConfig};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};

const SAMPLES_PER_PACKET: u32 = 960;
const HOST_START_MS: u64 = 491_000;

fn scheduler_config() -> SchedulerConfig {
    let mut config = SchedulerConfig::new(
        SessionId::new("session-first").expect("session id"),
        StreamId::new("stream-first").expect("stream id"),
        48_000,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    config.startup_buffer_target_ms = 0;
    config
}

fn ring_config() -> RenderRingConfig {
    RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    }
}

fn first_packet() -> AudioDatagram {
    AudioDatagram {
        session_id: SessionId::new("session-first").expect("session id"),
        stream_id: StreamId::new("stream-first").expect("stream id"),
        sequence: PacketSequence::new(0),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: SAMPLES_PER_PACKET,
        first_sample_index: SampleIndex::new(0),
        host_presentation_time_ms: MonotonicMillis::new(HOST_START_MS),
        payload: (0..SAMPLES_PER_PACKET * 2)
            .flat_map(|_| 1_000_i16.to_le_bytes())
            .collect(),
    }
}

#[test]
fn a_first_stream_opened_from_a_session_locked_before_it_starts_locked_and_takes_its_first_packet()
{
    let _guard = registry_test_guard();
    let session = ListenerSyncSession::new().expect("session");
    assert!(!session.is_locked());
    session
        .begin_sync_probe(1, 10_000)
        .expect("probe registered");
    let outcome = session
        .observe_sync_response(1, 10_000, 500_000, 500_002, 10_020)
        .expect("correlated response");
    assert!(outcome.accepted);
    assert!(
        outcome.sync_locked,
        "a stream opened now would start locked"
    );
    assert!(session.is_locked());
    let before = session.now_ms();

    let stream = ListenerPlaybackRuntime::start_continuing(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        session.sync_handover(),
    )
    .expect("first stream starts");
    assert!(
        stream.diagnostics().sync_locked,
        "locked before any sample of its own"
    );
    assert!(stream.now_ms() >= before, "the session's clock continues");
    stream
        .submit_packet(first_packet())
        .expect("packet submitted");
    let diagnostics = stream.diagnostics();
    assert_eq!(diagnostics.packets_accepted, 1, "taken at once, not held");
    assert_eq!(diagnostics.dropped_before_sync, 0);
    assert_eq!(diagnostics.replayed_after_sync, 0);
    // The estimate carries on: the stream's next sample counts on top.
    stream
        .begin_sync_probe(2, 10_100)
        .expect("probe registered");
    let next = stream
        .observe_sync_response(2, 10_100, 500_080, 500_082, 10_120)
        .expect("correlated response");
    assert_eq!(next.accepted_sample_count, 2);
    stream.stop().expect("stream stops");
}

#[test]
fn a_session_not_yet_locked_leaves_the_first_stream_waiting_for_sync() {
    let _guard = registry_test_guard();
    let session = ListenerSyncSession::new().expect("session");
    let stream = ListenerPlaybackRuntime::start_continuing(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        session.sync_handover(),
    )
    .expect("first stream starts");
    assert!(!stream.diagnostics().sync_locked);
    stream
        .submit_packet(first_packet())
        .expect("packet submitted");
    assert_eq!(
        stream.diagnostics().packets_accepted,
        0,
        "held for the lock"
    );
    stream.stop().expect("stream stops");
}

#[test]
fn an_uncorrelated_response_is_refused_by_the_session() {
    let session = ListenerSyncSession::new().expect("session");
    assert!(
        session
            .observe_sync_response(9, 10_000, 500_000, 500_002, 10_020)
            .is_err()
    );
    session
        .begin_sync_probe(1, 10_000)
        .expect("probe registered");
    assert!(
        session.begin_sync_probe(1, 10_001).is_err(),
        "duplicate correlation ID"
    );
}

#[test]
fn the_ffi_session_hands_its_lock_to_the_first_stream_handle() {
    let _guard = registry_test_guard();
    let session = FfiListenerSyncSession::new().expect("session");
    session
        .begin_sync_probe(1, 10_000)
        .expect("probe registered");
    let outcome = session
        .observe_sync_response(1, 10_000, 500_000, 500_002, 10_020)
        .expect("correlated response");
    assert!(outcome.sync_locked);
    let stream = FfiListenerPlaybackHandle::open_continuing(
        FfiListenerPlaybackConfig {
            session_id: "session-first".to_owned(),
            stream_id: "stream-first".to_owned(),
            sample_rate: 48_000,
            host_start_time_ms: HOST_START_MS,
            samples_per_packet: SAMPLES_PER_PACKET,
            channels: 2,
            startup_buffer_target_ms: 0,
            rebuffer_target_ms: 0,
            ring_capacity_frames: 48_000,
            ring_target_fill_frames: 19_200,
            write_lead_ms: 400,
            max_prefill_ms: 800,
            volume: 1.0,
        },
        session.sync_handover(),
    )
    .expect("first stream opens");
    assert!(stream.diagnostics().sync_locked);
    stream.stop().expect("stream stops");
}
