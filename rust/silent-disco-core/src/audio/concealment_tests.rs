use super::{
    ConcealmentConfigErrorKind, ConcealmentOutcome, ConcealmentPolicy,
    MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT,
};

#[test]
fn conceals_with_fresh_silence_of_the_expected_length() {
    let mut policy = ConcealmentPolicy::new(5).expect("valid policy");
    let (samples, outcome) = policy.conceal(960, 2);

    assert_eq!(samples.len(), 960 * 2);
    assert!(samples.iter().all(|&sample| sample == 0));
    assert_eq!(outcome, ConcealmentOutcome::Concealed);
    assert_eq!(policy.statistics().total_concealed_packets, 1);
    assert_eq!(policy.statistics().consecutive_concealed_packets, 1);
}

#[test]
fn never_reuses_a_previous_buffer_across_calls() {
    let mut policy = ConcealmentPolicy::new(5).expect("valid policy");
    let (first, _) = policy.conceal(4, 1);
    let (second, _) = policy.conceal(4, 1);

    // Each call must allocate its own silent buffer rather than sharing or
    // mutating a cached one; corrupting `first` must never affect `second`.
    assert_eq!(first, vec![0_i16; 4]);
    assert_eq!(second, vec![0_i16; 4]);
    assert_ne!(first.as_ptr(), second.as_ptr());
}

#[test]
fn record_delivery_resets_the_consecutive_count_but_not_the_total() {
    let mut policy = ConcealmentPolicy::new(5).expect("valid policy");
    policy.conceal(4, 1);
    policy.conceal(4, 1);
    policy.record_delivery();

    assert_eq!(policy.statistics().consecutive_concealed_packets, 0);
    assert_eq!(policy.statistics().total_concealed_packets, 2);
}

#[test]
fn signals_hard_resync_once_the_consecutive_bound_is_reached() {
    let mut policy = ConcealmentPolicy::new(3).expect("valid policy");

    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
    let (_, outcome) = policy.conceal(4, 1);

    assert_eq!(outcome, ConcealmentOutcome::HardResyncRequired);
    assert_eq!(policy.statistics().hard_resync_signals, 1);
    assert_eq!(policy.statistics().consecutive_concealed_packets, 3);
}

#[test]
fn reset_consecutive_count_allows_concealment_to_resume_after_a_rebuffer() {
    let mut policy = ConcealmentPolicy::new(2).expect("valid policy");
    policy.conceal(4, 1);
    policy.conceal(4, 1);
    policy.reset_consecutive_count();

    assert_eq!(policy.statistics().consecutive_concealed_packets, 0);
    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
}

#[test]
fn rejects_a_zero_consecutive_bound() {
    let error = ConcealmentPolicy::new(0).expect_err("zero bound must be rejected");
    assert_eq!(
        error.kind,
        ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange
    );
}

#[test]
fn rejects_an_oversized_consecutive_bound() {
    let error = ConcealmentPolicy::new(MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT + 1)
        .expect_err("oversized bound must be rejected");
    assert_eq!(
        error.kind,
        ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange
    );
}
