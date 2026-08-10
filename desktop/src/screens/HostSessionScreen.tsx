import QRCode from "qrcode";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  approveJoinRequest,
  createHostInvitation,
  endHostSession,
  getHostSessionState,
  pauseHostPlayback,
  rejectJoinRequest,
  removeListener,
  resumeHostPlayback,
  startHostPlayback,
  stopHostPlayback,
} from "../core/client";
import type {
  CommandReceiptDto,
  DesktopErrorDto,
  HostInvitationDto,
  HostSessionSnapshotDto,
} from "../core/generated/desktop-bindings";
import { ListenerDetailScreen } from "./ListenerDetailScreen";

const POLL_INTERVAL_MS = 1_000;

type DecisionKind = "approve" | "reject";

interface PendingOperation {
  kind: DecisionKind | "remove";
  acceptedAtRevision: string | null;
  baselineDelivery: string;
  baselineError: string;
}

function errorKey(error: DesktopErrorDto | null): string {
  return error ? `${error.code}\u0000${error.message}` : "";
}

function deliveryKey(snapshot: HostSessionSnapshotDto): string {
  const delivery = snapshot.lastDelivery;
  return delivery
    ? `${delivery.intendedPeers}/${delivery.successfulPeers}/${delivery.failedPeers}/${delivery.severity}`
    : "";
}

function revisionIsNewer(current: string, baseline: string): boolean {
  if (current.length !== baseline.length) {
    return current.length > baseline.length;
  }
  return current > baseline;
}

function operationFailure(
  next: HostSessionSnapshotDto,
  operation: PendingOperation,
): DesktopErrorDto | null {
  if (
    operation.acceptedAtRevision === null ||
    !revisionIsNewer(next.revision, operation.acceptedAtRevision)
  ) {
    return null;
  }
  if (next.lastError && errorKey(next.lastError) !== operation.baselineError) {
    return next.lastError;
  }
  if (
    next.lastDelivery &&
    deliveryKey(next) !== operation.baselineDelivery &&
    next.lastDelivery.failedPeers > 0 &&
    next.lastDelivery.successfulPeers === 0
  ) {
    return {
      code: "core.transport_delivery_failed",
      subsystem: "transport",
      severity: "error",
      retryable: true,
      message: "The control message was not delivered to the listener.",
    };
  }
  return null;
}

function formatAge(ageMs: string): string {
  const value = Number(ageMs);
  if (!Number.isFinite(value)) {
    return `${ageMs} ms`;
  }
  if (value < 1_000) {
    return `${Math.max(0, Math.round(value))} ms`;
  }
  return `${Math.max(0, Math.round(value / 1_000))} s`;
}

function formatTimestamp(ms: string | null): string {
  if (ms === null) {
    return "--:--";
  }
  const totalSeconds = Math.floor(Math.max(0, Number(ms)) / 1_000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

// Display-only wall-clock formatting for an invitation's absolute
// expiration moment -- distinct from formatTimestamp's playback-position
// elapsed-time formatting above. Never used for sync/playback scheduling,
// which stays monotonic-only.
function formatWallClock(ms: string): string {
  const value = Number(ms);
  if (!Number.isFinite(value)) {
    return "unknown";
  }
  return new Date(value).toLocaleTimeString();
}

function isInvitationExpired(invitation: HostInvitationDto): boolean {
  const expiresAtMs = Number(invitation.expiresAtMs);
  return !Number.isFinite(expiresAtMs) || expiresAtMs <= Date.now();
}

export function HostSessionScreen() {
  const [snapshot, setSnapshot] = useState<HostSessionSnapshotDto | null>(null);
  const [refreshFailure, setRefreshFailure] = useState<DesktopErrorDto | null>(null);
  const [endPending, setEndPending] = useState(false);
  const [playbackPending, setPlaybackPending] = useState(false);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState<string | null>(null);
  const [remember, setRemember] = useState<Record<string, boolean>>({});
  const [selectedListenerId, setSelectedListenerId] = useState<string | null>(null);
  const [decisionOperations, setDecisionOperations] = useState<Record<string, PendingOperation>>(
    {},
  );
  const [removalOperations, setRemovalOperations] = useState<Record<string, PendingOperation>>({});
  const [decisionFailures, setDecisionFailures] = useState<Record<string, DesktopErrorDto>>({});
  const [removalFailures, setRemovalFailures] = useState<Record<string, DesktopErrorDto>>({});
  const [invitation, setInvitation] = useState<HostInvitationDto | null>(null);
  const [invitationQrDataUrl, setInvitationQrDataUrl] = useState<string | null>(null);
  const [invitationPending, setInvitationPending] = useState(false);
  const [invitationError, setInvitationError] = useState<DesktopErrorDto | null>(null);

  const snapshotRef = useRef(snapshot);
  const decisionOperationsRef = useRef(decisionOperations);
  const removalOperationsRef = useRef(removalOperations);

  const updateDecisionOperations = useCallback((next: Record<string, PendingOperation>) => {
    decisionOperationsRef.current = next;
    setDecisionOperations(next);
  }, []);
  const updateRemovalOperations = useCallback((next: Record<string, PendingOperation>) => {
    removalOperationsRef.current = next;
    setRemovalOperations(next);
  }, []);

  const reconcile = useCallback(
    (next: HostSessionSnapshotDto) => {
      const pendingIds = new Set(next.pendingJoinRequests.map((request) => request.requestId));
      const listenerIds = new Set(next.connectedListeners.map((listener) => listener.deviceId));

      const decisions = { ...decisionOperationsRef.current };
      for (const [requestId, operation] of Object.entries(decisions)) {
        if (!pendingIds.has(requestId)) {
          delete decisions[requestId];
          setDecisionFailures((current) => {
            const failures = { ...current };
            delete failures[requestId];
            return failures;
          });
          setAnnouncement(
            operation.kind === "approve"
              ? "Listener approval was confirmed by the Rust core."
              : "Listener rejection was confirmed by the Rust core.",
          );
          continue;
        }
        const failure = operationFailure(next, operation);
        if (failure) {
          delete decisions[requestId];
          setDecisionFailures((current) => ({
            ...current,
            [requestId]: failure,
          }));
          setAnnouncement("The join decision failed and remains actionable.");
        }
      }
      updateDecisionOperations(decisions);

      const removals = { ...removalOperationsRef.current };
      for (const [listenerId, operation] of Object.entries(removals)) {
        if (!listenerIds.has(listenerId)) {
          delete removals[listenerId];
          setRemovalFailures((current) => {
            const failures = { ...current };
            delete failures[listenerId];
            return failures;
          });
          setAnnouncement("Listener removal was confirmed by the Rust core.");
          continue;
        }
        const failure = operationFailure(next, operation);
        if (failure) {
          delete removals[listenerId];
          setRemovalFailures((current) => ({
            ...current,
            [listenerId]: failure,
          }));
          setAnnouncement("Listener removal failed; the listener remains connected.");
        }
      }
      updateRemovalOperations(removals);

      if (selectedListenerId && !listenerIds.has(selectedListenerId)) {
        setSelectedListenerId(null);
      }
    },
    [selectedListenerId, updateDecisionOperations, updateRemovalOperations],
  );

  const refresh = useCallback(async () => {
    try {
      const next = await getHostSessionState();
      reconcile(next);
      snapshotRef.current = next;
      setSnapshot(next);
      setRefreshFailure(null);
    } catch (error) {
      setRefreshFailure(error as DesktopErrorDto);
    }
  }, [reconcile]);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [refresh]);

  // Renders the QR image from the signed invitation text -- pure
  // presentation over data the backend already validated and signed;
  // nothing security-sensitive happens in this step.
  useEffect(() => {
    if (!invitation) {
      setInvitationQrDataUrl(null);
      return;
    }
    let cancelled = false;
    QRCode.toDataURL(invitation.payload)
      .then((dataUrl) => {
        if (!cancelled) setInvitationQrDataUrl(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setInvitationQrDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [invitation]);

  // A session ending (the manual endpoint disappearing) invalidates any
  // invitation that named it -- clearing here means a leftover invitation
  // is never shown as though it still points at a live session.
  useEffect(() => {
    if (!snapshot?.connection) {
      setInvitation(null);
      setInvitationQrDataUrl(null);
      setInvitationError(null);
    }
  }, [snapshot?.connection]);

  async function decide(requestId: string, kind: DecisionKind) {
    const current = snapshotRef.current;
    if (!current || decisionOperationsRef.current[requestId]) {
      return;
    }
    setDecisionFailures((failures) => {
      const next = { ...failures };
      delete next[requestId];
      return next;
    });
    const submitting: PendingOperation = {
      kind,
      acceptedAtRevision: null,
      baselineDelivery: deliveryKey(current),
      baselineError: errorKey(current.lastError),
    };
    updateDecisionOperations({
      ...decisionOperationsRef.current,
      [requestId]: submitting,
    });
    try {
      const receipt: CommandReceiptDto =
        kind === "approve"
          ? await approveJoinRequest({
              expectedRevision: current.revision,
              requestId,
              rememberForFuture: remember[requestId] ?? false,
            })
          : await rejectJoinRequest({
              expectedRevision: current.revision,
              requestId,
            });
      const waiting = {
        ...submitting,
        acceptedAtRevision: receipt.acceptedAtRevision,
      };
      updateDecisionOperations({
        ...decisionOperationsRef.current,
        [requestId]: waiting,
      });
      setAnnouncement(
        kind === "approve"
          ? "Approval queued. Waiting for delivery confirmation."
          : "Rejection queued. Waiting for delivery confirmation.",
      );
    } catch (error) {
      const failure = error as DesktopErrorDto;
      const next = { ...decisionOperationsRef.current };
      delete next[requestId];
      updateDecisionOperations(next);
      setDecisionFailures((failures) => ({
        ...failures,
        [requestId]: failure,
      }));
      setAnnouncement("The join decision was rejected before delivery.");
    }
  }

  async function requestRemoval(listenerId: string) {
    const current = snapshotRef.current;
    if (!current || removalOperationsRef.current[listenerId]) {
      return;
    }
    setRemovalFailures((failures) => {
      const next = { ...failures };
      delete next[listenerId];
      return next;
    });
    const submitting: PendingOperation = {
      kind: "remove",
      acceptedAtRevision: null,
      baselineDelivery: deliveryKey(current),
      baselineError: errorKey(current.lastError),
    };
    updateRemovalOperations({
      ...removalOperationsRef.current,
      [listenerId]: submitting,
    });
    try {
      const receipt = await removeListener({
        expectedRevision: current.revision,
        listenerId,
      });
      updateRemovalOperations({
        ...removalOperationsRef.current,
        [listenerId]: {
          ...submitting,
          acceptedAtRevision: receipt.acceptedAtRevision,
        },
      });
      setAnnouncement("Disconnect queued. Waiting for delivery confirmation.");
    } catch (error) {
      const failure = error as DesktopErrorDto;
      const next = { ...removalOperationsRef.current };
      delete next[listenerId];
      updateRemovalOperations(next);
      setRemovalFailures((failures) => ({
        ...failures,
        [listenerId]: failure,
      }));
      setAnnouncement("Listener removal was rejected before delivery.");
    }
  }

  async function copyValue(label: string, value: string) {
    try {
      await navigator.clipboard.writeText(value);
      setCopyStatus(`${label} copied.`);
    } catch {
      setCopyStatus(`Could not copy ${label.toLowerCase()}.`);
    }
  }

  async function refreshInvitation() {
    if (invitationPending) {
      return;
    }
    setInvitationPending(true);
    setInvitationError(null);
    try {
      const next = await createHostInvitation();
      // Always overwrite, never merge with whatever was showing before --
      // a stale invitation must never linger next to (or be mistaken for)
      // the fresh one this explicit refresh just created (31.2).
      setInvitation(next);
      setAnnouncement("Created a new QR invitation.");
    } catch (error) {
      setInvitation(null);
      setInvitationQrDataUrl(null);
      setInvitationError(error as DesktopErrorDto);
    } finally {
      setInvitationPending(false);
    }
  }

  async function endSession() {
    const current = snapshotRef.current;
    if (!current || endPending) {
      return;
    }
    setEndPending(true);
    try {
      await endHostSession(current.revision);
      setAnnouncement("End-session request queued.");
    } catch (error) {
      setRefreshFailure(error as DesktopErrorDto);
      setEndPending(false);
    }
  }

  async function controlPlayback(action: "start" | "pause" | "resume" | "stop") {
    if (playbackPending) {
      return;
    }
    setPlaybackPending(true);
    try {
      switch (action) {
        case "start":
          await startHostPlayback();
          break;
        case "pause":
          await pauseHostPlayback();
          break;
        case "resume":
          await resumeHostPlayback();
          break;
        case "stop":
          await stopHostPlayback();
          break;
      }
      setAnnouncement(`Playback ${action} requested.`);
    } catch (error) {
      setRefreshFailure(error as DesktopErrorDto);
      setAnnouncement(`Playback ${action} failed.`);
    } finally {
      setPlaybackPending(false);
    }
  }

  if (!snapshot) {
    return (
      <main className="mx-auto max-w-6xl p-6 text-slate-100">
        <h1 className="text-2xl font-semibold">Host session</h1>
        <p className="mt-3">Loading authoritative host state…</p>
        {refreshFailure ? <ErrorAlert error={refreshFailure} /> : null}
      </main>
    );
  }

  const selectedListener = snapshot.connectedListeners.find(
    (listener) => listener.deviceId === selectedListenerId,
  );
  const connectionPayload = snapshot.connection
    ? JSON.stringify({
        hostAddress: snapshot.connection.hostAddress,
        controlPort: snapshot.connection.controlPort,
        syncPort: snapshot.connection.syncPort,
        audioPort: snapshot.connection.audioPort,
        sessionId: snapshot.connection.sessionId,
        protocolVersion: snapshot.connection.protocolVersion,
        inviteCodeRequired: snapshot.connection.inviteCodeRequired,
        expiresAtMs: snapshot.connection.expiresAtMs,
      })
    : null;

  return (
    <main className="mx-auto max-w-6xl p-6 text-slate-100">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">
            Rust-authoritative desktop host
          </p>
          <h1 className="mt-1 text-3xl font-bold">{snapshot.sessionName}</h1>
          <p className="mt-2 text-sm text-slate-400">Snapshot revision {snapshot.revision}</p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => void refresh()}
            className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-semibold hover:border-slate-400"
          >
            Refresh host state
          </button>
          <button
            type="button"
            onClick={() => void endSession()}
            disabled={endPending}
            className="rounded-lg border border-rose-500/70 px-4 py-2 text-sm font-semibold text-rose-100 hover:bg-rose-950/50 disabled:opacity-50"
          >
            {endPending ? "Ending session…" : "End session"}
          </button>
        </div>
      </div>

      <div aria-live="polite" aria-atomic="true" className="mt-3 min-h-6 text-sm text-cyan-200">
        {announcement ?? copyStatus}
      </div>

      <section aria-labelledby="host-status-title" className="mt-6 grid gap-3 md:grid-cols-4">
        <h2 id="host-status-title" className="sr-only">
          Host status
        </h2>
        <StatusCard label="Host lifecycle" value={snapshot.hostLifecycle} />
        <StatusCard label="Transport" value={snapshot.transportState} />
        <StatusCard label="Playback" value={snapshot.playbackState} />
        <StatusCard
          label="Transport worker"
          value={snapshot.transportWorkerRunning ? "running" : "not running"}
        />
      </section>

      {snapshot.connection ? (
        <section
          aria-labelledby="connection-title"
          className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
        >
          <h2 id="connection-title" className="text-xl font-semibold">
            Manual connection details
          </h2>
          <p className="mt-2 text-sm text-slate-400">
            Listeners can enter these details without mDNS.
          </p>
          <dl className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <Detail label="Host address" value={snapshot.connection.hostAddress} />
            <Detail label="Control port" value={String(snapshot.connection.controlPort)} />
            <Detail label="Sync port" value={String(snapshot.connection.syncPort)} />
            <Detail label="Audio port" value={String(snapshot.connection.audioPort)} />
            <Detail label="Session ID" value={snapshot.connection.sessionId} />
            <Detail label="Protocol version" value={String(snapshot.connection.protocolVersion)} />
            <Detail
              label="Invite code"
              value={snapshot.connection.inviteCodeRequired ? "required" : "not required"}
            />
            <Detail
              label="Expiration"
              value={snapshot.connection.expiresAtMs ?? "No expiration policy"}
            />
          </dl>
          <div className="mt-4 flex flex-wrap gap-2">
            <CopyButton
              label="Copy host address"
              onClick={() => void copyValue("Host address", snapshot.connection?.hostAddress ?? "")}
            />
            <CopyButton
              label="Copy session ID"
              onClick={() => void copyValue("Session ID", snapshot.connection?.sessionId ?? "")}
            />
            {connectionPayload ? (
              <CopyButton
                label="Copy connection payload"
                onClick={() => void copyValue("Connection payload", connectionPayload)}
              />
            ) : null}
          </div>
        </section>
      ) : (
        <p className="mt-6 rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100">
          The core has not published an active manual endpoint.
        </p>
      )}

      {snapshot.connection ? (
        <section
          aria-labelledby="invitation-title"
          className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
        >
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 id="invitation-title" className="text-xl font-semibold">
                QR invitation
              </h2>
              <p className="mt-2 text-sm text-slate-400">
                A signed, time-limited invitation a phone can scan to join directly -- a convenience
                alongside the manual details above, not a replacement for them.
              </p>
            </div>
            <button
              type="button"
              onClick={() => void refreshInvitation()}
              disabled={invitationPending}
              className="rounded-lg border border-cyan-500/60 px-4 py-2 text-sm font-semibold text-cyan-100 hover:bg-cyan-950/40 disabled:opacity-50"
            >
              {invitationPending
                ? "Creating…"
                : invitation
                  ? "Refresh QR invitation"
                  : "Create QR invitation"}
            </button>
          </div>

          {invitationError ? <ErrorAlert error={invitationError} /> : null}

          {invitation ? (
            isInvitationExpired(invitation) ? (
              <p
                role="status"
                className="mt-4 rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100"
              >
                This invitation expired. Create a new one -- an expired invitation is never reused
                automatically.
              </p>
            ) : (
              <div className="mt-4 flex flex-wrap items-start gap-4">
                {invitationQrDataUrl ? (
                  <img
                    src={invitationQrDataUrl}
                    alt="Signed Silent Disco join QR code"
                    className="h-56 w-56 rounded-xl bg-white p-2"
                  />
                ) : (
                  <p
                    role="alert"
                    className="rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
                  >
                    The signed invitation was created, but this app could not render its QR image.
                    Use the text fallback below instead.
                  </p>
                )}
                <div className="min-w-[16rem] flex-1 space-y-3">
                  <Detail label="Expires" value={formatWallClock(invitation.expiresAtMs)} />
                  <CopyButton
                    label="Copy invitation text"
                    onClick={() => void copyValue("Invitation text", invitation.payload)}
                  />
                </div>
              </div>
            )
          ) : (
            <p className="mt-4 text-sm text-slate-400">
              No invitation created yet. This does not affect manual connections above.
            </p>
          )}
        </section>
      ) : null}

      <section
        aria-labelledby="pending-title"
        className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
      >
        <h2 id="pending-title" className="text-xl font-semibold">
          Pending join requests
        </h2>
        {snapshot.pendingJoinRequests.length === 0 ? (
          <p className="mt-3 text-sm text-slate-400">No pending requests.</p>
        ) : (
          <ul className="mt-4 space-y-3">
            {snapshot.pendingJoinRequests.map((request) => {
              const operation = decisionOperations[request.requestId];
              const failure = decisionFailures[request.requestId];
              return (
                <li
                  key={request.requestId}
                  aria-busy={Boolean(operation)}
                  className="rounded-xl bg-slate-900 p-4"
                >
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="font-semibold">{request.displayName}</p>
                      <p className="mt-1 font-mono text-xs text-slate-400">
                        Request {request.requestId}
                      </p>
                      <p className="font-mono text-xs text-slate-500">Device {request.deviceId}</p>
                    </div>
                    <div className="text-right text-xs text-slate-300">
                      <p>Age: {formatAge(request.ageMs)}</p>
                      <p>Trust: {request.trustState}</p>
                      <p>Invite: {request.inviteCodeValid ? "valid" : "not valid"}</p>
                    </div>
                  </div>

                  <fieldset
                    disabled={Boolean(operation)}
                    className="mt-4 flex flex-wrap items-center gap-3"
                  >
                    <legend className="sr-only">Decision for {request.displayName}</legend>
                    <label className="flex items-center gap-2 text-sm text-slate-200">
                      <input
                        type="checkbox"
                        checked={remember[request.requestId] ?? false}
                        disabled={request.trustState === "trusted"}
                        onChange={(event) =>
                          setRemember((current) => ({
                            ...current,
                            [request.requestId]: event.target.checked,
                          }))
                        }
                      />
                      Remember this device
                    </label>
                    <button
                      type="button"
                      onClick={() => void decide(request.requestId, "approve")}
                      className="rounded-lg bg-cyan-600 px-4 py-2 text-sm font-semibold text-white hover:bg-cyan-500 disabled:opacity-50"
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      onClick={() => void decide(request.requestId, "reject")}
                      className="rounded-lg border border-rose-500/70 px-4 py-2 text-sm font-semibold text-rose-100 hover:bg-rose-950/40 disabled:opacity-50"
                    >
                      Reject
                    </button>
                  </fieldset>
                  {operation ? (
                    <p className="mt-3 text-sm text-cyan-200">
                      {operation.kind === "approve"
                        ? "Waiting for approval delivery confirmation…"
                        : "Waiting for rejection delivery confirmation…"}
                    </p>
                  ) : null}
                  {failure ? <ErrorAlert error={failure} /> : null}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section
        aria-labelledby="listeners-title"
        className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
      >
        <h2 id="listeners-title" className="text-xl font-semibold">
          Connected listeners
        </h2>
        {selectedListener ? (
          <div className="mt-4">
            <ListenerDetailScreen
              listener={selectedListener}
              lastDelivery={snapshot.lastDelivery}
              pending={Boolean(removalOperations[selectedListener.deviceId])}
              failure={removalFailures[selectedListener.deviceId] ?? null}
              onRemove={() => void requestRemoval(selectedListener.deviceId)}
              onBack={() => setSelectedListenerId(null)}
            />
          </div>
        ) : snapshot.connectedListeners.length === 0 ? (
          <p className="mt-3 text-sm text-slate-400">
            No listeners have completed delivery-confirmed approval.
          </p>
        ) : (
          <ul className="mt-4 grid gap-3 md:grid-cols-2">
            {snapshot.connectedListeners.map((listener) => (
              <li key={listener.deviceId} className="rounded-xl bg-slate-900 p-4">
                <p className="font-semibold">{listener.displayName}</p>
                <p className="mt-1 text-xs text-slate-400">
                  {listener.transportState} · {listener.trustState}
                </p>
                <p className="mt-1 text-xs text-slate-400">
                  Sync: {listener.syncConfidence ?? "not available"}
                </p>
                <button
                  type="button"
                  onClick={() => setSelectedListenerId(listener.deviceId)}
                  className="mt-3 rounded-lg border border-slate-600 px-3 py-2 text-sm font-semibold hover:border-slate-400"
                >
                  View {listener.displayName}
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section
        aria-labelledby="playback-title"
        className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
      >
        <h2 id="playback-title" className="text-xl font-semibold">
          Playback controls
        </h2>
        <p className="mt-2 text-sm text-slate-400">
          Requires a selected audio source and an active host session.
        </p>
        {snapshot.audioSource ? (
          <div className="mt-3 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
            <span className="font-semibold text-slate-100">{snapshot.audioSource.displayName}</span>
            <span className="font-mono text-slate-400">
              {formatTimestamp(snapshot.playbackPositionMs)} /{" "}
              {formatTimestamp(snapshot.audioSource.durationMs)}
            </span>
            {snapshot.streamEndedNaturally ? (
              <span className="rounded-full border border-cyan-500/60 bg-cyan-950/30 px-2 py-0.5 text-xs font-semibold text-cyan-200">
                Finished
              </span>
            ) : null}
          </div>
        ) : null}
        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() =>
              void controlPlayback(snapshot.playbackState === "paused" ? "resume" : "start")
            }
            disabled={
              playbackPending ||
              !snapshot.playbackControlsEnabled ||
              !["stopped", "ready", "paused"].includes(snapshot.playbackState)
            }
            className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
          >
            {snapshot.playbackState === "paused" ? "Resume" : "Play"}
          </button>
          <button
            type="button"
            onClick={() => void controlPlayback("pause")}
            disabled={
              playbackPending ||
              !snapshot.playbackControlsEnabled ||
              snapshot.playbackState !== "playing"
            }
            className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
          >
            Pause
          </button>
          <button
            type="button"
            onClick={() => void controlPlayback("stop")}
            disabled={
              playbackPending ||
              !snapshot.playbackControlsEnabled ||
              !["playing", "paused", "buffering"].includes(snapshot.playbackState)
            }
            className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
          >
            Stop
          </button>
        </div>
      </section>

      {snapshot.lastDelivery ? (
        <p
          role={snapshot.lastDelivery.successfulPeers === 0 ? "alert" : undefined}
          className={`mt-6 rounded-xl border p-4 text-sm ${
            snapshot.lastDelivery.successfulPeers === 0 && snapshot.lastDelivery.intendedPeers > 0
              ? "border-rose-500/60 bg-rose-950/30 text-rose-100"
              : snapshot.lastDelivery.failedPeers > 0
                ? "border-amber-500/60 bg-amber-950/30 text-amber-100"
                : "border-emerald-500/60 bg-emerald-950/30 text-emerald-100"
          }`}
        >
          {snapshot.lastDelivery.successfulPeers === 0 && snapshot.lastDelivery.intendedPeers > 0
            ? `Last delivery reached nobody: 0 of ${snapshot.lastDelivery.intendedPeers} listeners (${snapshot.lastDelivery.severity}).`
            : `Last delivery: ${snapshot.lastDelivery.successfulPeers} of ${snapshot.lastDelivery.intendedPeers} succeeded; ${snapshot.lastDelivery.failedPeers} failed (${snapshot.lastDelivery.severity}).`}
        </p>
      ) : null}
      {snapshot.broadcast ? (
        <p
          role={Number(snapshot.broadcast.queueOverflows) > 0 ? "alert" : undefined}
          className={`mt-3 text-sm ${
            Number(snapshot.broadcast.queueOverflows) > 0 ? "text-amber-200" : "text-slate-400"
          }`}
        >
          Broadcast queue: {snapshot.broadcast.queueDepth} queued (peak{" "}
          {snapshot.broadcast.queuePeakDepth})
          {Number(snapshot.broadcast.queueOverflows) > 0
            ? `; ${snapshot.broadcast.queueOverflows} frame(s) dropped for a full queue.`
            : "."}
        </p>
      ) : null}
      {snapshot.transportError ? (
        <p role="alert" className="mt-6 text-sm text-rose-200">
          Transport worker: {snapshot.transportError}
        </p>
      ) : null}
      {snapshot.lastError ? <ErrorAlert error={snapshot.lastError} /> : null}
      {refreshFailure ? <ErrorAlert error={refreshFailure} /> : null}
    </main>
  );
}

function StatusCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-slate-700 bg-slate-900 p-4">
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</p>
      <p className="mt-2 break-words text-lg font-semibold">{value}</p>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-slate-900 p-3">
      <dt className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</dt>
      <dd className="mt-1 break-all text-sm text-slate-100">{value}</dd>
    </div>
  );
}

function CopyButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-lg border border-slate-600 px-3 py-2 text-sm font-semibold hover:border-slate-400"
    >
      {label}
    </button>
  );
}

function ErrorAlert({ error }: { error: DesktopErrorDto }) {
  return (
    <div
      role="alert"
      className="mt-4 rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
    >
      <p className="font-semibold">{error.message}</p>
      <p className="mt-1 font-mono text-xs">{error.code}</p>
    </div>
  );
}
