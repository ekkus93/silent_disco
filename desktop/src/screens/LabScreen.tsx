import { useCallback, useEffect, useState } from "react";

import { labActions } from "../app/labSlice";
import { selectLabState } from "../app/selectors";
import { useAppDispatch, useAppSelector } from "../app/store";
import {
  advanceLabVirtualTime,
  exportLabRecordingFile,
  getLabState,
  openLabScenarioFile,
  runLoadedLabScenario,
  saveLabScenarioFile,
  startLabNode,
  stopAllLabNodes,
  stopLabNode,
} from "../core/client";
import type { DesktopErrorDto } from "../core/generated/desktop-bindings";

const DEFAULT_STEP_DELTA_MS = "1000";

function isDesktopErrorDto(value: unknown): value is DesktopErrorDto {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as { code: unknown }).code === "string"
  );
}

/**
 * Block 42 "Build Lab Mode UI". Clearly and unmistakably labeled as a
 * developer/testing tool throughout (amber "LAB MODE" banner, matching the
 * amber badge convention `App.tsx` already established for a Lab-Mode
 * build) so it can never be confused with real session UI.
 *
 * This screen never mutates node domain state directly -- every control
 * below submits a scenario/test command to `LabRuntime` through
 * `core/client.ts`'s typed IPC wrappers and only ever renders what the
 * backend reports back (Block 42's own "UI must not mutate node domain
 * state directly" rule).
 */
export function LabScreen() {
  const dispatch = useAppDispatch();
  const lab = useAppSelector(selectLabState);
  const [offsetMs, setOffsetMs] = useState("0");
  const [driftPpm, setDriftPpm] = useState("0");
  const [stepDeltaMs, setStepDeltaMs] = useState(DEFAULT_STEP_DELTA_MS);
  const [busy, setBusy] = useState(false);

  const refreshState = useCallback(async () => {
    const state = await getLabState();
    dispatch(
      labActions.labStateReceived({
        nowMs: state.nowMs,
        running: state.running,
        nodes: state.nodes,
        loadedScenario: state.loadedScenario,
        lastRun: state.lastRun,
      }),
    );
  }, [dispatch]);

  useEffect(() => {
    void refreshState();
  }, [refreshState]);

  const reportFailure = useCallback(
    (error: unknown) => {
      if (isDesktopErrorDto(error)) {
        dispatch(labActions.commandFailed(error));
        return;
      }
      dispatch(
        labActions.commandFailed({
          code: "desktop.lab.unknown_frontend_failure",
          subsystem: "bridge",
          severity: "error",
          retryable: true,
          message: error instanceof Error ? error.message : "Lab Mode command failed.",
        }),
      );
    },
    [dispatch],
  );

  const runGuarded = useCallback(
    async (action: () => Promise<void>) => {
      setBusy(true);
      dispatch(labActions.commandErrorCleared());
      try {
        await action();
        await refreshState();
      } catch (error: unknown) {
        reportFailure(error);
      } finally {
        setBusy(false);
      }
    },
    [dispatch, reportFailure, refreshState],
  );

  const handleOpenScenario = useCallback(() => {
    void runGuarded(async () => {
      try {
        const summary = await openLabScenarioFile();
        if (summary) {
          dispatch(labActions.scenarioLoaded(summary));
        }
      } catch (error: unknown) {
        if (isDesktopErrorDto(error)) {
          dispatch(labActions.scenarioLoadFailed(error));
          return;
        }
        throw error;
      }
    });
  }, [dispatch, runGuarded]);

  const handleSaveScenario = useCallback(() => {
    void runGuarded(async () => {
      await saveLabScenarioFile();
    });
  }, [runGuarded]);

  // Block 42 "start": runs the loaded scenario to completion. Disabled
  // while `lab.running` is true (real, backend-reported state) or nothing
  // is loaded -- see `lab_commands.rs`'s own module doc comment for why
  // this, not an interior interruption, is what "start" honestly runs.
  const handleRunScenario = useCallback(() => {
    void runGuarded(async () => {
      await runLoadedLabScenario();
    });
  }, [runGuarded]);

  // Block 42 "step": the only way virtual time ever advances (spec 29.2
  // "manual advancement"). Gated by the frontend-only `stepPaused` toggle
  // ("pause") in addition to the backend's own `running` guard.
  const handleStep = useCallback(() => {
    void runGuarded(async () => {
      await advanceLabVirtualTime(stepDeltaMs);
    });
  }, [runGuarded, stepDeltaMs]);

  const handleTogglePause = useCallback(() => {
    dispatch(labActions.stepPausedSet(!lab.stepPaused));
  }, [dispatch, lab.stepPaused]);

  const handleStartNode = useCallback(() => {
    void runGuarded(async () => {
      await startLabNode(offsetMs, driftPpm);
    });
  }, [runGuarded, offsetMs, driftPpm]);

  const handleStopNode = useCallback(
    (nodeId: string) => {
      void runGuarded(async () => {
        await stopLabNode(nodeId);
      });
    },
    [runGuarded],
  );

  // Block 42 "stop": tears down every currently active Lab node.
  const handleStopAll = useCallback(() => {
    void runGuarded(async () => {
      await stopAllLabNodes();
    });
  }, [runGuarded]);

  const handleExportRecording = useCallback(() => {
    void runGuarded(async () => {
      await exportLabRecordingFile();
    });
  }, [runGuarded]);

  const canStep = !lab.running && !lab.stepPaused;
  const canRun = !lab.running && lab.loadedScenario !== null;

  return (
    <section aria-label="Lab Mode" className="mt-6 space-y-6">
      <div
        role="alert"
        className="rounded-xl border-2 border-amber-400/70 bg-amber-950/40 px-4 py-3 text-amber-100"
      >
        <p className="text-sm font-bold uppercase tracking-[0.2em]">Lab Mode</p>
        <p className="mt-1 text-sm text-amber-100/80">
          Developer testing tool. Multi-node scenarios run against isolated, synthetic Lab nodes --
          never a real listening session or real device identity.
        </p>
      </div>

      {lab.commandError ? (
        <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
          <p className="font-semibold">Lab command failed</p>
          <p className="mt-2 text-sm">{lab.commandError.message}</p>
          <p className="mt-1 font-mono text-xs text-red-100/60">{lab.commandError.code}</p>
        </div>
      ) : null}

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <div className="rounded-xl border border-violet-300/20 bg-slate-950/50 p-4">
          <h2 className="text-lg font-semibold">Virtual time</h2>
          <p className="mt-1 font-mono text-2xl text-cyan-200">{lab.nowMs} ms</p>
          <p className="mt-1 text-xs text-violet-100/60">
            {lab.running ? "A scenario run is in progress." : "Idle."}
          </p>
          <div className="mt-3 flex flex-wrap items-end gap-3">
            <label className="flex flex-col text-xs text-violet-100/70" htmlFor="lab-step-delta">
              Step by (ms)
              <input
                id="lab-step-delta"
                type="number"
                min={0}
                value={stepDeltaMs}
                onChange={(event) => setStepDeltaMs(event.target.value)}
                className="mt-1 w-32 rounded-md border border-violet-300/30 bg-slate-900 px-2 py-1 text-sm text-violet-50"
              />
            </label>
            <button
              type="button"
              onClick={handleStep}
              disabled={busy || !canStep}
              className="rounded-lg border border-cyan-300/50 px-4 py-2 text-sm font-semibold text-cyan-100 hover:border-cyan-200 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Step
            </button>
            <button
              type="button"
              onClick={handleTogglePause}
              aria-pressed={lab.stepPaused}
              className="rounded-lg border border-violet-300/40 px-4 py-2 text-sm font-semibold text-violet-100 hover:border-violet-200"
            >
              {lab.stepPaused ? "Resume stepping" : "Pause stepping"}
            </button>
          </div>
        </div>

        <div className="rounded-xl border border-violet-300/20 bg-slate-950/50 p-4">
          <h2 className="text-lg font-semibold">Nodes</h2>
          <div className="mt-3 flex flex-wrap items-end gap-3">
            <label className="flex flex-col text-xs text-violet-100/70" htmlFor="lab-node-offset">
              Offset (ms)
              <input
                id="lab-node-offset"
                type="number"
                value={offsetMs}
                onChange={(event) => setOffsetMs(event.target.value)}
                className="mt-1 w-24 rounded-md border border-violet-300/30 bg-slate-900 px-2 py-1 text-sm text-violet-50"
              />
            </label>
            <label className="flex flex-col text-xs text-violet-100/70" htmlFor="lab-node-drift">
              Drift (ppm)
              <input
                id="lab-node-drift"
                type="number"
                value={driftPpm}
                onChange={(event) => setDriftPpm(event.target.value)}
                className="mt-1 w-24 rounded-md border border-violet-300/30 bg-slate-900 px-2 py-1 text-sm text-violet-50"
              />
            </label>
            <button
              type="button"
              onClick={handleStartNode}
              disabled={busy}
              className="rounded-lg border border-cyan-300/50 px-4 py-2 text-sm font-semibold text-cyan-100 hover:border-cyan-200 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Start node
            </button>
            <button
              type="button"
              onClick={handleStopAll}
              disabled={busy || lab.nodes.length === 0}
              className="rounded-lg border border-red-300/40 px-4 py-2 text-sm font-semibold text-red-100 hover:border-red-200 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Stop all
            </button>
          </div>
          <ul className="mt-3 space-y-2">
            {lab.nodes.length === 0 ? (
              <li className="text-sm text-violet-100/50">No active Lab nodes.</li>
            ) : null}
            {lab.nodes.map((node) => (
              <li
                key={node.nodeId}
                className="flex items-center justify-between rounded-lg border border-violet-300/15 bg-slate-900/60 px-3 py-2 text-sm"
              >
                <span className="font-mono">
                  node {node.nodeId} -- offset {node.offsetMs}ms, drift {node.driftPpm}ppm
                </span>
                <button
                  type="button"
                  onClick={() => handleStopNode(node.nodeId)}
                  disabled={busy}
                  className="rounded-md border border-red-300/40 px-3 py-1 text-xs font-semibold text-red-100 hover:border-red-200 disabled:cursor-not-allowed disabled:opacity-40"
                >
                  Stop
                </button>
              </li>
            ))}
          </ul>
        </div>
      </div>

      <div className="rounded-xl border border-violet-300/20 bg-slate-950/50 p-4">
        <h2 className="text-lg font-semibold">Scenario</h2>
        <div className="mt-3 flex flex-wrap gap-3">
          <button
            type="button"
            onClick={handleOpenScenario}
            disabled={busy || lab.running}
            className="rounded-lg border border-cyan-300/50 px-4 py-2 text-sm font-semibold text-cyan-100 hover:border-cyan-200 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Open scenario…
          </button>
          <button
            type="button"
            onClick={handleSaveScenario}
            disabled={busy || lab.loadedScenario === null}
            className="rounded-lg border border-violet-300/40 px-4 py-2 text-sm font-semibold text-violet-100 hover:border-violet-200 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Save scenario…
          </button>
          <button
            type="button"
            onClick={handleRunScenario}
            disabled={busy || !canRun}
            className="rounded-lg border border-emerald-300/50 px-4 py-2 text-sm font-semibold text-emerald-100 hover:border-emerald-200 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {lab.running ? "Running…" : "Run scenario"}
          </button>
        </div>

        {lab.scenarioError ? (
          <div role="alert" className="mt-4 rounded-xl border border-red-300/30 bg-red-950/40 p-4">
            <p className="font-semibold">Invalid scenario</p>
            <p className="mt-2 text-sm">{lab.scenarioError.message}</p>
          </div>
        ) : null}

        {lab.loadedScenario ? (
          <dl className="mt-4 grid grid-cols-2 gap-x-6 gap-y-1 text-sm text-violet-100/80 sm:grid-cols-4">
            <dt className="text-violet-100/50">Schema</dt>
            <dd>{lab.loadedScenario.schemaVersion}</dd>
            <dt className="text-violet-100/50">Seed</dt>
            <dd>{lab.loadedScenario.seed}</dd>
            <dt className="text-violet-100/50">Nodes</dt>
            <dd>{lab.loadedScenario.nodeIds.join(", ") || "none"}</dd>
            <dt className="text-violet-100/50">Timeout</dt>
            <dd>{lab.loadedScenario.timeoutMs} ms</dd>
            <dt className="text-violet-100/50">Steps</dt>
            <dd>{lab.loadedScenario.stepCount}</dd>
            <dt className="text-violet-100/50">Assertions</dt>
            <dd>{lab.loadedScenario.assertionCount}</dd>
          </dl>
        ) : (
          <p className="mt-4 text-sm text-violet-100/50">No scenario open.</p>
        )}

        {lab.loadedScenario && lab.loadedScenario.links.length > 0 ? (
          <div className="mt-4">
            <h3 className="text-sm font-semibold text-violet-100/80">
              Declared fault configuration
            </h3>
            <p className="text-xs text-violet-100/50">
              Display-only: links are not yet wired into live Lab transport.
            </p>
            <table className="mt-2 w-full text-left text-xs">
              <thead className="text-violet-100/50">
                <tr>
                  <th className="pr-3">From</th>
                  <th className="pr-3">To</th>
                  <th className="pr-3">Latency (ms)</th>
                  <th className="pr-3">Jitter (ms)</th>
                  <th>Loss (‰)</th>
                </tr>
              </thead>
              <tbody>
                {lab.loadedScenario.links.map((link, index) => (
                  // Links have no unique identifier of their own -- a
                  // scenario document may legitimately declare more than
                  // one link between the same pair -- so the stable
                  // declaration index is the correct React key here.
                  // biome-ignore lint/suspicious/noArrayIndexKey: links have no other stable identity
                  <tr key={index} className="font-mono text-violet-100/80">
                    <td className="pr-3">{link.from}</td>
                    <td className="pr-3">{link.to}</td>
                    <td className="pr-3">{link.latencyMs}</td>
                    <td className="pr-3">{link.jitterMs}</td>
                    <td>{link.lossPermille}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>

      <div className="rounded-xl border border-violet-300/20 bg-slate-950/50 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Last run</h2>
          <button
            type="button"
            onClick={handleExportRecording}
            disabled={busy || lab.lastRun === null}
            className="rounded-lg border border-violet-300/40 px-4 py-2 text-sm font-semibold text-violet-100 hover:border-violet-200 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Export recording…
          </button>
        </div>

        {lab.lastRun ? (
          <div className="mt-3 space-y-4">
            <p className="text-sm" data-testid="lab-last-run-outcome">
              Outcome:{" "}
              <span
                className={
                  lab.lastRun.outcome === "completed" ? "text-emerald-300" : "text-amber-300"
                }
              >
                {lab.lastRun.outcome}
              </span>{" "}
              at {lab.lastRun.finalTimeMs} ms
            </p>

            <div>
              <h3 className="text-sm font-semibold text-violet-100/80">Assertion results</h3>
              {lab.lastRun.assertionResults.length === 0 ? (
                <p className="text-xs text-violet-100/50">No assertions declared.</p>
              ) : (
                <ul className="mt-1 space-y-1 text-xs">
                  {lab.lastRun.assertionResults.map((assertion, index) => (
                    // biome-ignore lint/suspicious/noArrayIndexKey: assertion order is the only stable identity a report carries
                    <li key={index} className="font-mono">
                      <span
                        className={
                          assertion.outcome === "held" ? "text-emerald-300" : "text-red-300"
                        }
                      >
                        {assertion.outcome === "held" ? "PASS" : "FAIL"}
                      </span>{" "}
                      {assertion.kind} ({assertion.node}, by {assertion.byMs}ms)
                    </li>
                  ))}
                </ul>
              )}
            </div>

            <div>
              <h3 className="text-sm font-semibold text-violet-100/80">
                Event timeline{" "}
                {lab.lastRun.timelineTruncated ? (
                  <span className="text-xs font-normal text-amber-300">(truncated)</span>
                ) : null}
              </h3>
              {lab.lastRun.timeline.length === 0 ? (
                <p className="text-xs text-violet-100/50">No recorded events.</p>
              ) : (
                <ol className="mt-1 max-h-64 space-y-1 overflow-y-auto text-xs">
                  {lab.lastRun.timeline.map((entry) => (
                    <li key={`${entry.node}-${entry.sequence}`} className="font-mono">
                      <span className="text-cyan-300">{entry.node}</span>{" "}
                      <span className="text-violet-100/50">#{entry.sequence}</span>{" "}
                      <span className="text-amber-200">{entry.kind}</span> {entry.summary}
                    </li>
                  ))}
                </ol>
              )}
            </div>
          </div>
        ) : (
          <p className="mt-3 text-sm text-violet-100/50">No scenario has run yet.</p>
        )}
      </div>
    </section>
  );
}
