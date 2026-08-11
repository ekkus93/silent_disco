use super::types::{
    MAX_ASSERTIONS, MAX_ERROR_CODE_BYTES, MAX_FIXTURES, MAX_LINKS, MAX_LINK_JITTER_MS,
    MAX_LINK_LATENCY_MS, MAX_LOSS_PERMILLE, MAX_NODES, MAX_SCENARIO_DURATION_MS, MAX_STEPS,
    Scenario, ScenarioAction, ScenarioAssertion, ScenarioValidationError,
};
use std::collections::{HashMap, HashSet};

impl Scenario {
    pub(crate) fn validate(&self) -> Result<(), ScenarioValidationError> {
        self.validate_bounds()?;
        let known_nodes = self.known_node_ids()?;
        let known_fixtures: HashSet<&str> = self
            .fixtures
            .iter()
            .map(|fixture| fixture.id.as_str())
            .collect();
        self.validate_links(&known_nodes)?;
        self.validate_clocks(&known_nodes)?;
        self.validate_steps(&known_nodes, &known_fixtures)?;
        self.validate_assertions(&known_nodes)
    }

    fn validate_bounds(&self) -> Result<(), ScenarioValidationError> {
        for (field, actual, limit) in [
            ("nodes", self.nodes.len(), MAX_NODES),
            ("links", self.links.len(), MAX_LINKS),
            ("fixtures", self.fixtures.len(), MAX_FIXTURES),
            ("steps", self.steps.len(), MAX_STEPS),
            ("assertions", self.assertions.len(), MAX_ASSERTIONS),
        ] {
            if actual > limit {
                return Err(ScenarioValidationError::TooMany { field, limit });
            }
        }
        if self.timeout_ms > MAX_SCENARIO_DURATION_MS {
            return Err(ScenarioValidationError::DurationOutOfBounds {
                field: "timeoutMs",
                limit: MAX_SCENARIO_DURATION_MS,
            });
        }
        Ok(())
    }

    fn known_node_ids(&self) -> Result<HashSet<&str>, ScenarioValidationError> {
        let mut known_nodes = HashSet::new();
        for node in &self.nodes {
            if !known_nodes.insert(node.id.as_str()) {
                return Err(ScenarioValidationError::DuplicateNodeId(
                    node.id.to_string(),
                ));
            }
        }
        Ok(known_nodes)
    }

    fn validate_links(&self, known_nodes: &HashSet<&str>) -> Result<(), ScenarioValidationError> {
        let mut incoming: HashMap<&str, (u64, u64, u16)> = HashMap::new();
        for link in &self.links {
            for (field, node) in [("links[].from", &link.from), ("links[].to", &link.to)] {
                if !known_nodes.contains(node.as_str()) {
                    return Err(ScenarioValidationError::UnknownNode {
                        field,
                        node: node.to_string(),
                    });
                }
            }
            for (field, actual, limit) in [
                ("latencyMs", link.latency_ms, MAX_LINK_LATENCY_MS),
                ("jitterMs", link.jitter_ms, MAX_LINK_JITTER_MS),
                (
                    "lossPermille",
                    u64::from(link.loss_permille),
                    u64::from(MAX_LOSS_PERMILLE),
                ),
            ] {
                if actual > limit {
                    return Err(ScenarioValidationError::LinkOutOfBounds { field, limit });
                }
            }
            let profile = (link.latency_ms, link.jitter_ms, link.loss_permille);
            if let Some(existing) = incoming.insert(link.to.as_str(), profile)
                && existing != profile
            {
                return Err(ScenarioValidationError::AmbiguousInboundLinkFaults {
                    node: link.to.to_string(),
                });
            }
        }
        Ok(())
    }

    fn validate_clocks(&self, known_nodes: &HashSet<&str>) -> Result<(), ScenarioValidationError> {
        for node in self.clocks.keys() {
            if !known_nodes.contains(node.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "clocks",
                    node: node.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_steps(
        &self,
        known_nodes: &HashSet<&str>,
        known_fixtures: &HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        let mut last_at_ms = 0;
        for (index, step) in self.steps.iter().enumerate() {
            if step.at_ms > MAX_SCENARIO_DURATION_MS {
                return Err(ScenarioValidationError::DurationOutOfBounds {
                    field: "steps[].atMs",
                    limit: MAX_SCENARIO_DURATION_MS,
                });
            }
            if index > 0 && step.at_ms < last_at_ms {
                return Err(ScenarioValidationError::StepsNotTimeOrdered { index });
            }
            last_at_ms = step.at_ms;
            if !known_nodes.contains(step.node.as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "steps[].node",
                    node: step.node.to_string(),
                });
            }
            if let Some(fixture) = match &step.action {
                ScenarioAction::ConfigureHost { fixture, .. }
                | ScenarioAction::StartPlayback { fixture } => Some(fixture),
                _ => None,
            } && !known_fixtures.contains(fixture.as_str())
            {
                return Err(ScenarioValidationError::UnknownFixture {
                    field: "steps[].action.fixture",
                    fixture: fixture.to_string(),
                });
            }
            if let ScenarioAction::RemoveListener { listener_node } = &step.action
                && !known_nodes.contains(listener_node.as_str())
            {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "steps[].action.listenerNode",
                    node: listener_node.to_string(),
                });
            }
        }
        Ok(())
    }

    fn validate_assertions(
        &self,
        known_nodes: &HashSet<&str>,
    ) -> Result<(), ScenarioValidationError> {
        for assertion in &self.assertions {
            if assertion.by_ms() > MAX_SCENARIO_DURATION_MS {
                return Err(ScenarioValidationError::DurationOutOfBounds {
                    field: "assertions[].byMs",
                    limit: MAX_SCENARIO_DURATION_MS,
                });
            }
            if !known_nodes.contains(assertion.node().as_str()) {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "assertions[].node",
                    node: assertion.node().to_string(),
                });
            }
            if let ScenarioAssertion::ErrorCodeObserved { code, .. } = assertion
                && (code.is_empty() || code.len() > MAX_ERROR_CODE_BYTES)
            {
                return Err(ScenarioValidationError::InvalidToken {
                    kind: "error code",
                    reason: "blank or oversized",
                });
            }
        }
        Ok(())
    }
}
