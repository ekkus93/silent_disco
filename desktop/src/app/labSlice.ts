import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type {
  DesktopErrorDto,
  LabNodeDto,
  LabRunOutcomeDto,
  LabScenarioSummaryDto,
} from "../core/generated/desktop-bindings";

// Block 37.1 "add frontend build flag derived from backend capability,
// not only JavaScript environment": `available` is set only from the
// backend's own `get_lab_mode_available` command result -- never derived
// from `import.meta.env`/`process.env` or any other JS-only signal, so a
// dev-mode frontend build alone can never show Lab UI a production Rust
// binary did not actually compile in.
export interface LabState {
  // `null` until the backend has answered -- distinct from `false`, so
  // the UI never briefly claims Lab Mode is unavailable before it has
  // actually asked.
  available: boolean | null;
  nowMs: string;
  // Block 42 "running-state command disablement": mirrors the backend's
  // own `LabStateDto.running` -- the frontend never invents its own
  // optimistic "running" flag ahead of a real command response.
  running: boolean;
  nodes: LabNodeDto[];
  loadedScenario: LabScenarioSummaryDto | null;
  scenarioError: DesktopErrorDto | null;
  // Bounded to exactly one run (not an array): Block 42's own "bounded
  // history" requirement is satisfied by construction here, not by
  // evicting a longer list down to a cap -- there is never more than one
  // in memory at all. Its own `timeline`/`stepResults`/`assertionResults`
  // are already backend-bounded (`lab_commands.rs`'s
  // `MAX_TIMELINE_ENTRIES_PER_NODE`, `lab::scenario`'s own step/assertion
  // bounds).
  lastRun: LabRunOutcomeDto | null;
  commandError: DesktopErrorDto | null;
  // Frontend-only gate over the "step" (advance virtual time) action --
  // real time only ever moves through an explicit backend `advance` call
  // (spec 29.2 "manual advancement"), so "pause" has nothing to pause on
  // the backend; it exists here to give the operator a genuine, testable,
  // keyboard-operable control that disables further stepping.
  stepPaused: boolean;
}

const initialState: LabState = {
  available: null,
  nowMs: "0",
  running: false,
  nodes: [],
  loadedScenario: null,
  scenarioError: null,
  lastRun: null,
  commandError: null,
  stepPaused: false,
};

export const labSlice = createSlice({
  name: "lab",
  initialState,
  reducers: {
    labModeAvailabilityReceived(state, action: PayloadAction<boolean>) {
      state.available = action.payload;
    },
    labStateReceived(
      state,
      action: PayloadAction<{
        nowMs: string;
        running: boolean;
        nodes: LabNodeDto[];
        loadedScenario: LabScenarioSummaryDto | null;
        lastRun: LabRunOutcomeDto | null;
      }>,
    ) {
      state.nowMs = action.payload.nowMs;
      state.running = action.payload.running;
      state.nodes = action.payload.nodes;
      state.loadedScenario = action.payload.loadedScenario;
      state.lastRun = action.payload.lastRun;
    },
    scenarioLoaded(state, action: PayloadAction<LabScenarioSummaryDto>) {
      state.loadedScenario = action.payload;
      state.scenarioError = null;
    },
    // Block 42 "invalid scenario display": a malformed/invalid scenario is
    // recorded and shown, never silently dropped.
    scenarioLoadFailed(state, action: PayloadAction<DesktopErrorDto>) {
      state.scenarioError = action.payload;
    },
    commandFailed(state, action: PayloadAction<DesktopErrorDto>) {
      state.commandError = action.payload;
    },
    commandErrorCleared(state) {
      state.commandError = null;
    },
    stepPausedSet(state, action: PayloadAction<boolean>) {
      state.stepPaused = action.payload;
    },
  },
});

export const labActions = labSlice.actions;
export default labSlice.reducer;
