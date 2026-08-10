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
  advanceLabVirtualTime,
  startLabNode,
  stopLabNode,
  stopAllLabNodes,
  exportLabRecordingFile,
} = vi.hoisted(() => ({
  getLabState: vi.fn(),
  openLabScenarioFile: vi.fn(),
  saveLabScenarioFile: vi.fn(),
  runLoadedLabScenario: vi.fn(),
  advanceLabVirtualTime: vi.fn(),
  startLabNode: vi.fn(),
  stopLabNode: vi.fn(),
  stopAllLabNodes: vi.fn(),
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
    advanceLabVirtualTime,
    startLabNode,
    stopLabNode,
    stopAllLabNodes,
    exportLabRecordingFile,
  };
});

function emptyState(overrides: Partial<LabStateDto> = {}): LabStateDto {
  return {
    nowMs: "0",
    running: false,
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
    advanceLabVirtualTime.mockReset();
    startLabNode.mockReset();
    stopLabNode.mockReset();
    stopAllLabNodes.mockReset();
    exportLabRecordingFile.mockReset();
    getLabState.mockResolvedValue(emptyState());
    advanceLabVirtualTime.mockResolvedValue("1000");
  });

  it("is unmistakably labeled as a developer testing tool", async () => {
    renderScreen();
    expect(await screen.findByRole("alert")).toHaveTextContent(/lab mode/i);
    expect(screen.getByText(/developer testing tool/i)).toBeVisible();
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
});
