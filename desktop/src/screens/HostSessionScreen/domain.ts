import type {
  ConnectedListenerDto,
  DesktopErrorDto,
  HostInvitationDto,
  HostSessionSnapshotDto,
} from "../../core/generated/desktop-bindings";

export const POLL_INTERVAL_MS = 1_000;

export type DecisionKind = "approve" | "reject";

export interface PendingOperation {
  kind: DecisionKind | "remove";
  acceptedAtRevision: string | null;
  /** Wall-clock time the core accepted the command; `null` until it does. */
  acceptedAtMs: number | null;
  baselineDelivery: string;
  baselineError: string;
}

/**
 * How long an accepted join decision or removal may wait for its delivery
 * confirmation before it is reported as failed. Confirmation normally takes
 * well under a second; without a bound, an operation whose confirmation never
 * arrives showed "Waiting for ... delivery confirmation" forever (Block 52.4),
 * including when an identical failure left no new error or delivery report to
 * compare against.
 */
export const DELIVERY_CONFIRMATION_TIMEOUT_MS = 15_000;

export function errorKey(error: DesktopErrorDto | null): string {
  return error ? `${error.code}\u0000${error.message}` : "";
}

export function deliveryKey(snapshot: HostSessionSnapshotDto): string {
  const delivery = snapshot.lastDelivery;
  return delivery
    ? `${delivery.intendedPeers}/${delivery.successfulPeers}/${delivery.failedPeers}/${delivery.severity}`
    : "";
}

export function revisionIsNewer(current: string, baseline: string): boolean {
  if (current.length !== baseline.length) {
    return current.length > baseline.length;
  }
  return current > baseline;
}

export function operationFailure(
  next: HostSessionSnapshotDto,
  operation: PendingOperation,
  nowMs: number = Date.now(),
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
  if (
    operation.acceptedAtMs !== null &&
    nowMs - operation.acceptedAtMs >= DELIVERY_CONFIRMATION_TIMEOUT_MS
  ) {
    return {
      code: "desktop.delivery_confirmation_timeout",
      subsystem: "transport",
      severity: "error",
      retryable: true,
      message: `No delivery confirmation arrived within ${
        DELIVERY_CONFIRMATION_TIMEOUT_MS / 1000
      } seconds. The listener may not have received it; try again.`,
    };
  }
  return null;
}

export function formatAge(ageMs: string): string {
  const value = Number(ageMs);
  if (!Number.isFinite(value)) {
    return `${ageMs} ms`;
  }
  if (value < 1_000) {
    return `${Math.max(0, Math.round(value))} ms`;
  }
  return `${Math.max(0, Math.round(value / 1_000))} s`;
}

export function formatTimestamp(ms: string | null): string {
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
export function formatWallClock(ms: string): string {
  const value = Number(ms);
  if (!Number.isFinite(value)) {
    return "unknown";
  }
  return new Date(value).toLocaleTimeString();
}

export function isInvitationExpired(invitation: HostInvitationDto): boolean {
  const expiresAtMs = Number(invitation.expiresAtMs);
  return !Number.isFinite(expiresAtMs) || expiresAtMs <= Date.now();
}

/**
 * What an approved listener is doing, in operator terms.
 *
 * The core records an approved listener's transport state as `connecting` and
 * nothing on the desktop ever advances it, so showing that raw value made a
 * listener that was approved and waiting -- or even playing in sync -- look
 * stuck (Block 52.5). Every listener in the list has already completed
 * delivery-confirmed approval, so its progress is read from whether a stream
 * is running and whether it has reported sync.
 */
export function listenerProgress(
  listener: Pick<ConnectedListenerDto, "syncConfidence">,
  playbackState: string,
): string {
  if (listener.syncConfidence !== null) {
    return `Connected — sync ${listener.syncConfidence}`;
  }
  if (playbackState === "buffering" || playbackState === "playing") {
    return "Approved — waiting for its first clock sync";
  }
  return "Approved — waiting for the host to start playback";
}
