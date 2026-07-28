import { useEffect, useState } from "react";

import { ensureDesktopBridge, subscribeDesktopNotifications } from "./core/bridge";
import { getCoreSmoke, type CoreSmokeDto } from "./core/client";

type LoadState =
  | { kind: "loading" }
  | {
      kind: "ready";
      core: CoreSmokeDto;
      connectionKind: "opened" | "reattached";
      subscriptionId: string;
      latestRevision: string;
    }
  | { kind: "failed"; message: string };

function describeError(error: unknown): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { message: unknown }).message;
    if (typeof message === "string" && message.trim().length > 0) {
      return message;
    }
  }
  return "The desktop shell could not open or reattach the authoritative Rust profile.";
}

export function App() {
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  useEffect(() => {
    let active = true;
    let latestRevision: string | null = null;
    const unsubscribe = subscribeDesktopNotifications((notification) => {
      if (notification.kind !== "snapshot") {
        return;
      }
      latestRevision = notification.details.revision;
      if (active) {
        setState((current) =>
          current.kind === "ready"
            ? { ...current, latestRevision: notification.details.revision }
            : current,
        );
      }
    });

    Promise.all([getCoreSmoke(42), ensureDesktopBridge("main")])
      .then(([core, connection]) => {
        if (active) {
          setState({
            kind: "ready",
            core,
            connectionKind: connection.connectionKind,
            subscriptionId: connection.notifications.subscriptionId,
            latestRevision: latestRevision ?? connection.snapshot.revision,
          });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setState({ kind: "failed", message: describeError(error) });
        }
      });

    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  return (
    <main className="min-h-screen bg-[radial-gradient(circle_at_top,#312e81_0%,#171229_42%,#100d1a_100%)] px-6 py-10 text-violet-50">
      <section className="mx-auto max-w-3xl rounded-3xl border border-violet-300/20 bg-slate-950/70 p-8 shadow-2xl shadow-black/40 backdrop-blur">
        <p className="text-sm font-semibold uppercase tracking-[0.24em] text-cyan-300">
          Desktop notification bridge
        </p>
        <h1 className="mt-3 text-4xl font-bold tracking-tight">Silent Disco</h1>
        <p className="mt-4 max-w-2xl text-base leading-7 text-violet-100/80">
          The desktop shell opens one Rust-owned profile and subscribes to its revisioned,
          authoritative notification stream. Host controls remain unavailable until their shared
          Rust lifecycle block is complete.
        </p>

        <div className="mt-8 rounded-2xl border border-violet-200/15 bg-black/30 p-5">
          <h2 className="text-lg font-semibold">Authoritative Rust connection</h2>

          {state.kind === "loading" ? (
            <p className="mt-3 text-violet-100/75" role="status" aria-live="polite">
              Opening or reattaching the main profile…
            </p>
          ) : null}

          {state.kind === "ready" ? (
            <dl className="mt-4 grid gap-4 sm:grid-cols-2" aria-label="Desktop bridge status">
              <div>
                <dt className="text-sm text-violet-100/60">Core version</dt>
                <dd className="mt-1 font-mono text-lg">
                  {state.core.major}.{state.core.minor}.{state.core.patch}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Profile connection</dt>
                <dd className="mt-1 text-sm text-cyan-200">
                  {state.connectionKind === "opened"
                    ? "Opened the main profile"
                    : "Reattached to the running main profile"}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Subscription ID</dt>
                <dd className="mt-1 break-all font-mono text-sm text-cyan-200">
                  {state.subscriptionId}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Authoritative revision</dt>
                <dd className="mt-1 break-all font-mono text-sm text-cyan-200">
                  {state.latestRevision}
                </dd>
              </div>
              <div className="sm:col-span-2">
                <dt className="text-sm text-violet-100/60">Deterministic smoke value</dt>
                <dd className="mt-1 break-all font-mono text-sm text-violet-100/80">
                  {state.core.smoke}
                </dd>
              </div>
            </dl>
          ) : null}

          {state.kind === "failed" ? (
            <div
              className="mt-4 rounded-xl border border-red-300/30 bg-red-950/40 p-4"
              role="alert"
            >
              <p className="font-semibold text-red-100">Desktop bridge startup failed</p>
              <p className="mt-2 text-sm leading-6 text-red-100/80">{state.message}</p>
            </div>
          ) : null}
        </div>
      </section>
    </main>
  );
}
