use super::{
    AssertionOutcome, ScenarioExecutionError, ScenarioOutcome, ScenarioTrace,
    ScenarioValidationError, load_scenario_json, run_scenario, run_scenario_with_trace,
};
use crate::lab::LabRuntime;
use crate::lab::recorder::RecordedNotificationKind;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-lab-live-proof-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn live_join_scenario(
    latency_ms: u64,
    loss_permille: u16,
    barrier_ms: u64,
    timeout_ms: u64,
) -> super::Scenario {
    let document = format!(
        r#"{{
            "schemaVersion": 1,
            "seed": 77,
            "nodes": [{{"id": "host1"}}, {{"id": "listener1"}}],
            "links": [{{
                "from": "host1",
                "to": "listener1",
                "latencyMs": {latency_ms},
                "jitterMs": 0,
                "lossPermille": {loss_permille}
            }}],
            "fixtures": [{{"id": "track", "displayName": "Lab Track"}}],
            "steps": [
                {{"atMs": 0, "node": "host1", "action": {{"kind": "selectRole", "role": "host"}}}},
                {{"atMs": 1, "node": "host1", "action": {{"kind": "configureHost", "sessionName": "Lab Party", "fixture": "track"}}}},
                {{"atMs": 2, "node": "host1", "action": {{"kind": "createHostSession"}}}},
                {{"atMs": 3, "node": "listener1", "action": {{"kind": "selectRole", "role": "listener"}}}},
                {{"atMs": 4, "node": "listener1", "action": {{"kind": "startDiscovery"}}}},
                {{"atMs": 5, "node": "listener1", "action": {{"kind": "selectSession", "sessionId": "session-1"}}}},
                {{"atMs": 6, "node": "listener1", "action": {{"kind": "submitJoin"}}}},
                {{"atMs": 7, "node": "host1", "action": {{"kind": "injectUnderrun", "missingFrames": 0}}}},
                {{"atMs": 8, "node": "host1", "action": {{"kind": "approveJoin", "requestId": "desktop-join-1"}}}},
                {{"atMs": {barrier_ms}, "node": "listener1", "action": {{"kind": "injectUnderrun", "missingFrames": 0}}}}
            ],
            "assertions": [
                {{"kind": "listenerCountAtLeast", "byMs": {timeout_ms}, "node": "host1", "count": 1}},
                {{"kind": "lifecycleReached", "byMs": {timeout_ms}, "node": "listener1", "target": {{"machine": "listener", "state": "approved"}}}},
                {{"kind": "synchronizationWithinBounds", "byMs": {timeout_ms}, "node": "listener1", "maxAbsOffsetMs": 1000.0, "maxRoundTripMs": 1000.0}}
            ],
            "timeoutMs": {timeout_ms}
        }}"#
    );
    load_scenario_json(document.as_bytes()).expect("live proof scenario should parse")
}

fn run_live_join(
    latency_ms: u64,
    loss_permille: u16,
    barrier_ms: u64,
    timeout_ms: u64,
) -> (super::ScenarioReport, ScenarioTrace) {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = live_join_scenario(latency_ms, loss_permille, barrier_ms, timeout_ms);
    run_scenario_with_trace(&lab, &scenario).expect("live proof scenario should execute")
}

fn last_listener_round_trip_ms(trace: &ScenarioTrace) -> Option<f64> {
    let (_, entries) = trace
        .node_notifications
        .iter()
        .find(|(node, _)| node == "listener1")?;
    entries.iter().rev().find_map(|entry| {
        let RecordedNotificationKind::Snapshot { summary, .. } = &entry.kind else {
            return None;
        };
        summary
            .synchronization
            .as_ref()
            .map(|sync| sync.round_trip_ms)
    })
}

fn synchronization_assertion(report: &super::ScenarioReport) -> AssertionOutcome {
    report
        .assertion_results
        .iter()
        .find(|result| result.kind == "synchronizationWithinBounds")
        .expect("synchronization assertion should be present")
        .outcome
}

#[test]
fn zero_fault_join_and_sync_use_the_live_transport_path() {
    let (report, trace) = run_live_join(0, 0, 9, 20);

    assert_eq!(report.outcome, ScenarioOutcome::Completed);
    assert!(
        report
            .step_results
            .iter()
            .all(|step| step.submit_error.is_none())
    );
    assert_eq!(synchronization_assertion(&report), AssertionOutcome::Held);
    let round_trip =
        last_listener_round_trip_ms(&trace).expect("live sync sample should be recorded");
    assert!(
        round_trip.abs() <= f64::EPSILON,
        "zero-fault round trip was {round_trip}"
    );
}

#[test]
fn configured_latency_holds_sync_until_deadline_and_reaches_the_estimator() {
    let (early_report, early_trace) = run_live_join(25, 0, 19, 20);
    assert_eq!(early_report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(
        synchronization_assertion(&early_report),
        AssertionOutcome::TimedOut
    );
    assert!(
        last_listener_round_trip_ms(&early_trace).is_none(),
        "sync must not be fabricated before the virtual latency deadline"
    );

    let (released_report, released_trace) = run_live_join(25, 0, 33, 40);
    assert_eq!(released_report.outcome, ScenarioOutcome::Completed);
    let round_trip = last_listener_round_trip_ms(&released_trace)
        .expect("released sync sample should be recorded");
    assert!(
        (round_trip - 25.0).abs() <= f64::EPSILON,
        "25 ms receive latency must be visible to sync estimation; observed {round_trip} ms"
    );
}

#[test]
fn one_hundred_percent_sync_loss_never_fabricates_sync_success() {
    let (report, trace) = run_live_join(0, 1_000, 20, 30);

    assert_eq!(report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(report.assertion_results[0].outcome, AssertionOutcome::Held);
    assert_eq!(report.assertion_results[1].outcome, AssertionOutcome::Held);
    assert_eq!(
        synchronization_assertion(&report),
        AssertionOutcome::TimedOut
    );
    assert!(
        last_listener_round_trip_ms(&trace).is_none(),
        "100% synchronization loss must not create a synthetic sync sample"
    );
}

#[test]
fn conflicting_inbound_link_profiles_are_rejected_before_live_execution() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1,
            "seed": 9,
            "nodes": [{"id": "host1"}, {"id": "host2"}, {"id": "listener1"}],
            "links": [
                {"from": "host1", "to": "listener1", "latencyMs": 0, "jitterMs": 0, "lossPermille": 0},
                {"from": "host2", "to": "listener1", "latencyMs": 25, "jitterMs": 0, "lossPermille": 0}
            ],
            "steps": [],
            "assertions": [],
            "timeoutMs": 50
        }"#,
    )
    .expect("conflict is semantic, not a JSON-shape failure");

    let error = run_scenario(&lab, &scenario).expect_err("conflicting receive profiles must fail");
    assert!(matches!(
        error,
        ScenarioExecutionError::Validation(
            ScenarioValidationError::AmbiguousInboundLinkFaults { ref node }
        ) if node == "listener1"
    ));
}

#[test]
fn mid_run_loss_mutation_applies_to_an_already_connected_listener() {
    let document = br#"{
        "schemaVersion": 1,
        "seed": 77,
        "nodes": [{"id": "host1"}, {"id": "listener1"}],
        "links": [{
            "from": "host1",
            "to": "listener1",
            "latencyMs": 0,
            "jitterMs": 0,
            "lossPermille": 0
        }],
        "fixtures": [{"id": "track", "displayName": "Lab Track"}],
        "steps": [
            {"atMs": 0, "node": "host1", "action": {"kind": "selectRole", "role": "host"}},
            {"atMs": 1, "node": "host1", "action": {"kind": "configureHost", "sessionName": "Lab Party", "fixture": "track"}},
            {"atMs": 2, "node": "host1", "action": {"kind": "createHostSession"}},
            {"atMs": 3, "node": "listener1", "action": {"kind": "selectRole", "role": "listener"}},
            {"atMs": 4, "node": "listener1", "action": {"kind": "startDiscovery"}},
            {"atMs": 5, "node": "listener1", "action": {"kind": "selectSession", "sessionId": "session-1"}},
            {"atMs": 6, "node": "listener1", "action": {"kind": "submitJoin"}},
            {"atMs": 7, "node": "listener1", "action": {
                "kind": "setLinkFaults",
                "from": "host1",
                "to": "listener1",
                "latencyMs": 0,
                "jitterMs": 0,
                "lossPermille": 1000
            }},
            {"atMs": 8, "node": "host1", "action": {"kind": "approveJoin", "requestId": "desktop-join-1"}},
            {"atMs": 20, "node": "listener1", "action": {"kind": "injectUnderrun", "missingFrames": 0}}
        ],
        "assertions": [
            {"kind": "listenerCountAtLeast", "byMs": 30, "node": "host1", "count": 1},
            {"kind": "lifecycleReached", "byMs": 30, "node": "listener1", "target": {"machine": "listener", "state": "approved"}},
            {"kind": "synchronizationWithinBounds", "byMs": 30, "node": "listener1", "maxAbsOffsetMs": 1000.0, "maxRoundTripMs": 1000.0}
        ],
        "timeoutMs": 30
    }"#;
    let scenario = load_scenario_json(document).expect("fault-mutation scenario should parse");
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let (report, trace) = run_scenario_with_trace(&lab, &scenario)
        .expect("fault-mutation scenario should execute deterministically");

    assert_eq!(report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(report.assertion_results[0].outcome, AssertionOutcome::Held);
    assert_eq!(report.assertion_results[1].outcome, AssertionOutcome::Held);
    assert_eq!(
        synchronization_assertion(&report),
        AssertionOutcome::TimedOut
    );
    assert!(
        report
            .step_results
            .iter()
            .find(|step| step.at_ms == 7)
            .is_some_and(|step| step.submit_error.is_none()),
        "the in-flight setLinkFaults step itself must settle successfully"
    );
    assert!(
        last_listener_round_trip_ms(&trace).is_none(),
        "100% loss applied after connect but before approval must suppress the later sync response"
    );
}

#[test]
fn fault_mutation_rejects_an_undeclared_link_before_execution() {
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1,
            "seed": 1,
            "nodes": [{"id": "host1"}, {"id": "listener1"}],
            "links": [],
            "steps": [{
                "atMs": 1,
                "node": "listener1",
                "action": {
                    "kind": "setLinkFaults",
                    "from": "host1",
                    "to": "listener1",
                    "latencyMs": 1,
                    "jitterMs": 0,
                    "lossPermille": 0
                }
            }],
            "assertions": [],
            "timeoutMs": 10
        }"#,
    )
    .expect("unknown-link mutation is semantic, not a JSON-shape failure");
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let error = run_scenario(&lab, &scenario).expect_err("undeclared link mutation must fail");
    assert!(matches!(
        error,
        ScenarioExecutionError::Validation(ScenarioValidationError::UnknownLink {
            ref from,
            ref to,
        }) if from == "host1" && to == "listener1"
    ));
}

#[test]
fn fault_mutation_rejects_an_ambiguous_multi_inbound_target() {
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1,
            "seed": 2,
            "nodes": [{"id": "host1"}, {"id": "host2"}, {"id": "listener1"}],
            "links": [
                {"from": "host1", "to": "listener1", "latencyMs": 0, "jitterMs": 0, "lossPermille": 0},
                {"from": "host2", "to": "listener1", "latencyMs": 0, "jitterMs": 0, "lossPermille": 0}
            ],
            "steps": [{
                "atMs": 1,
                "node": "listener1",
                "action": {
                    "kind": "setLinkFaults",
                    "from": "host1",
                    "to": "listener1",
                    "latencyMs": 5,
                    "jitterMs": 0,
                    "lossPermille": 0
                }
            }],
            "assertions": [],
            "timeoutMs": 10
        }"#,
    )
    .expect("multi-inbound mutation is semantic, not a JSON-shape failure");
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let error = run_scenario(&lab, &scenario).expect_err("ambiguous link mutation must fail");
    assert!(matches!(
        error,
        ScenarioExecutionError::Validation(
            ScenarioValidationError::AmbiguousFaultMutationTarget { ref node }
        ) if node == "listener1"
    ));
}

#[test]
fn fault_mutation_rejects_out_of_bounds_loss_before_execution() {
    let scenario = load_scenario_json(
        br#"{
            "schemaVersion": 1,
            "seed": 3,
            "nodes": [{"id": "host1"}, {"id": "listener1"}],
            "links": [
                {"from": "host1", "to": "listener1", "latencyMs": 0, "jitterMs": 0, "lossPermille": 0}
            ],
            "steps": [{
                "atMs": 1,
                "node": "listener1",
                "action": {
                    "kind": "setLinkFaults",
                    "from": "host1",
                    "to": "listener1",
                    "latencyMs": 0,
                    "jitterMs": 0,
                    "lossPermille": 1001
                }
            }],
            "assertions": [],
            "timeoutMs": 10
        }"#,
    )
    .expect("out-of-bounds mutation is semantic, not a JSON-shape failure");
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let error = run_scenario(&lab, &scenario).expect_err("out-of-bounds fault mutation must fail");
    assert!(matches!(
        error,
        ScenarioExecutionError::Validation(ScenarioValidationError::LinkOutOfBounds {
            field: "lossPermille",
            ..
        })
    ));
}
