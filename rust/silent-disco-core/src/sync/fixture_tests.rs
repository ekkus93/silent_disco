use super::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};
use crate::domain::SyncConfidence;

const ANDROID_CLOCK_SYNC_FIXTURE: &str = include_str!(
    "../../../../app/src/test/resources/rust-migration/sync/clock_sync_v1.json"
);

#[derive(Debug, Clone, Copy)]
struct FixtureSample {
    t1: u64,
    t2: u64,
    t3: u64,
    t4: u64,
    expected_rtt_ms: f64,
    expected_offset_ms: f64,
    accepted: bool,
}

#[test]
fn android_clock_sync_fixture_matches_rust_estimator() {
    let max_samples = parse_usize(ANDROID_CLOCK_SYNC_FIXTURE, "maxSamples");
    let max_accepted_rtt_ms = parse_f64(ANDROID_CLOCK_SYNC_FIXTURE, "maxAcceptedRttMs");
    let samples_section = array_section(ANDROID_CLOCK_SYNC_FIXTURE, "samples");
    let samples: Vec<FixtureSample> = object_sections(samples_section)
        .into_iter()
        .map(|object| FixtureSample {
            t1: parse_u64(object, "t1"),
            t2: parse_u64(object, "t2"),
            t3: parse_u64(object, "t3"),
            t4: parse_u64(object, "t4"),
            expected_rtt_ms: parse_f64(object, "expectedRttMs"),
            expected_offset_ms: parse_f64(object, "expectedOffsetMs"),
            accepted: parse_bool(object, "accepted"),
        })
        .collect();

    let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig {
        max_samples,
        max_accepted_rtt_ms,
        ..SyncEstimatorConfig::default()
    })
    .unwrap_or_else(|error| panic!("Android fixture config must be valid: {error}"));

    for (index, fixture) in samples.iter().enumerate() {
        let correlation = SyncCorrelationId::new(index as u64 + 1);
        estimator
            .begin_probe(correlation, LocalMonotonicMillis::new(fixture.t1))
            .unwrap_or_else(|error| panic!("fixture probe registration failed: {error}"));
        let observation = estimator
            .observe_response(
                correlation,
                LocalMonotonicMillis::new(fixture.t1),
                HostMonotonicMillis::new(fixture.t2),
                HostMonotonicMillis::new(fixture.t3),
                LocalMonotonicMillis::new(fixture.t4),
            )
            .unwrap_or_else(|error| panic!("fixture observation failed: {error}"));
        assert_eq!(observation.sample.round_trip_time_ms, fixture.expected_rtt_ms);
        assert_eq!(observation.sample.offset_ms, fixture.expected_offset_ms);
        assert_eq!(observation.accepted, fixture.accepted);
    }

    let expected = object_section(ANDROID_CLOCK_SYNC_FIXTURE, "expectedFinalState");
    let snapshot = estimator.snapshot();
    assert_eq!(snapshot.offset_ms, parse_f64(expected, "offsetMs"));
    assert_eq!(
        snapshot.round_trip_time_ms,
        parse_f64(expected, "rttMs")
    );
    assert_eq!(snapshot.jitter_ms, parse_f64(expected, "jitterMs"));
    assert_eq!(
        snapshot.confidence,
        parse_confidence(expected, "confidence")
    );
}

fn key_position(source: &str, key: &str) -> usize {
    source
        .find(&format!("\"{key}\""))
        .unwrap_or_else(|| panic!("fixture key {key} is missing"))
}

fn raw_value<'a>(source: &'a str, key: &str) -> &'a str {
    let key_start = key_position(source, key);
    let after_key = &source[key_start..];
    let colon = after_key
        .find(':')
        .unwrap_or_else(|| panic!("fixture key {key} has no colon"));
    let value = after_key[colon + 1..].trim_start();
    let end = value
        .find([',', '\n', '}'])
        .unwrap_or(value.len());
    value[..end].trim()
}

fn parse_u64(source: &str, key: &str) -> u64 {
    raw_value(source, key)
        .parse()
        .unwrap_or_else(|error| panic!("fixture key {key} is not u64: {error}"))
}

fn parse_usize(source: &str, key: &str) -> usize {
    raw_value(source, key)
        .parse()
        .unwrap_or_else(|error| panic!("fixture key {key} is not usize: {error}"))
}

fn parse_f64(source: &str, key: &str) -> f64 {
    raw_value(source, key)
        .parse()
        .unwrap_or_else(|error| panic!("fixture key {key} is not f64: {error}"))
}

fn parse_bool(source: &str, key: &str) -> bool {
    match raw_value(source, key) {
        "true" => true,
        "false" => false,
        value => panic!("fixture key {key} is not boolean: {value}"),
    }
}

fn parse_confidence(source: &str, key: &str) -> SyncConfidence {
    match raw_value(source, key).trim_matches('"') {
        "EXCELLENT" => SyncConfidence::Excellent,
        "GOOD" => SyncConfidence::Good,
        "FAIR" => SyncConfidence::Fair,
        "POOR" => SyncConfidence::Poor,
        value => panic!("fixture key {key} has unsupported confidence {value}"),
    }
}

fn array_section<'a>(source: &'a str, key: &str) -> &'a str {
    delimited_section(source, key, '[', ']')
}

fn object_section<'a>(source: &'a str, key: &str) -> &'a str {
    delimited_section(source, key, '{', '}')
}

fn delimited_section<'a>(source: &'a str, key: &str, open: char, close: char) -> &'a str {
    let key_start = key_position(source, key);
    let after_key = &source[key_start..];
    let start_offset = after_key
        .find(open)
        .unwrap_or_else(|| panic!("fixture key {key} has no opening delimiter"));
    let start = key_start + start_offset;
    let end = matching_delimiter(source, start, open, close);
    &source[start + open.len_utf8()..end]
}

fn object_sections(source: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let mut search_start = 0;
    while let Some(relative_start) = source[search_start..].find('{') {
        let start = search_start + relative_start;
        let end = matching_delimiter(source, start, '{', '}');
        objects.push(&source[start + 1..end]);
        search_start = end + 1;
    }
    objects
}

fn matching_delimiter(source: &str, start: usize, open: char, close: char) -> usize {
    let mut depth = 0_usize;
    for (relative_index, character) in source[start..].char_indices() {
        if character == open {
            depth += 1;
        } else if character == close {
            depth = depth
                .checked_sub(1)
                .unwrap_or_else(|| panic!("fixture delimiter nesting underflow"));
            if depth == 0 {
                return start + relative_index;
            }
        }
    }
    panic!("fixture delimiter is not closed")
}
