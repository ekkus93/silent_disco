use super::ramp::{apply_fade_in, apply_fade_out_tail, blend_sample, last_frame, scale_sample};

#[test]
fn scale_sample_scales_and_treats_a_zero_denominator_as_silence() {
    assert_eq!(scale_sample(8_000, 1, 2), 4_000);
    assert_eq!(scale_sample(8_000, 0, 4), 0);
    assert_eq!(scale_sample(8_000, 4, 4), 8_000);
    assert_eq!(scale_sample(-8_000, 1, 4), -2_000);
    assert_eq!(scale_sample(8_000, 1, 0), 0);
}

#[test]
fn scale_sample_saturates_rather_than_wrapping_on_amplification() {
    assert_eq!(scale_sample(i16::MAX, 4, 1), i16::MAX);
    assert_eq!(scale_sample(i16::MIN, 4, 1), i16::MIN);
}

#[test]
fn blend_sample_walks_from_the_previous_value_to_the_target() {
    assert_eq!(blend_sample(8_000, 0, 0, 4), 0);
    assert_eq!(blend_sample(8_000, 0, 2, 4), 4_000);
    assert_eq!(blend_sample(8_000, 0, 4, 4), 8_000);
    // Starting from a non-zero previous value.
    assert_eq!(blend_sample(0, 8_000, 0, 4), 8_000);
    assert_eq!(blend_sample(0, 8_000, 2, 4), 4_000);
    // A zero-length ramp cannot blend, so the target applies immediately.
    assert_eq!(blend_sample(8_000, 100, 0, 0), 8_000);
}

#[test]
fn last_frame_returns_the_final_frames_channels_or_nothing() {
    let stereo = [1_i16, 2, 3, 4, 5, 6];
    assert_eq!(last_frame(&stereo, 2), Some(&[5_i16, 6][..]));
    assert_eq!(last_frame(&[], 2), None);
    // A partial frame is not a frame.
    assert_eq!(last_frame(&[1_i16], 2), None);
    assert_eq!(last_frame(&stereo, 0), None);
}

#[test]
fn fade_in_ramps_the_head_and_leaves_the_rest_alone() {
    let mut samples = vec![8_000_i16; 10 * 2];
    apply_fade_in(&mut samples, 2, 4);

    assert_eq!(samples[0], 0);
    assert_eq!(samples[1], 0);
    assert_eq!(samples[2 * 2], 4_000);
    assert_eq!(samples[4 * 2], 8_000);
    assert_eq!(samples[9 * 2], 8_000);
}

#[test]
fn fade_out_ramps_the_tail_to_zero_and_leaves_the_rest_alone() {
    let mut samples = vec![8_000_i16; 10 * 2];
    apply_fade_out_tail(&mut samples, 2, 4);

    assert_eq!(samples[0], 8_000);
    assert_eq!(samples[5 * 2], 8_000);
    assert_eq!(samples[9 * 2], 0);
    assert_eq!(samples[9 * 2 + 1], 0);
}

#[test]
fn a_ramp_longer_than_the_buffer_fades_across_its_whole_span_without_panicking() {
    let mut faded_in = vec![8_000_i16; 3 * 2];
    apply_fade_in(&mut faded_in, 2, 1_000);
    assert_eq!(faded_in[0], 0);

    let mut faded_out = vec![8_000_i16; 3 * 2];
    apply_fade_out_tail(&mut faded_out, 2, 1_000);
    assert_eq!(faded_out[2 * 2], 0);
}

#[test]
fn degenerate_shapes_are_no_ops_rather_than_panics() {
    let mut empty: Vec<i16> = Vec::new();
    apply_fade_in(&mut empty, 2, 4);
    apply_fade_out_tail(&mut empty, 2, 4);
    assert!(empty.is_empty());

    let mut zero_channels = vec![1_i16, 2, 3];
    apply_fade_in(&mut zero_channels, 0, 4);
    apply_fade_out_tail(&mut zero_channels, 0, 4);
    assert_eq!(zero_channels, vec![1, 2, 3]);
}
