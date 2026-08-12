import "@testing-library/jest-dom/vitest";

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createAppStore } from "../app/store";
import type { LabRunOutcomeDto, LabStateDto } from "../core/generated/desktop-bindings";
import { LabScreen } from "./LabScreen";

const {
  getLabState,
  openLabScenarioFile,
  saveLabScenarioFile,
  runLoadedLabScenario,
  pauseLoadedLabScenario,
  resumeLoadedLabScenario,
  advanceLabVirtualTime,
  startLabNode,
  stopLabNode,
  stopAllLabNodes,
  setLabLinkFaults,
  exportLabRecordingFile,
} = vi.hoisted(() => ({
  getLabState: vi.fn(),
  openLabScenarioFile: vi.fn(),
  saveLabScenarioFile: vi.fn(),
  runLoadedLabScenario: vi.fn(),
  pauseLoadedLabScenario: vi.fn(),
  resumeLoadedLabScenario: vi.fn(),
  advanceLabVirtualTime: vi.fn(),
  startLabNode: vi.fn(),
  stopLabNode: vi.fn(),
  stopAllLabNodes: vi.fn(),
  setLabLinkFaults: vi.fn(),
  exportLabRecordingFile: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return {
    ...actual,
    getLabState,
    openLabScenarioFile,
    saveLabScenarioFile,
    runLoadedLabScenario,
    pauseLoadedLabScenario,
    resumeLoadedLabScenario,
    advanceLabVirtualTime,
    startLabNode,
    stopLabNode,
    stopAllLabNodes,
    setLabLinkFaults,
    exportLabRecordingFile,
  };
});

function emptyState(overrides: Partial<LabStateDto> = {}): LabStateDto {
  return {
    nowMs: "0",
    running: false,
    paused: false,
    nodes: [],
    loadedScenario: null,
    lastRun: null,
    ...overrides,
  };
}

function runOutcome(overrides: Partial<LabRunOutcomeDto> = {}): LabRunOutcomeDto {
  return {
    outcome: "completed",
    finalTimeMs: "1000",
    stepResults: [],
    assertionResults: [],
    timeline: [],
    timelineTruncated: false,
    ...overrides,
  };
}

function renderScreen() {
  const store = createAppStore();
  return {
    store,
    view: render(
      <Provider store={store}>
        <LabScreen />
      </Provider>,
    ),
  };
}

describe("LabScreen", () => {
  beforeEach(() => {
    getLabState.mockReset();
    openLabScenarioFile.mockReset();
    saveLabScenarioFile.mockReset();
    runLoadedLabScenario.mockReset();
    pauseLoadedLabScenario.mockReset();
    resumeLoadedLabScenario.mockReset();
    advanceLabVirtualTime.mockReset();
    startLabNode.mockReset();
    stopLabNode.mockReset();
    stopAllLabNodes.mockReset();
    setLabLinkFaults.mockReset();
    exportLabRecordingFile.mockReset();
    getLabState.mockResolvedValue(emptyState());
    advanceLabVirtualTime.mockResolvedValue("1000");
    pauseLoadedLabScenario.mockResolvedValue(undefined);
    resumeLoadedLabScenario.mockResolvedValue(undefined);
  });

  it("is unmistakably labeled as a developer testing tool", async () => {
    renderScreen();
    expect(await screen.findByRole("alert")).toHaveTextContent(/lab mode/i);
    expect(screen.getByText(/developer testing tool/i)).toBeVisible();
  });

  it("reports an initial Lab state read failure instead of dropping it", async () => {
    getLabState.mockRejectedValue({
      code: "desktop.lab.state_unavailable",
      subsystem: "bridge",
      severity: "error",
      retryable: true,
      message: "Lab state could not be read.",
    });
    renderScreen();

    expect(await screen.findByText("Lab command failed")).toBeVisible();
    expect(screen.getByText("Lab state could not be read.")).toBeVisible();
    expect(screen.getByText("desktop.lab.state_unavailable")).toBeVisible();
  });

  // Block 42 test list: "keyboard control" -- every control is a real
  // <button>/<input>, reachable and activatable without a mouse.
  it("can be operated entirely from the keyboard", async () => {
    const user = userEvent.setup();
    renderScreen();
    await screen.findByRole("alert");

    const stepButton = screen.getByRole("button", { name: "Step" });
    stepButton.focus();
    expect(stepButton).toHaveFocus();
    await user.keyboard("{Enter}");

    await waitFor(() => expect(advanceLabVirtualTime).toHaveBeenCalledWith("1000"));
  });

  // Block 42 test list: "invalid scenario display" -- a malformed/rejected
  // scenario is shown, never silently swallowed.
  it("displays a scenario validation failure instead of swallowing it", async () => {
    openLabScenarioFile.mockRejectedValue({
      code: "desktop.lab.scenario_invalid",
      subsystem: "validation",
      severity: "error",
      retryable: false,
      message: "unknown node 'ghost' referenced at step 0",
    });
    const user = userEvent.setup();
    renderScreen();
    await screen.findByRole("alert");

    await user.click(screen.getByRole("button", { name: /open scenario/i }));

    expect(await screen.findByText("Invalid scenario")).toBeVisible();
    expect(screen.getByText(/unknown node 'ghost'/)).toBeVisible();
  });

  // Block 42 test list: "running-state command disablement" -- a second run
  // cannot be submitted, and stepping is refused, while one is in flight.
  it("disables run and step controls while a scenario is already running", async () => {
    getLabState.mockResolvedValue(
      emptyState({
        running: true,
        loadedScenario: {
          schemaVersion: 1,
          seed: "1",
          nodeIds: ["host1"],
          linkCount: 0,
          fixtureCount: 0,
          stepCount: 1,
          assertionCount: 0,
          timeoutMs: "1000",
          links: [],
        },
      }),
    );
    renderScreen();

    expect(await screen.findByRole("button", { name: "Running…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Step" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause scenario" })).toBeEnabled();
  });

  it("pauses and resumes an in-flight scenario through backend commands", async () => {
    const loadedScenario = {
      schemaVersion: 1,
      seed: "1",
      nodeIds: ["host1"],
      linkCount: 0,
      fixtureCount: 0,
      stepCount: 2,
      assertionCount: 0,
      timeoutMs: "1000",
      links: [],
    };
    getLabState
      .mockResolvedValueOnce(emptyState({ running: true, paused: false, loadedScenario }))
      .mockResolvedValueOnce(emptyState({ running: true, paused: true, loadedScenario }))
      .mockResolvedValueOnce(emptyState({ running: true, paused: false, loadedScenario }));
    const user = userEvent.setup();
    renderScreen();

    const pauseButton = await screen.findByRole("button", { name: "Pause scenario" });
    await user.click(pauseButton);
    await waitFor(() => expect(pauseLoadedLabScenario).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("button", { name: "Resume scenario" })).toBeEnabled();
    expect(screen.getByText(/pause accepted.*no later step will begin/i)).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Resume scenario" }));
    await waitFor(() => expect(resumeLoadedLabScenario).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("button", { name: "Pause scenario" })).toBeEnabled();
  });

  it("keeps Stop all available while a scenario is running", async () => {
    getLabState.mockResolvedValue(emptyState({ running: true, nodes: [] }));
    const user = userEvent.setup();
    renderScreen();

    const stop = await screen.findByRole("button", { name: "Stop all" });
    expect(stop).toBeEnabled();
    await user.click(stop);
    await waitFor(() => expect(stopAllLabNodes).toHaveBeenCalledTimes(1));
  });

  // Block 42 test list: "deterministic timeline rendering" -- entries
  // render in exactly the backend's own order, not re-sorted or shuffled.
  it("renders the event timeline in the exact order the backend reported it", async () => {
    getLabState.mockResolvedValue(
      emptyState({
        lastRun: runOutcome({
          timeline: [
            { node: "host1", sequence: "0", kind: "snapshot", summary: "first" },
            { node: "host1", sequence: "1", kind: "effect", summary: "second" },
            { node: "listener1", sequence: "0", kind: "error", summary: "third" },
          ],
        }),
      }),
    );
    renderScreen();

    // Waits for the real, asynchronously fetched timeline entries to land
    // before querying -- the "no active nodes" placeholder list item is
    // present on the very first synchronous render and would otherwise let
    // `findAllByRole` resolve too early, before `getLabState` settles.
    await screen.findByText("third");
    const items = screen.getAllByRole("listitem");
    const timelineItems = items.filter((item) =>
      ["first", "second", "third"].some((text) => item.textContent?.includes(text)),
    );
    expect(timelineItems.map((item) => item.textContent)).toEqual([
      expect.stringContaining("first"),
      expect.stringContaining("second"),
      expect.stringContaining("third"),
    ]);
  });

  // Block 42 test list: "bounded history" -- only the single most recent
  // run is ever rendered, never an accumulating list across runs.
  it("shows only the most recent run, never accumulating run history", async () => {
    const loadedScenario = {
      schemaVersion: 1,
      seed: "1",
      nodeIds: ["host1"],
      linkCount: 0,
      fixtureCount: 0,
      stepCount: 0,
      assertionCount: 0,
      timeoutMs: "1000",
      links: [],
    };
    getLabState
      .mockResolvedValueOnce(emptyState({ loadedScenario }))
      .mockResolvedValueOnce(
        emptyState({ loadedScenario, lastRun: runOutcome({ finalTimeMs: "111" }) }),
      )
      .mockResolvedValueOnce(
        emptyState({ loadedScenario, lastRun: runOutcome({ finalTimeMs: "222" }) }),
      );
    runLoadedLabScenario.mockResolvedValue(runOutcome());
    const user = userEvent.setup();
    renderScreen();

    const runButton = await screen.findByRole("button", { name: "Run scenario" });
    await user.click(runButton);
    await waitFor(() =>
      expect(screen.getByTestId("lab-last-run-outcome")).toHaveTextContent("at 111 ms"),
    );

    await user.click(screen.getByRole("button", { name: "Run scenario" }));
    await waitFor(() =>
      expect(screen.getByTestId("lab-last-run-outcome")).toHaveTextContent("at 222 ms"),
    );

    expect(screen.getAllByTestId("lab-last-run-outcome")).toHaveLength(1);
  });

  it("edits a loaded link fault profile through the backend command", async () => {
    const initialScenario = {
      schemaVersion: 1,
      seed: "9",
      nodeIds: ["host", "listener"],
      linkCount: 1,
      fixtureCount: 0,
      stepCount: 0,
      assertionCount: 0,
      timeoutMs: "1000",
      links: [
        { from: "host", to: "listener", latencyMs: "30", jitterMs: "8", lossPermille: 10 },
      ],
    };
    const editedScenario = {
      ...initialScenario,
      links: [
        {
          from: "host",
          to: "listener",
          latencyMs: "125",
          jitterMs: "12",
          lossPermille: 25,
        },
      ],
    };
    getLabState
      .mockResolvedValueOnce(emptyState({ loadedScenario: initialScenario }))
      .mockResolvedValueOnce(emptyState({ loadedScenario: editedScenario }));
    setLabLinkFaults.mockResolvedValue(editedScenario);
    const user = userEvent.setup();
    renderScreen();

    const latency = await screen.findByRole("spinbutton", {
      name: "Latency for host → listener",
    });
    const jitter = screen.getByRole("spinbutton", { name: "Jitter for host → listener" });
    const loss = screen.getByRole("spinbutton", {
      name: "Loss permille for host → listener",
    });
    await user.clear(latency);
    await user.type(latency, "125");
    await user.clear(jitter);
    await user.type(jitter, "12");
    await user.clear(loss);
    await user.type(loss, "25");
    await user.click(screen.getByRole("button", { name: "Apply faults for host → listener" }));

    await waitFor(() =>
      expect(setLabLinkFaults).toHaveBeenCalledWith(0, "host", "listener", "125", "12", "25"),
    );
    await waitFor(() => expect(latency).toHaveValue(125));
    expect(screen.queryByText(/links are not yet wired/i)).not.toBeInTheDocument();
    expect(screen.getByText(/used by live Lab transport on the next run/i)).toBeVisible();
  });

  it("disables fault edits while a scenario run owns the transport", async () => {
    getLabState.mockResolvedValue(
      emptyState({
        running: true,
        loadedScenario: {
          schemaVersion: 1,
          seed: "9",
          nodeIds: ["host", "listener"],
          linkCount: 1,
          fixtureCount: 0,
          stepCount: 0,
          assertionCount: 0,
          timeoutMs: "1000",
          links: [
            {
              from: "host",
              to: "listener",
              latencyMs: "30",
              jitterMs: "8",
              lossPermille: 10,
            },
          ],
        },
      }),
    );
    renderScreen();

    expect(
      await screen.findByRole("button", { name: "Apply faults for host → listener" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("spinbutton", { name: "Latency for host → listener" }),
    ).toBeDisabled();
  });
});
