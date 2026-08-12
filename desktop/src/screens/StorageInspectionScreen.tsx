import { useCallback, useEffect, useState } from "react";

import { getStorageInspection, toDesktopBridgeError } from "../core/client";
import type { StorageInspectionDto } from "../core/generated/desktop-bindings";

export function StorageInspectionScreen() {
  const [inspection, setInspection] = useState<StorageInspectionDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setInspection(await getStorageInspection());
    } catch (cause: unknown) {
      const failure = toDesktopBridgeError(cause, "load storage inspection");
      setInspection(null);
      setError(`${failure.code}: ${failure.message}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section
      aria-labelledby="storage-inspection-title"
      className="mt-6 rounded-2xl border border-cyan-300/20 bg-slate-900/60 p-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="storage-inspection-title" className="text-lg font-semibold text-cyan-100">
            Profile storage inspection
          </h2>
          <p className="mt-1 text-xs text-violet-100/60">
            Read-only data from the active Rust-owned database worker.
          </p>
        </div>
        <button
          type="button"
          onClick={() => void load()}
          disabled={loading}
          className="rounded-lg border border-cyan-300/40 px-3 py-2 text-sm font-semibold text-cyan-100 disabled:opacity-50"
        >
          Refresh
        </button>
      </div>

      {loading ? (
        <p role="status" className="mt-4 text-sm text-violet-100/70">
          Reading storage metadata…
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="mt-4 rounded-lg border border-red-300/30 bg-red-950/40 p-3 text-sm text-red-100">
          {error}
        </p>
      ) : null}

      {inspection ? (
        <div className="mt-4 space-y-5">
          <dl className="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2 lg:grid-cols-4">
            <div><dt className="text-violet-100/55">Schema</dt><dd>{inspection.schemaVersion}</dd></div>
            <div><dt className="text-violet-100/55">SQLite</dt><dd>{inspection.sqliteVersion}</dd></div>
            <div><dt className="text-violet-100/55">Journal</dt><dd>{inspection.journalMode}</dd></div>
            <div><dt className="text-violet-100/55">Integrity</dt><dd>{inspection.integrityCheck}</dd></div>
            <div><dt className="text-violet-100/55">Foreign keys</dt><dd>{inspection.foreignKeysEnabled ? "enabled" : "disabled"}</dd></div>
            <div><dt className="text-violet-100/55">Busy timeout</dt><dd>{inspection.busyTimeoutMs} ms</dd></div>
            <div><dt className="text-violet-100/55">Synchronous policy</dt><dd>{inspection.synchronousPolicy}</dd></div>
            <div><dt className="text-violet-100/55">P2 store</dt><dd>{inspection.p2StoreApplicable ? "configured" : "not applicable"}</dd></div>
          </dl>

          <div>
            <h3 className="text-sm font-semibold text-violet-100">Validated settings</h3>
            {inspection.settings ? (
              <p className="mt-2 text-sm text-violet-100/75">
                Sync window {inspection.settings.syncSampleWindow}; cadence {inspection.settings.syncCadenceMs} ms; startup buffer {inspection.settings.startupBufferMs} ms.
              </p>
            ) : (
              <p className="mt-2 text-sm text-violet-100/55">No persisted settings row.</p>
            )}
          </div>

          <div>
            <h3 className="text-sm font-semibold text-violet-100">Trusted devices ({inspection.trustedDevices.length})</h3>
            {inspection.trustedDevices.length === 0 ? (
              <p className="mt-2 text-sm text-violet-100/55">No trusted devices.</p>
            ) : (
              <ul className="mt-2 space-y-1 text-sm">
                {inspection.trustedDevices.map((device) => (
                  <li key={device.deviceId}>
                    {device.displayName} <span className="font-mono text-xs text-violet-100/55">{device.deviceId}</span> — {device.trustState}
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div>
            <h3 className="text-sm font-semibold text-violet-100">Recent sessions ({inspection.recentSessions.length})</h3>
            {inspection.recentSessions.length === 0 ? (
              <p className="mt-2 text-sm text-violet-100/55">No session history.</p>
            ) : (
              <ol className="mt-2 space-y-2 text-sm">
                {inspection.recentSessions.map((session) => (
                  <li key={session.sessionId} className="rounded-lg border border-violet-200/10 p-2">
                    <span className="font-semibold">{session.sessionName}</span> — {session.role} / {session.outcome}; listeners {session.listenerCount}
                    {session.failureCode ? (
                      <span className="block text-xs text-red-200">{session.failureCode}: {session.failureMessage ?? "no failure message"}</span>
                    ) : null}
                  </li>
                ))}
              </ol>
            )}
          </div>
        </div>
      ) : null}
    </section>
  );
}
