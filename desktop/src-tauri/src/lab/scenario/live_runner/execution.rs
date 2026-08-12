fn execute_steps_and_assertions(
    lab: &LabRuntime,
    scenario: &Scenario,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
    driver: &mut LiveTransportDriver,
    clock_advances: &mut Vec<ClockAdvance>,
    control: &ScenarioRunControl,
) -> Result<ScenarioReport, ScenarioExecutionError> {
    let mut current_ms = lab.now().get();
    let mut step_results = Vec::with_capacity(scenario.steps.len());

    for (index, step) in scenario.steps.iter().enumerate() {
        if step.at_ms >= scenario.timeout_ms {
            break;
        }
        // Pause is honored only between complete scenario steps. The step
        // that is already in flight is allowed to settle atomically; this is
        // the boundary that preserves deterministic step semantics while
        // still giving the operator real control over future progression.
        control.wait_until_runnable()?;
        advance_to(
            lab,
            driver,
            &mut current_ms,
            step.at_ms,
            clock_advances,
            control,
        )?;

        if let super::ScenarioAction::SetLinkFaults {
            from,
            to,
            latency_ms,
            jitter_ms,
            loss_permille,
        } = &step.action
        {
            driver
                .set_link_faults(from, to, *latency_ms, *jitter_ms, *loss_permille)
                .map_err(ScenarioExecutionError::Lab)?;
            driver.pump().map_err(ScenarioExecutionError::Lab)?;
            control.check_stop()?;
            step_results.push(StepResult {
                index,
                at_ms: step.at_ms,
                node: step.node.clone(),
                submit_error: None,
                settlement: StepSettlement::Settled,
            });
            continue;
        }

        let lab_node_id = node_id_for(step.node.as_str(), &step.node, lab_node_ids)?;
        let handle = node_handle(lab, lab_node_id)?;
        let recorder = recorders
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let revision_before = current_revision(&handle)?;
        let sequence_before = recorder.next_sequence();
        let submit_error = submit_action(
            lab,
            scenario,
            &handle,
            lab_node_id,
            lab_node_ids,
            &step.action,
        )?;
        driver.pump().map_err(ScenarioExecutionError::Lab)?;

        let settlement = if submit_error.is_some() {
            StepSettlement::Settled
        } else {
            wait_for_step_settled(
                &handle,
                recorder,
                driver,
                revision_before,
                sequence_before,
                action_revision_delta(&step.action),
                control,
            )?
        };
        step_results.push(StepResult {
            index,
            at_ms: step.at_ms,
            node: step.node.clone(),
            submit_error,
            settlement,
        });
    }

    control.wait_until_runnable()?;
    advance_to(
        lab,
        driver,
        &mut current_ms,
        scenario.timeout_ms,
        clock_advances,
        control,
    )?;
    control.check_stop()?;
    let (outcome, assertion_results) = evaluate_assertions(lab, scenario, lab_node_ids, recorders)?;

    Ok(ScenarioReport {
        schema_version: scenario.schema_version,
        seed: scenario.seed,
        outcome,
        final_time_ms: current_ms,
        step_results,
        assertion_results,
    })
}

fn advance_to(
    lab: &LabRuntime,
    driver: &mut LiveTransportDriver,
    current_ms: &mut u64,
    target_ms: u64,
    clock_advances: &mut Vec<ClockAdvance>,
    control: &ScenarioRunControl,
) -> Result<(), ScenarioExecutionError> {
    control.check_stop()?;
    if target_ms <= *current_ms {
        driver.pump().map_err(ScenarioExecutionError::Lab)?;
        control.check_stop()?;
        return Ok(());
    }
    let delta = target_ms - *current_ms;
    let resulting = lab.advance(delta).map_err(ScenarioExecutionError::Lab)?;
    *current_ms = resulting.get();
    clock_advances.push(ClockAdvance {
        requested_delta_ms: delta,
        resulting_now_ms: *current_ms,
    });
    driver.pump().map_err(ScenarioExecutionError::Lab)?;
    control.check_stop()?;
    Ok(())
}

fn wait_for_step_settled(
    handle: &CoreActorHandle,
    recorder: &ScenarioRecorder,
    driver: &mut LiveTransportDriver,
    revision_before: SnapshotRevision,
    sequence_before: u64,
    minimum_revision_delta: u64,
    control: &ScenarioRunControl,
) -> Result<StepSettlement, ScenarioExecutionError> {
    let target_revision = revision_before.get().saturating_add(minimum_revision_delta);
    let mut remaining = STEP_SETTLE_TIMEOUT;
    loop {
        control.check_stop()?;
        driver.pump().map_err(ScenarioExecutionError::Lab)?;
        let snapshot = handle
            .current_snapshot()
            .map_err(|error| ScenarioExecutionError::Lab(error.into()))?;
        if snapshot.revision.get() >= target_revision {
            return Ok(StepSettlement::Settled);
        }
        if recorder.entries().iter().any(|entry| {
            entry.sequence >= sequence_before
                && matches!(entry.kind, RecordedNotificationKind::Error { .. })
        }) {
            return Ok(StepSettlement::Settled);
        }
        if remaining.is_zero() {
            return Ok(StepSettlement::TimedOut);
        }
        let chunk = remaining.min(STEP_SETTLE_POLL);
        remaining -= chunk;
        recorder.wait_for_progress(sequence_before, chunk);
    }
}

fn evaluate_assertions(
    lab: &LabRuntime,
    scenario: &Scenario,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
) -> Result<(ScenarioOutcome, Vec<AssertionResult>), ScenarioExecutionError> {
    let mut outcome = ScenarioOutcome::Completed;
    let mut results = Vec::with_capacity(scenario.assertions.len());
    for assertion in &scenario.assertions {
        let node = assertion_node(assertion);
        let lab_node_id = node_id_for(node.as_str(), node, lab_node_ids)?;
        let handle = node_handle(lab, lab_node_id)?;
        let recorder = recorders
            .get(node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))?;
        let snapshot = handle
            .current_snapshot()
            .map_err(|error| ScenarioExecutionError::Lab(error.into()))?;
        let held = evaluate_assertion(assertion, Some(&snapshot), &recorder.entries());
        let assertion_outcome = if held {
            AssertionOutcome::Held
        } else {
            outcome = ScenarioOutcome::TimedOut;
            AssertionOutcome::TimedOut
        };
        results.push(AssertionResult {
            kind: assertion_kind(assertion).to_owned(),
            node: node.clone(),
            by_ms: assertion_deadline(assertion),
            outcome: assertion_outcome,
        });
        if assertion_outcome != AssertionOutcome::Held
            && scenario.termination.stop_on_assertion_failure
        {
            break;
        }
    }
    Ok((outcome, results))
}

fn node_id_for(
    key: &str,
    node: &NodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
) -> Result<LabNodeId, ScenarioExecutionError> {
    lab_node_ids
        .get(key)
        .copied()
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))
}

fn node_handle(
    lab: &LabRuntime,
    node_id: LabNodeId,
) -> Result<CoreActorHandle, ScenarioExecutionError> {
    scenario_node_parts(lab, node_id)
        .map(|(handle, _identity, _clock)| handle)
        .map_err(ScenarioExecutionError::Lab)
}

fn collect_notifications(
    scenario: &Scenario,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
) -> Vec<(String, Vec<RecordedNotification>)> {
    scenario
        .nodes
        .iter()
        .filter_map(|node| {
            recorders
                .get(node.id.as_str())
                .map(|recorder| (node.id.to_string(), recorder.entries()))
        })
        .collect()
}

fn stop_scenario_nodes(
    lab: &LabRuntime,
    lab_node_ids: &HashMap<&str, LabNodeId>,
) -> Result<(), DesktopErrorDto> {
    let mut node_ids: Vec<LabNodeId> = lab_node_ids.values().copied().collect();
    node_ids.sort_unstable();
    let mut failure: Option<DesktopErrorDto> = None;
    for lab_node_id in node_ids {
        if let Err(error) = lab.stop_node(lab_node_id) {
            failure = Some(match failure {
                Some(previous) => previous.with_appended_cleanup(Some(error)),
                None => error,
            });
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
