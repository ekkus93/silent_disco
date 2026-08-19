//! The clock-offset gate: nothing plays before a real offset lands, the
//! first offset is adopted outright, later offsets correct softly or force a
//! rebuffer, and pre-sync packets are dropped rather than buffered.

use crate::audio::{PlaybackScheduler, RenderRing, RenderRingConfig, SchedulerConfig};
use crate::domain::{SessionId, StreamId};

use super::super::{PlaybackPump, PumpTick, SyncApplyOutcome};
use super::{
    HOST_START_MS, PACKET_DURATION_MS, SAMPLES_PER_PACKET, datagram, pump_with_unlocked_sync,
    unpaced_config,
};

#[test]
fn nothing_plays_until_a_real_clock_offset_has_been_applied() {
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("session-pump").expect("session id"),
        StreamId::new("stream-pump").expect("stream id"),
        48_000,
        HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    })
    .expect("valid ring");
    let (producer, _consumer) = ring.split();
    let mut pump =
        PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
    for sequence in 0..30 {
        pump.scheduler_mut()
            .submit_packet(datagram(sequence, 16_384))
            .expect("accepted");
    }

    // Audio is buffered and its deadlines have passed, but no sync sample
    // has been accepted: playing now would map every presentation time
    // through a placeholder offset.
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::AwaitingSync));
    assert!(!pump.is_sync_locked());
    assert_eq!(pump.queued_frames(), 0);

    assert_eq!(pump.apply_sync_offset(0.0), SyncApplyOutcome::Locked);
    assert!(pump.is_sync_locked());
    assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
}

#[test]
fn the_first_offset_is_adopted_outright_rather_than_treated_as_a_correction() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();

    // Host and listener clocks have unrelated epochs, so a real first
    // offset dwarfs any correction threshold. Adopting it must not be
    // mistaken for a jump that needs a rebuffer.
    assert_eq!(
        pump.apply_sync_offset(-746_105_745.0),
        SyncApplyOutcome::Locked
    );
    assert!(!pump.scheduler_mut().is_awaiting_rebuffer());
}

#[test]
fn later_offsets_correct_softly_or_force_a_rebuffer_when_they_jump() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();
    pump.apply_sync_offset(1_000.0);

    assert_eq!(
        pump.apply_sync_offset(1_010.0),
        SyncApplyOutcome::SoftCorrected
    );
    assert_eq!(pump.diagnostics().offset_driven_rebuffers, 0);
    // Beyond the hard-resync threshold: re-accumulate rather than splice.
    assert_eq!(
        pump.apply_sync_offset(1_500.0),
        SyncApplyOutcome::Rebuffered
    );
    // A4.4: an offset-driven rebuffer must be counted, and counted in
    // `hard_resync_signals` too -- before this fix, `SyncApplyOutcome::
    // Rebuffered` was produced but discarded at its one production call
    // site, so `hardResyncs` silently under-reported any stream whose
    // rebuffers were offset-driven rather than concealment-driven.
    let diagnostics = pump.diagnostics();
    assert_eq!(diagnostics.offset_driven_rebuffers, 1);
    assert_eq!(diagnostics.concealment_driven_rebuffers, 0);
    assert_eq!(diagnostics.hard_resync_signals, 1);

    // A second jump keeps counting, independently of the concealment path.
    assert_eq!(
        pump.apply_sync_offset(2_100.0),
        SyncApplyOutcome::Rebuffered
    );
    let diagnostics = pump.diagnostics();
    assert_eq!(diagnostics.offset_driven_rebuffers, 2);
    assert_eq!(diagnostics.hard_resync_signals, 2);
}

#[test]
fn packets_arriving_before_sync_locks_are_dropped_rather_than_stranding_the_buffer() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();

    // A whole second of audio arrives while the clock is still unlocked.
    // Buffering it would pile it against sequence zero until the reorder
    // window overflows, losing far more than the wait itself costs.
    for sequence in 0..50 {
        pump.submit_packet(datagram(sequence, 16_384))
            .expect("a pre-sync packet is dropped, not an error");
    }
    assert_eq!(pump.dropped_before_sync(), 50);
    assert_eq!(pump.diagnostics().packets_accepted, 0);
    assert_eq!(pump.diagnostics().reorder_window_rejections, 0);

    // Once the clock locks, the buffer adopts the live position: the
    // first arrivals corroborate the jump, then audio flows again.
    pump.apply_sync_offset(0.0);
    for sequence in 200..215 {
        let _ = pump.submit_packet(datagram(sequence, 16_384));
    }
    let diagnostics = pump.diagnostics();
    assert!(
        diagnostics.packets_accepted > 0,
        "the live stream was never picked up after sync locked"
    );
    assert_eq!(diagnostics.resynchronisations, 1);
}

#[test]
fn non_finite_offsets_are_rejected_without_locking_or_mutating_playback() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();

    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            pump.apply_sync_offset(invalid),
            SyncApplyOutcome::RejectedNonFinite
        );
        assert!(!pump.is_sync_locked());
    }

    assert_eq!(pump.apply_sync_offset(100.0), SyncApplyOutcome::Locked);
    assert!(pump.is_sync_locked());
    assert_eq!(
        pump.apply_sync_offset(f64::NAN),
        SyncApplyOutcome::RejectedNonFinite
    );
    // The rejected NaN must not poison the stored offset: a small correction
    // from the last valid 100ms estimate remains a soft correction.
    assert_eq!(
        pump.apply_sync_offset(110.0),
        SyncApplyOutcome::SoftCorrected
    );
}

#[test]
fn exactly_the_hard_resync_threshold_is_still_a_soft_correction() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();
    assert_eq!(pump.apply_sync_offset(0.0), SyncApplyOutcome::Locked);

    assert_eq!(
        pump.apply_sync_offset(crate::audio::DEFAULT_HARD_RESYNC_THRESHOLD_MS),
        SyncApplyOutcome::SoftCorrected,
        "the soft branch is inclusive; only a delta above the threshold may rebuffer"
    );
    assert_eq!(pump.diagnostics().offset_driven_rebuffers, 0);
    assert_eq!(
        pump.scheduler_mut().local_time_for_host_ms(1_000),
        995,
        "threshold equality stays soft and therefore uses the bounded 5ms slew"
    );
}

#[test]
fn soft_offset_updates_are_slew_limited_and_converge_without_an_instant_jump() {
    let (mut pump, _consumer) = pump_with_unlocked_sync();
    assert_eq!(pump.apply_sync_offset(0.0), SyncApplyOutcome::Locked);

    assert_eq!(
        pump.apply_sync_offset(100.0),
        SyncApplyOutcome::SoftCorrected
    );
    assert_eq!(
        pump.scheduler_mut().local_time_for_host_ms(1_000),
        995,
        "a 100ms estimator correction must move the playback timeline only 5ms on one update"
    );

    for _ in 0..19 {
        assert_eq!(
            pump.apply_sync_offset(100.0),
            SyncApplyOutcome::SoftCorrected
        );
    }
    assert_eq!(
        pump.scheduler_mut().local_time_for_host_ms(1_000),
        900,
        "repeated accepted observations must converge to the estimator target"
    );
    assert_eq!(pump.diagnostics().offset_driven_rebuffers, 0);
}
