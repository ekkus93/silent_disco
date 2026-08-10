import { describe, expect, it } from "vitest";

import type { LabRunOutcomeDto, LabScenarioSummaryDto } from "../core/generated/desktop-bindings";
import labReducer, { labActions } from "./labSlice";

const scenario: LabScenarioSummaryDto = {
  schemaVersion: 1,
  seed: "7",
  nodeIds: ["host1"],
  linkCount: 0,
  fixtureCount: 0,
  stepCount: 1,
  assertionCount: 1,
  timeoutMs: "1000",
  links: [],
};

const runOutcome: LabRunOutcomeDto = {
  outcome: "completed",
  finalTimeMs: "1000",
  stepResults: [],
  assertionResults: [],
  timeline: [],
  timelineTruncated: false,
};

describe("labSlice", () => {
  it("starts with availability unknown, not assumed false", () => {
    const state = labReducer(undefined, { type: "@@INIT" });
    expect(state.available).toBeNull();
    expect(state.running).toBe(false);
    expect(state.nodes).toEqual([]);
    expect(state.stepPaused).toBe(false);
  });

  it("records the backend's own availability answer", () => {
    const afterTrue = labReducer(undefined, labActions.labModeAvailabilityReceived(true));
    expect(afterTrue.available).toBe(true);

    const afterFalse = labReducer(afterTrue, labActions.labModeAvailabilityReceived(false));
    expect(afterFalse.available).toBe(false);
  });

  it("replaces node list, virtual time, and running flag from a real state response", () => {
    const state = labReducer(
      undefined,
      labActions.labStateReceived({
        nowMs: "5000",
        running: true,
        nodes: [{ nodeId: "1", offsetMs: "0", driftPpm: "0" }],
        loadedScenario: scenario,
        lastRun: runOutcome,
      }),
    );
    expect(state.nowMs).toBe("5000");
    expect(state.running).toBe(true);
    expect(state.nodes).toHaveLength(1);
    expect(state.loadedScenario).toEqual(scenario);
    expect(state.lastRun).toEqual(runOutcome);
  });

  it("keeps an invalid scenario error visible rather than silently dropping it", () => {
    const error = {
      code: "desktop.lab.scenario_invalid",
      subsystem: "validation",
      severity: "error",
      retryable: false,
      message: "unknown node 'x' referenced",
    };
    const state = labReducer(undefined, labActions.scenarioLoadFailed(error));
    expect(state.scenarioError).toEqual(error);
    expect(state.loadedScenario).toBeNull();
  });

  it("clears a stale scenario error once a scenario loads successfully", () => {
    const error = {
      code: "desktop.lab.scenario_invalid",
      subsystem: "validation",
      severity: "error",
      retryable: false,
      message: "bad scenario",
    };
    const withError = labReducer(undefined, labActions.scenarioLoadFailed(error));
    const recovered = labReducer(withError, labActions.scenarioLoaded(scenario));
    expect(recovered.scenarioError).toBeNull();
    expect(recovered.loadedScenario).toEqual(scenario);
  });

  it("retains only the single most recent run, never an unbounded history", () => {
    const first = labReducer(
      undefined,
      labActions.labStateReceived({
        nowMs: "100",
        running: false,
        nodes: [],
        loadedScenario: null,
        lastRun: runOutcome,
      }),
    );
    const second = labReducer(
      first,
      labActions.labStateReceived({
        nowMs: "200",
        running: false,
        nodes: [],
        loadedScenario: null,
        lastRun: { ...runOutcome, finalTimeMs: "200" },
      }),
    );
    expect(second.lastRun?.finalTimeMs).toBe("200");
  });

  it("toggles the frontend-only step-pause gate", () => {
    const paused = labReducer(undefined, labActions.stepPausedSet(true));
    expect(paused.stepPaused).toBe(true);
    const resumed = labReducer(paused, labActions.stepPausedSet(false));
    expect(resumed.stepPaused).toBe(false);
  });

  it("records and clears a command failure", () => {
    const error = {
      code: "desktop.lab.already_running",
      subsystem: "runtime",
      severity: "error",
      retryable: false,
      message: "already running",
    };
    const failed = labReducer(undefined, labActions.commandFailed(error));
    expect(failed.commandError).toEqual(error);
    const cleared = labReducer(failed, labActions.commandErrorCleared());
    expect(cleared.commandError).toBeNull();
  });
});
