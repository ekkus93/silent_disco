use super::{
    MAX_RING_CAPACITY_FRAMES, MIN_RING_CAPACITY_FRAMES, RENDER_CHANNELS, RenderRing,
    RenderRingConfig, RenderRingConfigErrorKind,
};

/// Produces `frame_count` frames of strictly increasing, globally unique
/// sample values starting at `start_frame`, so any reordering, duplication,
/// or corruption in a round trip is immediately visible as a mismatch.
#[allow(clippy::cast_precision_loss)]
fn interleaved_sequence(start_frame: usize, frame_count: usize) -> Vec<f32> {
    (0..frame_count * RENDER_CHANNELS)
        .map(|offset| (start_frame * RENDER_CHANNELS + offset) as f32)
        .collect()
}

#[test]
fn default_config_matches_the_documented_spec_defaults() {
    let config = RenderRingConfig::new();
    assert_eq!(config.capacity_frames, 48_000);
    assert_eq!(config.target_fill_frames, 19_200);
    assert_eq!(RENDER_CHANNELS, 2);
}

#[test]
fn read_from_an_empty_ring_returns_only_silence() {
    let ring = RenderRing::new(RenderRingConfig::new()).expect("valid config");
    let (_producer, consumer) = ring.split();

    let mut output = vec![1.0_f32; RENDER_CHANNELS * 4];
    let outcome = consumer.read_frames(&mut output);

    assert_eq!(outcome.frames_supplied, 0);
    assert_eq!(outcome.frames_silence_filled, 4);
    assert!(output.iter().all(|&sample| sample == 0.0));
    assert_eq!(consumer.telemetry().underrun_callbacks, 1);
}

#[test]
fn push_beyond_capacity_only_writes_what_fits_and_records_ring_full() {
    let capacity = MIN_RING_CAPACITY_FRAMES;
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: capacity,
        target_fill_frames: 1,
    })
    .expect("valid config");
    let (producer, _consumer) = ring.split();

    let frames = interleaved_sequence(0, capacity + 10);
    let written = producer.push_frames(&frames);

    assert_eq!(written, capacity);
    assert_eq!(producer.free_frames(), 0);
    assert_eq!(producer.telemetry().ring_full_events, 1);
    assert_eq!(producer.telemetry().frames_produced, capacity as u64);
}

#[test]
fn partial_write_then_partial_read_round_trips_exact_samples() {
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: MIN_RING_CAPACITY_FRAMES,
        target_fill_frames: 1,
    })
    .expect("valid config");
    let (producer, consumer) = ring.split();

    let input = interleaved_sequence(0, 5);
    assert_eq!(producer.push_frames(&input), 5);

    let mut first_read = vec![-1.0_f32; RENDER_CHANNELS * 3];
    let first_outcome = consumer.read_frames(&mut first_read);
    assert_eq!(first_outcome.frames_supplied, 3);
    assert_eq!(first_outcome.frames_silence_filled, 0);
    assert_eq!(first_read, interleaved_sequence(0, 3));

    let mut second_read = vec![-1.0_f32; RENDER_CHANNELS * 4];
    let second_outcome = consumer.read_frames(&mut second_read);
    assert_eq!(second_outcome.frames_supplied, 2);
    assert_eq!(second_outcome.frames_silence_filled, 2);
    assert_eq!(
        &second_read[..RENDER_CHANNELS * 2],
        &interleaved_sequence(3, 2)[..]
    );
    assert!(second_read[RENDER_CHANNELS * 2..].iter().all(|&s| s == 0.0));
}

#[test]
fn wraps_around_the_backing_buffer_without_corrupting_data() {
    let capacity = MIN_RING_CAPACITY_FRAMES;
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: capacity,
        target_fill_frames: 1,
    })
    .expect("valid config");
    let (producer, consumer) = ring.split();

    let batch = 100;
    let laps = 3;
    let mut next_frame = 0usize;
    let mut output = vec![0.0_f32; batch * RENDER_CHANNELS];

    for _ in 0..(capacity / batch * laps) {
        let input = interleaved_sequence(next_frame, batch);
        assert_eq!(producer.push_frames(&input), batch);

        let outcome = consumer.read_frames(&mut output);
        assert_eq!(outcome.frames_supplied, batch);
        assert_eq!(outcome.frames_silence_filled, 0);
        assert_eq!(output, input);

        next_frame += batch;
    }
}

#[test]
fn producer_faster_than_consumer_never_corrupts_or_loses_data() {
    let capacity = MIN_RING_CAPACITY_FRAMES;
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: capacity,
        target_fill_frames: 1,
    })
    .expect("valid config");
    let (producer, consumer) = ring.split();

    let total_frames = capacity * 5;
    let write_batch = 37;

    let producer_thread = std::thread::spawn(move || {
        let mut next_frame = 0usize;
        while next_frame < total_frames {
            let this_batch = write_batch.min(total_frames - next_frame);
            let input = interleaved_sequence(next_frame, this_batch);
            let mut offset = 0;
            while offset < this_batch {
                let written = producer.push_frames(&input[offset * RENDER_CHANNELS..]);
                offset += written;
                if written == 0 {
                    std::thread::yield_now();
                }
            }
            next_frame += this_batch;
        }
    });

    let read_batch = 11;
    let mut received = Vec::with_capacity(total_frames * RENDER_CHANNELS);
    let mut output = vec![-1.0_f32; read_batch * RENDER_CHANNELS];
    while received.len() < total_frames * RENDER_CHANNELS {
        let outcome = consumer.read_frames(&mut output);
        received.extend_from_slice(&output[..outcome.frames_supplied * RENDER_CHANNELS]);
        if outcome.frames_supplied == 0 {
            std::thread::yield_now();
        }
    }
    producer_thread
        .join()
        .expect("producer thread must not panic");

    assert_eq!(received, interleaved_sequence(0, total_frames));
}

#[test]
fn consumer_faster_than_producer_fills_silence_without_corrupting_later_data() {
    let ring = RenderRing::new(RenderRingConfig {
        capacity_frames: MIN_RING_CAPACITY_FRAMES,
        target_fill_frames: 1,
    })
    .expect("valid config");
    let (producer, consumer) = ring.split();

    let total_frames = 500;
    let producer_thread = std::thread::spawn(move || {
        for frame in 0..total_frames {
            let input = interleaved_sequence(frame, 1);
            while producer.push_frames(&input) == 0 {
                std::thread::yield_now();
            }
            std::thread::yield_now();
        }
    });

    let mut received = Vec::with_capacity(total_frames * RENDER_CHANNELS);
    let mut output = vec![-1.0_f32; RENDER_CHANNELS];
    while received.len() < total_frames * RENDER_CHANNELS {
        let outcome = consumer.read_frames(&mut output);
        if outcome.frames_supplied == 1 {
            received.extend_from_slice(&output);
        }
    }
    producer_thread
        .join()
        .expect("producer thread must not panic");

    assert_eq!(received, interleaved_sequence(0, total_frames));
    assert!(
        consumer.telemetry().underrun_callbacks > 0,
        "a consumer polling faster than a single-frame-at-a-time producer must observe at least one silence-filled read"
    );
}

#[test]
fn repeated_ring_creation_and_teardown_leaves_no_residual_state() {
    for _ in 0..20 {
        let ring = RenderRing::new(RenderRingConfig::new()).expect("valid config");
        let (producer, consumer) = ring.split();

        let input = interleaved_sequence(0, 10);
        assert_eq!(producer.push_frames(&input), 10);

        let mut output = vec![0.0_f32; 10 * RENDER_CHANNELS];
        let outcome = consumer.read_frames(&mut output);
        assert_eq!(outcome.frames_supplied, 10);
        assert_eq!(output, input);
        assert_eq!(consumer.telemetry().frames_produced, 10);
    }
}

#[test]
fn rejects_capacity_below_the_minimum() {
    let config = RenderRingConfig {
        capacity_frames: MIN_RING_CAPACITY_FRAMES - 1,
        ..RenderRingConfig::new()
    };
    let error = RenderRing::new(config).expect_err("too-small capacity must be rejected");
    assert_eq!(error.kind, RenderRingConfigErrorKind::CapacityOutOfRange);
}

#[test]
fn rejects_capacity_above_the_maximum() {
    let config = RenderRingConfig {
        capacity_frames: MAX_RING_CAPACITY_FRAMES + 1,
        ..RenderRingConfig::new()
    };
    let error = RenderRing::new(config).expect_err("too-large capacity must be rejected");
    assert_eq!(error.kind, RenderRingConfigErrorKind::CapacityOutOfRange);
}

#[test]
fn rejects_zero_target_fill() {
    let config = RenderRingConfig {
        target_fill_frames: 0,
        ..RenderRingConfig::new()
    };
    let error = RenderRing::new(config).expect_err("zero target fill must be rejected");
    assert_eq!(error.kind, RenderRingConfigErrorKind::TargetFillOutOfRange);
}

#[test]
fn rejects_target_fill_exceeding_capacity() {
    let config = RenderRingConfig {
        target_fill_frames: RenderRingConfig::new().capacity_frames + 1,
        ..RenderRingConfig::new()
    };
    let error =
        RenderRing::new(config).expect_err("target fill exceeding capacity must be rejected");
    assert_eq!(error.kind, RenderRingConfigErrorKind::TargetFillOutOfRange);
}
