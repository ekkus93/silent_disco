import { useEffect, useMemo, useState } from "react";

import { endHostSession, getHostSessionState, toDesktopBridgeError } from "../core/client";
import type {
  DesktopErrorDto,
  HostConnectionDto,
  HostSessionSnapshotDto,
} from "../core/generated/desktop-bindings";

const REFRESH_INTERVAL_MS = 1000;

function connectionText(connection: HostConnectionDto): string {
  return JSON.stringify(
    {
      address: connection.hostAddress,
      controlPort: connection.controlPort,
      syncPort: connection.syncPort,
      audioPort: connection.audioPort,
      sessionId: connection.sessionId,
      protocolVersion: connection.protocolVersion,
      inviteCodeRequired: connection.inviteCodeRequired,
      expiresAtMs: connection.expiresAtMs,
    },
    null,
    2,
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1 border-b border-violet-200/10 py-3 sm:grid-cols-[12rem_1fr] sm:items-baseline">
      <dt className="text-sm font-medium text-violet-100/65">{label}</dt>
      <dd className="break-all font-mono text-sm text-cyan-100">{value}</dd>
    </div>
  );
}

export function HostSessionScreen() {
  const [snapshot, setSnapshot] = useState<HostSessionSnapshotDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [failure, setFailure] = useState<DesktopErrorDto | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [endPending, setEndPending] = useState(false);
  const connectionPayload = useMemo(
    () => (snapshot?.connection ? connectionText(snapshot.connection) : null),
    [snapshot?.connection],
  );

  useEffect(() => {
    let active = true;

    async function refresh() {
      try {
        const next = await getHostSessionState();
        if (!active) return;
        setSnapshot(next);
        setFailure(null);
      } catch (error: unknown) {
        if (!active) return;
        setFailure(toDesktopBridgeError(error, "refresh host session"));
      } finally {
        if (active) setLoading(false);
      }
    }

    void refresh();
    const interval = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, []);

  async function copyValue(label: string, value: string) {
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("Clipboard access is unavailable.");
      }
      await navigator.clipboard.writeText(value);
      setCopyStatus(`${label} copied.`);
      setFailure(null);
    } catch (error: unknown) {
      setCopyStatus(null);
      setFailure(toDesktopBridgeError(error, `copy ${label.toLowerCase()}`));
    }
  }

  async function requestEndSession() {
    if (!snapshot || endPending) return;
    setEndPending(true);
    setFailure(null);
    try {
      await endHostSession(snapshot.revision);
    } catch (error: unknown) {
      setEndPending(false);
      setFailure(toDesktopBridgeError(error, "end host session"));
    }
  }

  return (
    <section aria-labelledby="host-session-heading" className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">
            Live host workflow
          </p>
          <h2 id="host-session-heading" className="mt-2 text-3xl font-bold">
            Host session
          </h2>
          <p className="mt-2 text-sm text-violet-100/70">
            Connection facts and listener state come from the authoritative Rust runtime.
          </p>
        </div>
        <button
          type="button"
          onClick={() => void requestEndSession()}
          disabled={!snapshot || endPending}
          className="rounded-xl border border-red-300/40 bg-red-950/50 px-4 py-2 font-semibold text-red-100 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {endPending ? "Ending session…" : "End session"}
        </button>
      </div>

      {loading && !snapshot ? (
        <p role="status" aria-live="polite">
          Loading authoritative host session…
        </p>
      ) : null}

      {failure ? (
        <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
          <p className="font-semibold">Host session operation failed</p>
          <p className="mt-2 text-sm">{failure.message}</p>
        </div>
      ) : null}

      {copyStatus ? (
        <p role="status" aria-live="polite" className="text-sm text-cyan-200">
          {copyStatus}
        </p>
      ) : null}

      {endPending ? (
        <p role="status" aria-live="polite" className="text-sm text-amber-200">
          End request accepted. Waiting for a newer Rust lifecycle snapshot.
        </p>
      ) : null}

      {snapshot ? (
        <>
          <section
            aria-labelledby="host-state-heading"
            className="rounded-2xl border border-violet-200/15 bg-slate-900/55 p-5"
          >
            <h3 id="host-state-heading" className="text-xl font-semibold">
              Authoritative host state
            </h3>
            <dl className="mt-3">
              <DetailRow label="Session" value={snapshot.sessionName || "Unnamed session"} />
              <DetailRow label="Revision" value={snapshot.revision} />
              <DetailRow label="Host lifecycle" value={snapshot.hostLifecycle} />
              <DetailRow label="Transport state" value={snapshot.transportState} />
              <DetailRow label="Playback state" value={snapshot.playbackState} />
              <DetailRow
                label="Transport worker"
                value={snapshot.transportWorkerRunning ? "running" : "not running"}
              />
            </dl>
          </section>

          <section
            aria-labelledby="manual-connection-heading"
            className="rounded-2xl border border-cyan-300/20 bg-cyan-950/20 p-5"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h3 id="manual-connection-heading" className="text-xl font-semibold">
                  Manual connection information
                </h3>
                <p className="mt-1 text-sm text-violet-100/65">
                  A listener can use these values directly; mDNS is not required.
                </p>
              </div>
              {snapshot.connection && connectionPayload ? (
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() =>
                      void copyValue("Host address", snapshot.connection?.hostAddress ?? "")
                    }
                    className="rounded-lg border border-cyan-300/30 px-3 py-2 text-sm font-semibold text-cyan-100"
                  >
                    Copy host address
                  </button>
                  <button
                    type="button"
                    onClick={() => void copyValue("Connection details", connectionPayload)}
                    className="rounded-lg border border-cyan-300/30 px-3 py-2 text-sm font-semibold text-cyan-100"
                  >
                    Copy connection details
                  </button>
                </div>
              ) : null}
            </div>

            {snapshot.connection ? (
              <dl className="mt-4">
                <DetailRow label="Host address" value={snapshot.connection.hostAddress} />
                <DetailRow label="Control port" value={String(snapshot.connection.controlPort)} />
                <DetailRow label="Synchronization port" value={String(snapshot.connection.syncPort)} />
                <DetailRow label="Audio port" value={String(snapshot.connection.audioPort)} />
                <DetailRow label="Session ID" value={snapshot.connection.sessionId} />
                <DetailRow
                  label="Protocol version"
                  value={String(snapshot.connection.protocolVersion)}
                />
                <DetailRow
                  label="Invite code"
                  value={snapshot.connection.inviteCodeRequired ? "required" : "not required"}
                />
                <DetailRow
                  label="Expiration"
                  value={snapshot.connection.expiresAtMs ?? "No expiration policy"}
                />
              </dl>
            ) : (
              <p role="status" className="mt-4 text-amber-200">
                Waiting for the shared transport to report a successfully bound endpoint.
              </p>
            )}
          </section>

          {snapshot.transportError ? (
            <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
              <p className="font-semibold">Transport worker error</p>
              <p className="mt-2 text-sm">{snapshot.transportError}</p>
            </div>
          ) : null}
          {snapshot.lastError ? (
            <div role="alert" className="rounded-xl border border-red-300/30 bg-red-950/40 p-4">
              <p className="font-semibold">Core host error</p>
              <p className="mt-2 text-sm">{snapshot.lastError.message}</p>
            </div>
          ) : null}

          <div className="grid gap-6 lg:grid-cols-2">
            <section
              aria-labelledby="pending-listeners-heading"
              className="rounded-2xl border border-violet-200/15 bg-slate-900/55 p-5"
            >
              <h3 id="pending-listeners-heading" className="text-xl font-semibold">
                Pending join requests
              </h3>
              {snapshot.pendingJoinRequests.length === 0 ? (
                <p className="mt-3 text-sm text-violet-100/65">No listener is waiting.</p>
              ) : (
                <ul className="mt-3 space-y-3">
                  {snapshot.pendingJoinRequests.map((request) => (
                    <li
                      key={request.requestId}
                      className="rounded-xl border border-violet-200/10 bg-slate-950/40 p-4"
                    >
                      <p className="font-semibold">{request.displayName}</p>
                      <p className="mt-1 break-all font-mono text-xs text-violet-100/65">
                        {request.deviceId}
                      </p>
                      <p className="mt-2 text-sm">
                        Trust: {request.trustState}; invite code: {request.inviteCodeValid ? "valid" : "not valid"}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
              <p className="mt-4 text-xs text-violet-100/55">
                Approval and rejection controls arrive in Desktop Block 23.
              </p>
            </section>

            <section
              aria-labelledby="connected-listeners-heading"
              className="rounded-2xl border border-violet-200/15 bg-slate-900/55 p-5"
            >
              <h3 id="connected-listeners-heading" className="text-xl font-semibold">
                Connected listeners
              </h3>
              {snapshot.connectedListeners.length === 0 ? (
                <p className="mt-3 text-sm text-violet-100/65">No approved listener is connected.</p>
              ) : (
                <ul className="mt-3 space-y-3">
                  {snapshot.connectedListeners.map((listener) => (
                    <li
                      key={listener.deviceId}
                      className="rounded-xl border border-violet-200/10 bg-slate-950/40 p-4"
                    >
                      <p className="font-semibold">{listener.displayName}</p>
                      <p className="mt-1 text-sm">
                        {listener.transportState} · {listener.trustState}
                      </p>
                      <p className="mt-1 text-xs text-violet-100/60">
                        Last contact: {listener.lastContactMs ?? "not observed"}
                      </p>
                      {listener.lastError ? (
                        <p className="mt-2 text-sm text-red-200">{listener.lastError.message}</p>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>

          <fieldset
            disabled
            aria-describedby="playback-disabled-explanation"
            className="rounded-2xl border border-violet-200/15 bg-slate-900/55 p-5"
          >
            <legend className="px-1 text-xl font-semibold">Playback controls</legend>
            <p id="playback-disabled-explanation" className="mt-2 text-sm text-violet-100/65">
              Audio playback and streaming controls are intentionally unavailable until the later audio pipeline blocks are validated.
            </p>
            <div className="mt-4 flex flex-wrap gap-3">
              <button type="button" className="rounded-lg border px-4 py-2">
                Play
              </button>
              <button type="button" className="rounded-lg border px-4 py-2">
                Pause
              </button>
              <button type="button" className="rounded-lg border px-4 py-2">
                Stop
              </button>
            </div>
          </fieldset>
        </>
      ) : null}
    </section>
  );
}
