import { useEffect, useState } from "react";

import { coreActions } from "./app/coreSlice";
import {
  selectBridgeLifecycle,
  selectCoreSnapshot,
  selectLatestCoreError,
  selectStaleNotificationCounters,
} from "./app/selectors";
import { useAppDispatch, useAppSelector } from "./app/store";
import { ensureDesktopBridge, subscribeDesktopNotifications } from "./core/bridge";
import { toDesktopBridgeError } from "./core/client";
import { HostSessionScreen } from "./screens/HostSessionScreen";
import { HostSetupScreen } from "./screens/HostSetupScreen";

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
  const [connection, setConnection] = useState<ShellConnectionState | null>(null);

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

  const ready = lifecycle.kind === "ready" && snapshot !== null && connection !== null;
  const failed = lifecycle.kind === "failed";

  return (
    <main className="min-h-screen bg-[radial-gradient(circle_at_top,#312e81_0%,#171229_42%,#100d1a_100%)] px-6 py-10 text-violet-50">
      <section className="mx-auto max-w-6xl rounded-3xl border border-violet-300/20 bg-slate-950/70 p-8 shadow-2xl shadow-black/40 backdrop-blur">
        <header className="flex flex-wrap items-end justify-between gap-4 border-b border-violet-200/15 pb-5">
          <div>
            <p className="text-sm font-semibold uppercase tracking-[0.24em] text-cyan-300">
              Silent Disco desktop host
            </p>
            <h1 className="mt-2 text-4xl font-bold tracking-tight">Host control</h1>
          </div>
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
        </div>
      </section>
    </main>
  );
}
