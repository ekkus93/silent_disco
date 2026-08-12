/// The first point at which a recorded [`ScenarioReport`] and a freshly
/// replayed one disagree (Block 41.2 "detect divergence at the first
/// meaningful event", "produce bounded diff").
///
/// Compared in the scenario's own chronological order -- every step result
/// (in submission order), then every assertion result (evaluated after all
/// steps, in declaration order) -- so "first" here means "first thing that
/// actually happened differently", not an arbitrary field ordering.
/// Deliberately a single value, not a list: Block 40 already proved
/// [`ScenarioReport`] is genuinely deterministic for the same scenario and
/// seed (`scenario::tests::identical_scenario_and_seed_produce_a_deterministic_report`),
/// so once one point diverges, every result downstream of it is expected to
/// diverge too and reporting all of them would mostly restate the same
/// root cause.
/// Deliberately varied naming (not a uniform `*Changed`/`*Mismatch` suffix
/// on every variant) to keep `clippy::enum_variant_names` from firing on
/// what would otherwise read as a repeated, redundant postfix.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Divergence {
    DifferentStepCount {
        recorded: usize,
        replayed: usize,
    },
    StepResultMismatch {
        index: usize,
        recorded: StepResult,
        replayed: StepResult,
    },
    DifferentAssertionCount {
        recorded: usize,
        replayed: usize,
    },
    AssertionResultMismatch {
        index: usize,
        recorded: AssertionResult,
        replayed: AssertionResult,
    },
    ClockAdvanceMismatch {
        index: usize,
        recorded: ClockAdvance,
        replayed: ClockAdvance,
    },
    DifferentClockAdvanceCount {
        recorded: usize,
        replayed: usize,
    },
    DifferentNotificationNodeCount {
        recorded: usize,
        replayed: usize,
    },
    NotificationNodeMismatch {
        index: usize,
        recorded: String,
        replayed: String,
    },
    DifferentNotificationCount {
        node: String,
        recorded: usize,
        replayed: usize,
    },
    NotificationMismatch {
        node: String,
        index: usize,
        recorded: Box<RecordedNotification>,
        replayed: Box<RecordedNotification>,
    },
    TransportFactMismatch {
        index: usize,
        recorded: Box<TransportFact>,
        replayed: Box<TransportFact>,
    },
    DifferentTransportFactCount {
        recorded: usize,
        replayed: usize,
    },
    TransportOverflowMismatch {
        recorded: u64,
        replayed: u64,
    },
    DifferentOutcome {
        recorded: ScenarioOutcome,
        replayed: ScenarioOutcome,
    },
}

impl fmt::Display for Divergence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DifferentStepCount { recorded, replayed } => write!(
                formatter,
                "step count changed: recorded {recorded} step result(s), replay produced {replayed}"
            ),
            Self::StepResultMismatch {
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "step {index} diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
            Self::DifferentAssertionCount { recorded, replayed } => write!(
                formatter,
                "assertion count changed: recorded {recorded} assertion result(s), replay \
                 produced {replayed}"
            ),
            Self::AssertionResultMismatch {
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "assertion {index} diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
            Self::ClockAdvanceMismatch {
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "clock advance {index} diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
            Self::DifferentClockAdvanceCount { recorded, replayed } => write!(
                formatter,
                "clock-advance count changed: recorded {recorded}, replay produced {replayed}"
            ),
            Self::DifferentNotificationNodeCount { recorded, replayed } => write!(
                formatter,
                "notification-node count changed: recorded {recorded}, replay produced {replayed}"
            ),
            Self::NotificationNodeMismatch {
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "notification node {index} diverged: recorded {recorded}, replay produced {replayed}"
            ),
            Self::DifferentNotificationCount {
                node,
                recorded,
                replayed,
            } => write!(
                formatter,
                "notification count for {node} changed: recorded {recorded}, replay produced {replayed}"
            ),
            Self::NotificationMismatch {
                node,
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "notification {index} for {node} diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
            Self::TransportFactMismatch {
                index,
                recorded,
                replayed,
            } => write!(
                formatter,
                "transport fact {index} diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
            Self::DifferentTransportFactCount { recorded, replayed } => write!(
                formatter,
                "transport fact count changed: recorded {recorded}, replay produced {replayed}"
            ),
            Self::TransportOverflowMismatch { recorded, replayed } => write!(
                formatter,
                "transport trace overflow count changed: recorded {recorded}, replay produced {replayed}"
            ),
            Self::DifferentOutcome { recorded, replayed } => write!(
                formatter,
                "overall outcome diverged: recorded {recorded:?}, replay produced {replayed:?}"
            ),
        }
    }
}

/// Finds the first [`Divergence`] between a recorded report and a freshly
/// replayed one, or `None` when they match exactly.
#[must_use]
pub(crate) fn first_divergence(
    recorded: &ScenarioReport,
    replayed: &ScenarioReport,
) -> Option<Divergence> {
    for (index, (recorded_step, replayed_step)) in recorded
        .step_results
        .iter()
        .zip(replayed.step_results.iter())
        .enumerate()
    {
        if recorded_step != replayed_step {
            return Some(Divergence::StepResultMismatch {
                index,
                recorded: recorded_step.clone(),
                replayed: replayed_step.clone(),
            });
        }
    }
    if recorded.step_results.len() != replayed.step_results.len() {
        return Some(Divergence::DifferentStepCount {
            recorded: recorded.step_results.len(),
            replayed: replayed.step_results.len(),
        });
    }

    for (index, (recorded_assertion, replayed_assertion)) in recorded
        .assertion_results
        .iter()
        .zip(replayed.assertion_results.iter())
        .enumerate()
    {
        if recorded_assertion != replayed_assertion {
            return Some(Divergence::AssertionResultMismatch {
                index,
                recorded: recorded_assertion.clone(),
                replayed: replayed_assertion.clone(),
            });
        }
    }
    if recorded.assertion_results.len() != replayed.assertion_results.len() {
        return Some(Divergence::DifferentAssertionCount {
            recorded: recorded.assertion_results.len(),
            replayed: replayed.assertion_results.len(),
        });
    }

    if recorded.outcome != replayed.outcome {
        return Some(Divergence::DifferentOutcome {
            recorded: recorded.outcome.clone(),
            replayed: replayed.outcome.clone(),
        });
    }

    None
}

/// Finds the first mismatch in the persisted deterministic trace after the
/// semantic [`ScenarioReport`] itself matched. Each source is compared in its
/// persisted deterministic order and only the first mismatch is returned.
#[must_use]
pub(crate) fn first_trace_divergence(
    recorded: &ScenarioTrace,
    replayed: &ScenarioTrace,
) -> Option<Divergence> {
    for (index, (recorded_advance, replayed_advance)) in recorded
        .clock_advances
        .iter()
        .zip(replayed.clock_advances.iter())
        .enumerate()
    {
        if recorded_advance != replayed_advance {
            return Some(Divergence::ClockAdvanceMismatch {
                index,
                recorded: *recorded_advance,
                replayed: *replayed_advance,
            });
        }
    }
    if recorded.clock_advances.len() != replayed.clock_advances.len() {
        return Some(Divergence::DifferentClockAdvanceCount {
            recorded: recorded.clock_advances.len(),
            replayed: replayed.clock_advances.len(),
        });
    }

    if recorded.node_notifications.len() != replayed.node_notifications.len() {
        return Some(Divergence::DifferentNotificationNodeCount {
            recorded: recorded.node_notifications.len(),
            replayed: replayed.node_notifications.len(),
        });
    }
    for (node_index, ((recorded_node, recorded_entries), (replayed_node, replayed_entries))) in
        recorded
            .node_notifications
            .iter()
            .zip(replayed.node_notifications.iter())
            .enumerate()
    {
        if recorded_node != replayed_node {
            return Some(Divergence::NotificationNodeMismatch {
                index: node_index,
                recorded: recorded_node.clone(),
                replayed: replayed_node.clone(),
            });
        }
        for (index, (recorded_entry, replayed_entry)) in recorded_entries
            .iter()
            .zip(replayed_entries.iter())
            .enumerate()
        {
            if recorded_entry != replayed_entry {
                return Some(Divergence::NotificationMismatch {
                    node: recorded_node.clone(),
                    index,
                    recorded: Box::new(recorded_entry.clone()),
                    replayed: Box::new(replayed_entry.clone()),
                });
            }
        }
        if recorded_entries.len() != replayed_entries.len() {
            return Some(Divergence::DifferentNotificationCount {
                node: recorded_node.clone(),
                recorded: recorded_entries.len(),
                replayed: replayed_entries.len(),
            });
        }
    }

    first_transport_divergence(recorded, replayed)
}

fn first_transport_divergence(
    recorded: &ScenarioTrace,
    replayed: &ScenarioTrace,
) -> Option<Divergence> {
    for (index, (recorded_fact, replayed_fact)) in recorded
        .transport_trace
        .facts
        .iter()
        .zip(replayed.transport_trace.facts.iter())
        .enumerate()
    {
        if recorded_fact != replayed_fact {
            return Some(Divergence::TransportFactMismatch {
                index,
                recorded: Box::new(recorded_fact.clone()),
                replayed: Box::new(replayed_fact.clone()),
            });
        }
    }
    if recorded.transport_trace.facts.len() != replayed.transport_trace.facts.len() {
        return Some(Divergence::DifferentTransportFactCount {
            recorded: recorded.transport_trace.facts.len(),
            replayed: replayed.transport_trace.facts.len(),
        });
    }
    if recorded.transport_trace.dropped_count != replayed.transport_trace.dropped_count {
        return Some(Divergence::TransportOverflowMismatch {
            recorded: recorded.transport_trace.dropped_count,
            replayed: replayed.transport_trace.dropped_count,
        });
    }
    None
}
