import { useEffect, useState } from "react";

import { getCoreSmoke, type CoreSmokeDto } from "./core/client";

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; value: CoreSmokeDto }
  | { kind: "failed"; message: string };

function describeError(error: unknown): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return "The desktop shell could not reach the shared Rust core.";
}

export function App() {
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  useEffect(() => {
    let active = true;

    getCoreSmoke(42)
      .then((value) => {
        if (active) {
          setState({ kind: "ready", value });
        }
      })
      .catch((error: unknown) => {
        if (active) {
          setState({ kind: "failed", message: describeError(error) });
        }
      });

    return () => {
      active = false;
    };
  }, []);

  return (
    <main className="min-h-screen bg-[radial-gradient(circle_at_top,#312e81_0%,#171229_42%,#100d1a_100%)] px-6 py-10 text-violet-50">
      <section className="mx-auto max-w-3xl rounded-3xl border border-violet-300/20 bg-slate-950/70 p-8 shadow-2xl shadow-black/40 backdrop-blur">
        <p className="text-sm font-semibold uppercase tracking-[0.24em] text-cyan-300">
          Desktop foundation
        </p>
        <h1 className="mt-3 text-4xl font-bold tracking-tight">Silent Disco</h1>
        <p className="mt-4 max-w-2xl text-base leading-7 text-violet-100/80">
          This is the production desktop shell foundation. Host controls remain unavailable until
          the authoritative Rust actor and host lifecycle are integrated.
        </p>

        <div className="mt-8 rounded-2xl border border-violet-200/15 bg-black/30 p-5">
          <h2 className="text-lg font-semibold">Shared Rust core</h2>

          {state.kind === "loading" ? (
            <p className="mt-3 text-violet-100/75" role="status" aria-live="polite">
              Verifying the shared core…
            </p>
          ) : null}

          {state.kind === "ready" ? (
            <dl className="mt-4 grid gap-4 sm:grid-cols-2" aria-label="Shared core status">
              <div>
                <dt className="text-sm text-violet-100/60">Core version</dt>
                <dd className="mt-1 font-mono text-lg">
                  {state.value.major}.{state.value.minor}.{state.value.patch}
                </dd>
              </div>
              <div>
                <dt className="text-sm text-violet-100/60">Deterministic smoke value</dt>
                <dd className="mt-1 break-all font-mono text-sm text-cyan-200">
                  {state.value.smoke}
                </dd>
              </div>
            </dl>
          ) : null}

          {state.kind === "failed" ? (
            <div className="mt-4 rounded-xl border border-red-300/30 bg-red-950/40 p-4" role="alert">
              <p className="font-semibold text-red-100">Shared core verification failed</p>
              <p className="mt-2 text-sm leading-6 text-red-100/80">{state.message}</p>
            </div>
          ) : null}
        </div>
      </section>
    </main>
  );
}
