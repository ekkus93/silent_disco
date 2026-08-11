import type {
  DesktopErrorDto,
  HostInvitationDto,
  HostSessionSnapshotDto,
} from "../../core/generated/desktop-bindings";

export const POLL_INTERVAL_MS = 1_000;

export type DecisionKind = "approve" | "reject";

export interface PendingOperation {
  kind: DecisionKind | "remove";
  acceptedAtRevision: string | null;
  baselineDelivery: string;
  baselineError: string;
}

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
