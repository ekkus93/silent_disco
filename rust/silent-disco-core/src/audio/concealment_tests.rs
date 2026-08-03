use super::{
    ConcealmentConfigErrorKind, ConcealmentOutcome, ConcealmentPolicy, MAX_CONCEALMENT_RAMP_FRAMES,
    MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT,
};

/// 960 stereo frames of a constant amplitude, matching a 20ms 48kHz packet.
fn constant_packet(amplitude: i16) -> Vec<i16> {
    vec![amplitude; 960 * 2]
}

fn sample_at(samples: &[i16], frame: usize, channel: usize) -> i16 {
    samples[frame * 2 + channel]
}

#[test]
fn conceals_with_silence_before_any_real_packet_has_been_delivered() {
    let mut policy = ConcealmentPolicy::new(5, 240).expect("valid policy");
    let (samples, outcome) = policy.conceal(960, 2);

    // Nothing has been delivered yet, so there is nothing to repeat.
    assert_eq!(samples.len(), 960 * 2);
    assert!(samples.iter().all(|&sample| sample == 0));
    assert_eq!(outcome, ConcealmentOutcome::Concealed);
    assert_eq!(policy.statistics().total_concealed_packets, 1);
    assert_eq!(policy.statistics().consecutive_concealed_packets, 1);
}

#[test]
fn repeats_the_last_real_packet_at_full_amplitude_with_continuous_seams() {
    let mut policy = ConcealmentPolicy::new(5, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let (samples, outcome) = policy.conceal(960, 2);

    assert_eq!(outcome, ConcealmentOutcome::Concealed);
    // Entry continuity: starts exactly where the delivered packet ended.
    assert_eq!(sample_at(&samples, 0, 0), 8_000);
    // Body: the delivered packet repeated at full amplitude, not silence and
    // not attenuated -- one packet of repeat is too short to sound like a loop,
    // and dropping 6dB for it is itself an audible artefact.
    assert_eq!(sample_at(&samples, 480, 0), 8_000);
    // Entry seam is continuous: the repeat starts where the real packet ended.
    assert_eq!(sample_at(&samples, 120, 0), 8_000);
    // Exit: the frame ends on its own amplitude. Whatever follows -- a further
    // concealment or resuming real audio -- continues from there, so forcing
    // the tail to zero here would create the step it prevents.
    assert_eq!(sample_at(&samples, 959, 0), 8_000);
}

#[test]
fn consecutive_concealments_halve_again_each_time_and_decay_toward_silence() {
    let mut policy = ConcealmentPolicy::new(200, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let (first, _) = policy.conceal(960, 2);
    let (second, _) = policy.conceal(960, 2);
    let (third, _) = policy.conceal(960, 2);
    let (eighth, _) = {
        for _ in 0..4 {
            policy.conceal(960, 2);
        }
        policy.conceal(960, 2)
    };

    // The first repeat is full amplitude; every consecutive one halves again.
    assert_eq!(sample_at(&first, 480, 0), 8_000);
    assert_eq!(sample_at(&second, 480, 0), 4_000);
    assert_eq!(sample_at(&third, 480, 0), 2_000);
    // Attenuation saturates at eight halvings; by then the repeat is
    // inaudible rather than looping one packet at a fixed floor forever.
    assert_eq!(sample_at(&eighth, 480, 0), 8_000 >> 7);
}

#[test]
fn a_concealment_run_continues_from_the_previous_concealed_tail() {
    let mut policy = ConcealmentPolicy::new(200, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);
    let (first, _) = policy.conceal(960, 2);

    let (second, _) = policy.conceal(960, 2);

    // The prior frame ended mid-decay, so this one continues from that value
    // rather than restarting from a zero the decay never reached.
    assert_eq!(sample_at(&first, 959, 0), 8_000);
    assert_eq!(sample_at(&second, 0, 0), 8_000);
    assert_eq!(sample_at(&second, 480, 0), 4_000);
}

/// A burst of consecutive losses must read as one decaying gap, not as one
/// blip per lost packet. The previous implementation faded every concealed
/// frame's tail to zero and blended the next one back in from that zero, so a
/// four-packet burst emitted four separate 20ms envelopes -- amplitude
/// modulation at the 50Hz packet rate, which a real device reproduced as an
/// audible hiccup on every burst.
#[test]
fn a_burst_of_losses_decays_continuously_without_returning_to_silence() {
    let mut policy = ConcealmentPolicy::new(200, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let burst: Vec<Vec<i16>> = (0..4).map(|_| policy.conceal(960, 2).0).collect();

    let mut previous_tail = 8_000;
    for (index, frame) in burst.iter().enumerate() {
        // No seam anywhere in the run: each frame opens exactly where the last
        // one closed.
        assert_eq!(
            sample_at(frame, 0, 0),
            previous_tail,
            "frame {index} steps at its opening seam"
        );
        // No frame collapses to silence mid-burst, which is what produced the
        // per-packet envelope.
        assert_ne!(
            sample_at(frame, 959, 0),
            0,
            "frame {index} returned to silence mid-burst"
        );
        previous_tail = sample_at(frame, 959, 0);
    }

    // The envelope still decays monotonically toward silence across the burst.
    let bodies: Vec<i16> = burst.iter().map(|frame| sample_at(frame, 480, 0)).collect();
    assert_eq!(bodies, vec![8_000, 4_000, 2_000, 1_000]);
}

/// The frame that reaches the consecutive bound is discarded by the scheduler
/// in favour of a rebuffer, so the last frame a listener actually hears is the
/// one before it -- and that frame, not the discarded one, has to land on
/// silence.
#[test]
fn the_last_audible_frame_of_a_bounded_run_lands_on_silence() {
    let mut policy = ConcealmentPolicy::new(3, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let (first, first_outcome) = policy.conceal(960, 2);
    let (last_emitted, second_outcome) = policy.conceal(960, 2);
    let (_, bound_outcome) = policy.conceal(960, 2);

    assert_eq!(first_outcome, ConcealmentOutcome::Concealed);
    assert_eq!(second_outcome, ConcealmentOutcome::Concealed);
    assert_eq!(bound_outcome, ConcealmentOutcome::HardResyncRequired);
    assert_ne!(sample_at(&first, 959, 0), 0);
    assert_eq!(sample_at(&last_emitted, 959, 0), 0);
}

#[test]
fn record_delivery_restores_full_amplitude_repetition_for_the_next_gap() {
    let mut policy = ConcealmentPolicy::new(200, 240).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);
    policy.conceal(960, 2);
    policy.conceal(960, 2);
    policy.record_delivery(&constant_packet(6_000), 2);

    let (samples, _) = policy.conceal(960, 2);

    assert_eq!(policy.statistics().consecutive_concealed_packets, 1);
    // Source is the newly delivered packet, and the decay generation reset.
    assert_eq!(sample_at(&samples, 480, 0), 6_000);
    assert_eq!(sample_at(&samples, 0, 0), 6_000);
}

#[test]
fn each_call_allocates_its_own_buffer() {
    let mut policy = ConcealmentPolicy::new(5, 2).expect("valid policy");
    let (first, _) = policy.conceal(4, 1);
    let (second, _) = policy.conceal(4, 1);

    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4);
    assert_ne!(first.as_ptr(), second.as_ptr());
}

#[test]
fn record_delivery_resets_the_consecutive_count_but_not_the_total() {
    let mut policy = ConcealmentPolicy::new(5, 2).expect("valid policy");
    policy.conceal(4, 1);
    policy.conceal(4, 1);
    policy.record_delivery(&[100, 100, 100, 100], 1);

    assert_eq!(policy.statistics().consecutive_concealed_packets, 0);
    assert_eq!(policy.statistics().total_concealed_packets, 2);
}

#[test]
fn signals_hard_resync_once_the_consecutive_bound_is_reached() {
    let mut policy = ConcealmentPolicy::new(3, 2).expect("valid policy");

    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
    let (_, outcome) = policy.conceal(4, 1);

    assert_eq!(outcome, ConcealmentOutcome::HardResyncRequired);
    assert_eq!(policy.statistics().hard_resync_signals, 1);
    assert_eq!(policy.statistics().consecutive_concealed_packets, 3);
}

#[test]
fn reset_consecutive_count_allows_concealment_to_resume_after_a_rebuffer() {
    let mut policy = ConcealmentPolicy::new(2, 2).expect("valid policy");
    policy.conceal(4, 1);
    policy.conceal(4, 1);
    policy.reset_consecutive_count();

    assert_eq!(policy.statistics().consecutive_concealed_packets, 0);
    assert_eq!(policy.conceal(4, 1).1, ConcealmentOutcome::Concealed);
}

#[test]
fn a_ramp_longer_than_the_packet_still_produces_a_full_length_frame() {
    let mut policy = ConcealmentPolicy::new(5, 4_000).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let (samples, _) = policy.conceal(960, 2);

    // The ramp is clamped to the frame's own length rather than truncating it.
    assert_eq!(samples.len(), 960 * 2);
    // Entry still starts exactly where the delivered packet ended.
    assert_eq!(sample_at(&samples, 0, 0), 8_000);
}

#[test]
fn a_bounded_run_shorter_than_its_ramp_still_lands_on_silence() {
    let mut policy = ConcealmentPolicy::new(1, 4_000).expect("valid policy");
    policy.record_delivery(&constant_packet(8_000), 2);

    let (samples, outcome) = policy.conceal(960, 2);

    assert_eq!(outcome, ConcealmentOutcome::HardResyncRequired);
    assert_eq!(samples.len(), 960 * 2);
    assert_eq!(sample_at(&samples, 959, 0), 0);
}

#[test]
fn rejects_a_zero_consecutive_bound() {
    let error = ConcealmentPolicy::new(0, 240).expect_err("zero bound must be rejected");
    assert_eq!(
        error.kind,
        ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange
    );
}

#[test]
fn rejects_an_oversized_consecutive_bound() {
    let error = ConcealmentPolicy::new(MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT + 1, 240)
        .expect_err("oversized bound must be rejected");
    assert_eq!(
        error.kind,
        ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange
    );
}

#[test]
fn rejects_an_out_of_range_ramp() {
    assert_eq!(
        ConcealmentPolicy::new(5, 0)
            .expect_err("zero ramp must be rejected")
            .kind,
        ConcealmentConfigErrorKind::RampFramesOutOfRange
    );
    assert_eq!(
        ConcealmentPolicy::new(5, MAX_CONCEALMENT_RAMP_FRAMES + 1)
            .expect_err("oversized ramp must be rejected")
            .kind,
        ConcealmentConfigErrorKind::RampFramesOutOfRange
    );
}
