import { lazy, Suspense, useEffect, useRef, useState } from "react";

import { coreActions } from "./app/coreSlice";
import {
  selectActiveView,
  selectBridgeLifecycle,
  selectCoreSnapshot,
  selectLatestCoreError,
  selectStaleNotificationCounters,
} from "./app/selectors";
import { useAppDispatch, useAppSelector } from "./app/store";
import { type DesktopView, uiActions } from "./app/uiSlice";
import { ErrorAlert } from "./components/ErrorAlert";
import { RecoverableActionNotice } from "./components/RecoverableActionNotice";
import { ensureDesktopBridge, subscribeDesktopNotifications } from "./core/bridge";
import { getAppShutdownState, getLabModeAvailable, toDesktopBridgeError } from "./core/client";
import type { AppShutdownPhaseDto } from "./core/generated/desktop-bindings";
import { readProfileIdPreference } from "./core/profilePreference";
import { DiagnosticsScreen } from "./screens/DiagnosticsScreen";
import { HostSessionScreen } from "./screens/HostSessionScreen";
import { HostSetupScreen } from "./screens/HostSetupScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { StorageInspectionScreen } from "./screens/StorageInspectionScreen";

// Standard production builds compile the condition to `false`, so Rollup
// removes the dynamic import entirely. This is intentionally a build-time
// boundary in addition to the backend's runtime-truthful availability gate.
const LabScreen = __LAB_FRONTEND_INCLUDED__
  ? lazy(async () => {
      const module = await import("./screens/LabScreen");
      return { default: module.LabScreen };
    })
  : null;

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

const DESTINATIONS: ReadonlyArray<{ id: DesktopView; label: string }> = [
  { id: "host", label: "Host control" },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "storage", label: "Storage" },
  { id: "settings", label: "Settings" },
  { id: "lab", label: "Lab Mode" },
];

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
  const activeView = useAppSelector(selectActiveView);
  const [labModeAvailable, setLabModeAvailable] = useState<boolean | null>(null);
  const [connection, setConnection] = useState<ShellConnectionState | null>(null);
  const destinationRef = useRef<HTMLDivElement | null>(null);
  const previousViewRef = useRef<DesktopView>(activeView);

  useEffect(() => {
    const profileId = readProfileIdPreference();
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
        if (active) setLabModeAvailable(available);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  // Activating a destination used to append it below the active screen, so on
  // a tall session page the click appeared to do nothing. Exactly one
  // destination now renders, and focus follows the switch.
  useEffect(() => {
    if (previousViewRef.current === activeView) return;
    previousViewRef.current = activeView;
    destinationRef.current?.focus();
  }, [activeView]);

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
          {/* Stale-rejected is a shell-level counter about the notification
              stream itself, so it stays here. Revision and lifecycle do not:
              they were also rendered by the session screen from a separately
              polled source, and the two could disagree on screen. */}
          {ready ? (
            <dl className="text-xs text-violet-100/65">
              <dt className="inline">Stale rejected </dt>
              <dd className="inline font-mono">{staleNotifications.snapshots}</dd>
            </dl>
          ) : null}
        </header>

        <div role="tablist" aria-label="Destinations" className="mt-5 flex flex-wrap gap-2">
          {DESTINATIONS.filter(
            (destination) =>
              destination.id !== "lab" || (labModeAvailable === true && LabScreen !== null),
          ).map((destination) => {
            const selected = activeView === destination.id;
            return (
              <button
                key={destination.id}
                type="button"
                role="tab"
                id={`destination-tab-${destination.id}`}
                aria-selected={selected}
                aria-controls="active-destination"
                onClick={() => dispatch(uiActions.activeViewChanged(destination.id))}
                className={`rounded-lg border px-4 py-2 text-sm font-semibold ${
                  selected
                    ? "border-cyan-300/70 bg-cyan-500/10 text-cyan-100"
                    : "border-violet-300/30 text-violet-100/80 hover:border-violet-200"
                }`}
              >
                {destination.label}
              </button>
            );
          })}
        </div>

        <div
          id="active-destination"
          role="tabpanel"
          aria-labelledby={`destination-tab-${activeView}`}
          ref={destinationRef}
          tabIndex={-1}
          className="mt-7 outline-none"
        >
          {activeView === "host" ? (
            <>
              {!ready && !failed ? (
                <p role="status" aria-live="polite">
                  Opening or reattaching the main profile…
                </p>
              ) : null}
              {failed && latestError ? (
                <ErrorAlert
                  error={latestError}
                  title="Desktop bridge startup failed"
                  observedAtMs={latestError.observedAtMs}
                />
              ) : null}
              {failed && !latestError ? (
                <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
                  <p className="font-semibold">Desktop bridge startup failed</p>
                  <p className="mt-2 text-sm">No structured error was returned.</p>
                </div>
              ) : null}
              {ready ? (
                <>
                  <RecoverableActionNotice action={snapshot.recoverableAction} />
                  {ACTIVE_HOST_LIFECYCLES.has(snapshot.hostLifecycle) ? (
                    <HostSessionScreen />
                  ) : (
                    <HostSetupScreen />
                  )}
                </>
              ) : null}
            </>
          ) : null}
          {/* Reachable even when the bridge failed to open -- diagnosing a
              startup failure is exactly when this screen matters most. It
              renders its own failure state, so it does not need `ready`. */}
          {activeView === "storage" ? <StorageInspectionScreen /> : null}
          {activeView === "diagnostics" ? <DiagnosticsScreen /> : null}
          {activeView === "settings" ? <SettingsScreen /> : null}
          {/* Block 42 "production build absence": two independent gates.
              `LabScreen` is literally null in the standard production
              bundle (its dynamic import is compile-time dead), and a Lab
              frontend still requires the backend to truthfully report that
              Rust was compiled with `lab-mode`. */}
          {activeView === "lab" && labModeAvailable && LabScreen ? (
            <Suspense fallback={<p role="status">Loading Lab Mode…</p>}>
              <LabScreen />
            </Suspense>
          ) : null}
        </div>
      </section>
    </main>
  );
}
