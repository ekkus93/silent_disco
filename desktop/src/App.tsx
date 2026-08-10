import { useEffect, useState } from "react";

import { coreActions } from "./app/coreSlice";
import { labActions } from "./app/labSlice";
import {
  selectBridgeLifecycle,
  selectCoreSnapshot,
  selectLabModeAvailable,
  selectLatestCoreError,
  selectStaleNotificationCounters,
} from "./app/selectors";
import { useAppDispatch, useAppSelector } from "./app/store";
import { ensureDesktopBridge, subscribeDesktopNotifications } from "./core/bridge";
import { getAppShutdownState, getLabModeAvailable, toDesktopBridgeError } from "./core/client";
import type { AppShutdownPhaseDto } from "./core/generated/desktop-bindings";
import { DiagnosticsScreen } from "./screens/DiagnosticsScreen";
import { HostSessionScreen } from "./screens/HostSessionScreen";
import { HostSetupScreen } from "./screens/HostSetupScreen";

// Block 36.3 "progress is visible" / "timeout becomes visible failure":
// polled rather than pushed -- the webview stays fully alive while a
// window close request is pending (only the native close is prevented,
// not the process), so a lightweight poll is enough, matching every other
// status screen in this app.
const SHUTDOWN_POLL_INTERVAL_MS = 500;

function ShutdownOverlay() {
  const [phase, setPhase] = useState<AppShutdownPhaseDto | null>(null);

  useEffect(() => {
    let active = true;
    const poll = () => {
      void getAppShutdownState()
        .then((next) => {
          if (active) setPhase(next);
        })
        .catch(() => {
          // A transient poll failure keeps the last-known phase on screen
          // rather than flashing back to "nothing is happening".
        });
    };
    poll();
    const interval = window.setInterval(poll, SHUTDOWN_POLL_INTERVAL_MS);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, []);

  if (!phase || phase.kind === "notRequested") {
    return null;
  }

  return (
    <div
      role="alert"
      aria-live="assertive"
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/90 backdrop-blur-sm"
    >
      <div className="max-w-md rounded-2xl border border-violet-300/30 bg-slate-900 p-8 text-center text-violet-50 shadow-2xl">
        {phase.kind === "shuttingDown" ? (
          <>
            <p className="text-lg font-semibold">Shutting down…</p>
            <p className="mt-2 text-sm text-violet-100/70">
              Closing the active session and releasing local resources. This window will close
              automatically once that finishes.
            </p>
          </>
        ) : null}
        {phase.kind === "shutdownFailed" ? (
          <>
            <p className="text-lg font-semibold text-rose-200">Shutdown did not complete cleanly</p>
            <p className="mt-2 text-sm text-rose-100/80">{phase.details.error.message}</p>
            <p className="mt-1 font-mono text-xs text-rose-100/60">{phase.details.error.code}</p>
            <p className="mt-4 text-xs text-violet-100/60">
              It is not safe to retry automatically -- close this window from your operating system
              if it does not exit on its own.
            </p>
          </>
        ) : null}
        {phase.kind === "terminated" ? (
          <p className="text-lg font-semibold">Shutdown complete.</p>
        ) : null}
      </div>
    </div>
  );
}

interface ShellConnectionState {
  connectionKind: "opened" | "reattached";
  subscriptionId: string;
}

const ACTIVE_HOST_LIFECYCLES = new Set([
  "creating_session",
  "advertising",
  "waiting_for_listeners",
  "ready",
  "streaming",
  "paused",
  "ending_session",
  "error",
]);

export function App() {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectCoreSnapshot);
  const lifecycle = useAppSelector(selectBridgeLifecycle);
  const latestError = useAppSelector(selectLatestCoreError);
  const staleNotifications = useAppSelector(selectStaleNotificationCounters);
  const labModeAvailable = useAppSelector(selectLabModeAvailable);
  const [connection, setConnection] = useState<ShellConnectionState | null>(null);
  const [showDiagnostics, setShowDiagnostics] = useState(false);

  useEffect(() => {
    const profileId = "main";
    let active = true;
    dispatch(coreActions.bridgeOpening({ profileId }));
    const unsubscribe = subscribeDesktopNotifications((notification) => {
      dispatch(coreActions.notificationReceived(notification));
    });
    ensureDesktopBridge(profileId)
      .then((bridgeConnection) => {
        if (!active) return;
        dispatch(coreActions.bridgeReady({ profileId, snapshot: bridgeConnection.snapshot }));
        setConnection({
          connectionKind: bridgeConnection.connectionKind,
          subscriptionId: bridgeConnection.notifications.subscriptionId,
        });
      })
      .catch((error: unknown) => {
        if (active) {
          dispatch(coreActions.bridgeFailed(toDesktopBridgeError(error, "desktop bridge startup")));
        }
      });
    return () => {
      active = false;
      unsubscribe();
    };
  }, [dispatch]);

  // Block 37.1 "UI is visibly labeled": asked once per session, straight
  // from the backend -- never assumed from a JS-only dev/prod distinction.
  // A transport failure here leaves `available` at its initial `null`
  // (not shown as available), the same fail-closed default as never
  // having asked at all.
  useEffect(() => {
    let active = true;
    getLabModeAvailable()
      .then((available) => {
        if (active) dispatch(labActions.labModeAvailabilityReceived(available));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [dispatch]);

  const ready = lifecycle.kind === "ready" && snapshot !== null && connection !== null;
  const failed = lifecycle.kind === "failed";

  return (
    <main className="min-h-screen bg-[radial-gradient(circle_at_top,#312e81_0%,#171229_42%,#100d1a_100%)] px-6 py-10 text-violet-50">
      <ShutdownOverlay />
      <section className="mx-auto max-w-6xl rounded-3xl border border-violet-300/20 bg-slate-950/70 p-8 shadow-2xl shadow-black/40 backdrop-blur">
        <header className="flex flex-wrap items-end justify-between gap-4 border-b border-violet-200/15 pb-5">
          <div>
            <div className="flex items-center gap-3">
              <p className="text-sm font-semibold uppercase tracking-[0.24em] text-cyan-300">
                Silent Disco desktop host
              </p>
              {labModeAvailable ? (
                <span
                  role="status"
                  title="This build was compiled with the lab-mode feature and must never be used for a real listening session."
                  className="rounded-full border border-amber-400/60 bg-amber-950/40 px-2 py-0.5 text-xs font-bold uppercase tracking-wide text-amber-200"
                >
                  Lab Mode build
                </span>
              ) : null}
            </div>
            <h1 className="mt-2 text-4xl font-bold tracking-tight">Host control</h1>
          </div>
          <div className="flex items-center gap-4">
            {ready ? (
              <dl className="grid grid-cols-2 gap-x-5 gap-y-1 text-xs text-violet-100/65">
                <dt>Revision</dt>
                <dd className="font-mono text-right text-cyan-200">{snapshot.revision}</dd>
                <dt>Lifecycle</dt>
                <dd className="font-mono text-right">{snapshot.hostLifecycle}</dd>
                <dt>Stale rejected</dt>
                <dd className="font-mono text-right">{staleNotifications.snapshots}</dd>
              </dl>
            ) : null}
            <button
              type="button"
              onClick={() => setShowDiagnostics((current) => !current)}
              aria-pressed={showDiagnostics}
              className="rounded-lg border border-violet-300/40 px-4 py-2 text-sm font-semibold text-violet-100 hover:border-violet-200"
            >
              {showDiagnostics ? "Hide diagnostics" : "Diagnostics"}
            </button>
          </div>
        </header>

        <div className="mt-7">
          {!ready && !failed ? (
            <p role="status" aria-live="polite">
              Opening or reattaching the main profile…
            </p>
          ) : null}
          {ready && ACTIVE_HOST_LIFECYCLES.has(snapshot.hostLifecycle) ? (
            <HostSessionScreen />
          ) : null}
          {ready && !ACTIVE_HOST_LIFECYCLES.has(snapshot.hostLifecycle) ? (
            <HostSetupScreen />
          ) : null}
          {failed ? (
            <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
              <p className="font-semibold">Desktop bridge startup failed</p>
              <p className="mt-2 text-sm">
                {latestError?.message ?? "No structured error was returned."}
              </p>
            </div>
          ) : null}
          {/* Available even when the bridge failed to open -- diagnosing a
              startup failure is exactly when this screen matters most. */}
          {showDiagnostics ? <DiagnosticsScreen /> : null}
        </div>
      </section>
    </main>
  );
}
