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

interface ShellConnectionState {
  connectionKind: "opened" | "reattached";
  subscriptionId: string;
}

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
        if (!active) {
          return;
        }
        dispatch(
          coreActions.bridgeReady({
            profileId,
            snapshot: bridgeConnection.snapshot,
          }),
        );
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
      <section className="mx-auto max-w-3xl rounded-3xl border border-violet-300/20 bg-slate-950/70 p-8 shadow-2xl shadow-black/40 backdrop-blur">
        <p className="text-sm font-semibold uppercase tracking-[0.24em] text-cyan-300">
          Authoritative desktop state
        </p>
        <h1 className="mt-3 text-4xl font-bold tracking-tight">Silent Disco</h1>
        <p className="mt-4 max-w-2xl text-base leading-7 text-violet-100/80">
          Redux stores the latest complete Rust snapshot. Revision guards reject duplicate or older
          notifications instead of reconstructing host lifecycle in React.
        </p>

        <div className="mt-8 rounded-2xl border border-violet-200/15 bg-black/30 p-5">
          <h2 className="text-lg font-semibold">Authoritative Rust connection</h2>

          {!ready && !failed ? (
            <p className="mt-3 text-violet-100/75" role="status" aria-live="polite">
              Opening or reattaching the main profile…
            </p>
          ) : null}

          {ready ? (
            <dl className="mt-4 grid gap-4 sm:grid-cols-2" aria-label="Desktop bridge status">
              <div>
                <dt className="text-sm text-violet-100/60">Profile connection</dt>
                <dd className="mt-1 text-sm text-cyan-200">
                  {connection.connectionKind === "opened"
                    ? "Opened the main profile"
                    : "Reattached to the running main profile"}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Subscription ID</dt>
                <dd className="mt-1 break-all font-mono text-sm text-cyan-200">
                  {connection.subscriptionId}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Authoritative revision</dt>
                <dd className="mt-1 break-all font-mono text-sm text-cyan-200">
                  {snapshot.revision}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Host lifecycle</dt>
                <dd className="mt-1 font-mono text-sm text-violet-100/80">
                  {snapshot.hostLifecycle}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Stale snapshots rejected</dt>
                <dd className="mt-1 font-mono text-sm text-violet-100/80">
                  {staleNotifications.snapshots}
                </dd>
              </div>
            </dl>
          ) : null}

          {failed ? (
            <div
              className="mt-4 rounded-xl border border-red-300/30 bg-red-950/40 p-4"
              role="alert"
            >
              <p className="font-semibold text-red-100">Desktop bridge startup failed</p>
              <p className="mt-2 text-sm leading-6 text-red-100/80">
                {latestError?.message ?? "The desktop bridge failed without an error payload."}
              </p>
            </div>
          ) : null}

          {!failed && latestError !== null ? (
            <div
              className="mt-4 rounded-xl border border-amber-300/30 bg-amber-950/40 p-4"
              role="alert"
            >
              <p className="font-semibold text-amber-100">Core command or bridge error</p>
              <p className="mt-2 text-sm leading-6 text-amber-100/80">{latestError.message}</p>
            </div>
          ) : null}
        </div>
      </section>
    </main>
  );
}
