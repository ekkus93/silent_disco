use super::{LabClock, LabClockError, LabNodeClock};
use silent_disco_core::transport::TransportClock;
use std::sync::{Arc, Mutex};

/// Block 38.3 "exact offset": a node clock with a pure offset (no drift)
/// tracks the base clock exactly, shifted by precisely that offset.
#[test]
fn a_pure_offset_shifts_time_exactly() {
    let base = Arc::new(LabClock::new(1_000));
    let node = LabNodeClock::new(Arc::clone(&base), 250, 0).expect("valid offset");
    assert_eq!(node.now().get(), 1_250);

    base.advance(500).expect("advance");
    assert_eq!(node.now().get(), 1_750);
}

/// Block 38.3 "positive and negative drift": a node running fast (positive
/// ppm) observes more elapsed time than the base clock; one running slow
/// (negative ppm) observes less, in exact proportion to elapsed base time.
#[test]
fn positive_and_negative_drift_move_time_proportionally() {
    let base = Arc::new(LabClock::new(0));
    let fast = LabNodeClock::new(Arc::clone(&base), 0, 10_000).expect("valid drift"); // +1%
    let slow = LabNodeClock::new(Arc::clone(&base), 0, -10_000).expect("valid drift"); // -1%
    let steady = LabNodeClock::new(Arc::clone(&base), 0, 0).expect("valid drift");

    base.advance(1_000_000).expect("advance one million ms");

    assert_eq!(steady.now().get(), 1_000_000);
    assert_eq!(fast.now().get(), 1_010_000);
    assert_eq!(slow.now().get(), 990_000);
}

/// Block 38.3 "long-run arithmetic": repeated, large advances accumulate
/// exactly as the drift formula predicts -- no cumulative rounding error
/// beyond the single final integer truncation, and no overflow across a
/// scenario spanning a simulated multi-year duration.
#[test]
fn long_run_advances_match_the_drift_formula_exactly() {
    const STEP_MS: u64 = 86_400_000; // one simulated day
    const STEPS: u64 = 400; // > one simulated year, in a few hundred steps

    let base = Arc::new(LabClock::new(0));
    let node = LabNodeClock::new(Arc::clone(&base), -5_000, 250).expect("valid drift");

    for _ in 0..STEPS {
        base.advance(STEP_MS).expect("advance one simulated day");
    }

    let expected_base = STEP_MS * STEPS;
    let expected = i128::from(expected_base) * (1_000_000 + 250) / 1_000_000 - 5_000;
    assert_eq!(
        node.now().get(),
        u64::try_from(expected).expect("fits in u64")
    );
}

/// Block 38.3 "overflow rejection": an advance that would overflow `u64`
/// milliseconds is rejected outright and leaves virtual time completely
/// unchanged; an out-of-bounds drift configuration is rejected at
/// construction, not silently clamped.
#[test]
fn overflow_and_out_of_bounds_configuration_are_rejected_not_silently_accepted() {
    let base = Arc::new(LabClock::new(u64::MAX - 10));
    let error = base
        .advance(20)
        .expect_err("an overflowing advance must be rejected");
    assert_eq!(error, LabClockError::AdvanceOverflow);
    assert_eq!(
        base.now().get(),
        u64::MAX - 10,
        "time must be unchanged after a rejected advance"
    );

    let Err(error) = LabNodeClock::new(Arc::clone(&base), 0, 1_000_000) else {
        panic!("drift far beyond the supported bound must be rejected");
    };
    assert_eq!(error, LabClockError::DriftOutOfBounds);
}

/// Block 38.3 "deterministic repeated seed": two independently constructed
/// clocks, given the identical initial time and the identical sequence of
/// advances, produce byte-identical results at every step -- proving
/// nothing here leaks real wall-clock time or any other hidden source of
/// nondeterminism.
#[test]
fn identical_seeds_and_advances_produce_identical_results() {
    let first = LabClock::new(12_345);
    let second = LabClock::new(12_345);
    let steps = [10, 999, 1, 60_000, 7, 86_400_000];

    for step in steps {
        let first_now = first.advance(step).expect("advance first clock");
        let second_now = second.advance(step).expect("advance second clock");
        assert_eq!(first_now, second_now);
    }
    assert_eq!(first.now(), second.now());
}

/// Block 38.3 "scheduler event order at equal timestamps": wakeups
/// registered for the exact same deadline always run in registration
/// order, deterministically -- never in `BinaryHeap`'s own unspecified
/// tie-breaking order.
#[test]
fn wakeups_at_the_same_deadline_run_in_registration_order() {
    let clock = LabClock::new(0);
    let order: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));

    for id in 0..5_u32 {
        let order = Arc::clone(&order);
        clock
            .schedule(1_000, move || order.lock().expect("lock").push(id))
            .expect("schedule wakeup");
    }

    clock
        .advance(1_000)
        .expect("advance to the shared deadline");
    assert_eq!(*order.lock().expect("lock"), vec![0, 1, 2, 3, 4]);
}

/// A wakeup scheduled in the future must not run early, and must run
/// exactly once once its deadline is reached (never dropped, never
/// re-run on a later advance).
#[test]
fn a_future_wakeup_runs_exactly_once_when_its_deadline_is_reached() {
    let clock = LabClock::new(0);
    let ran = Arc::new(Mutex::new(0_u32));
    let counter = Arc::clone(&ran);
    clock
        .schedule(500, move || *counter.lock().expect("lock") += 1)
        .expect("schedule wakeup");

    clock.advance(200).expect("advance before the deadline");
    assert_eq!(
        *ran.lock().expect("lock"),
        0,
        "must not run before its deadline"
    );

    clock.advance(300).expect("advance to the deadline");
    assert_eq!(*ran.lock().expect("lock"), 1);

    clock
        .advance(1_000)
        .expect("advance well past the deadline");
    assert_eq!(
        *ran.lock().expect("lock"),
        1,
        "must never run a second time"
    );
}

/// Block 38.2 "explicit invalid-discontinuity injection only for negative
/// tests": `force_discontinuity` can move time backward -- something
/// `advance` can never do -- and does not corrupt the clock's ability to
/// resume normal, checked advancement afterward.
#[test]
fn force_discontinuity_moves_time_backward_and_normal_advance_still_works_after() {
    let clock = LabClock::new(10_000);
    clock.force_discontinuity(1_000);
    assert_eq!(
        clock.now().get(),
        1_000,
        "a discontinuity can move time backward"
    );

    let next = clock
        .advance(500)
        .expect("advance still works after a discontinuity");
    assert_eq!(next.get(), 1_500);
}
