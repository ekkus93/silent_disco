import { describe, expect, it } from "vitest";

import type { HostSessionSnapshotDto } from "../../core/generated/desktop-bindings";
import {
  DELIVERY_CONFIRMATION_TIMEOUT_MS,
  deliveryKey,
  errorKey,
  listenerProgress,
  operationFailure,
  unconfirmedDisappearanceError,
  type PendingOperation,
} from "./domain";

function snapshot(overrides: Partial<HostSessionSnapshotDto> = {}): HostSessionSnapshotDto {
  return {
    revision: "10",
    hostLifecycle: "waiting_for_listeners",
    transportState: "advertising",
    playbackState: "stopped",
    sessionName: "Test",
    connection: null,
    pendingJoinRequests: [],
    connectedListeners: [],
    lastDelivery: null,
    recoverableAction: null,
    playbackControlsEnabled: false,
    transportWorkerRunning: true,
    transportError: null,
    playbackPositionMs: "0",
    streamEndedNaturally: false,
    audioSource: null,
    broadcast: null,
    lastError: null,
    monitor: { enabled: false, active: false, failureReason: null },
    ...overrides,
  };
}

const ACCEPTED_AT = 1_000_000;

function accepted(current: HostSessionSnapshotDto): PendingOperation {
  return {
    kind: "approve",
    targetDeviceId: "device-1",
    acceptedAtRevision: "9",
    acceptedAtMs: ACCEPTED_AT,
    baselineDelivery: deliveryKey(current),
    baselineError: errorKey(current.lastError),
  };
}

// Block 52.4: an accepted approval whose confirmation never arrived stayed on
// "Waiting for approval delivery confirmation" indefinitely.
describe("operationFailure delivery confirmation timeout", () => {
  it("keeps waiting inside the confirmation window", () => {
    const next = snapshot();
    expect(
      operationFailure(next, accepted(next), ACCEPTED_AT + DELIVERY_CONFIRMATION_TIMEOUT_MS - 1),
    ).toBeNull();
  });

  it("reports a retryable failure once the window passes without confirmation", () => {
    const next = snapshot();
    const failure = operationFailure(
      next,
      accepted(next),
      ACCEPTED_AT + DELIVERY_CONFIRMATION_TIMEOUT_MS,
    );
    expect(failure?.code).toBe("desktop.delivery_confirmation_timeout");
    expect(failure?.retryable).toBe(true);
  });

  // An identical repeat failure leaves no new error or report to compare
  // against; only the timeout can end that wait.
  it("still times out when a repeated failure looks identical to the baseline", () => {
    const failedDelivery = {
      intendedPeers: 1,
      successfulPeers: 0,
      failedPeers: 1,
      severity: "partial_failure",
    };
    const lastError = {
      code: "core.transport_delivery_failed",
      subsystem: "transport",
      severity: "error",
      retryable: true,
      message: "transport control delivery failed for every intended recipient",
    };
    const next = snapshot({
      lastDelivery: failedDelivery as HostSessionSnapshotDto["lastDelivery"],
      lastError,
    });
    const operation = accepted(next);
    expect(operationFailure(next, operation, ACCEPTED_AT + 1)).toBeNull();
    expect(
      operationFailure(next, operation, ACCEPTED_AT + DELIVERY_CONFIRMATION_TIMEOUT_MS)?.code,
    ).toBe("desktop.delivery_confirmation_timeout");
  });

  it("never times out an operation the core has not accepted yet", () => {
    const next = snapshot();
    const submitting = { ...accepted(next), acceptedAtRevision: null, acceptedAtMs: null };
    expect(operationFailure(next, submitting, ACCEPTED_AT + 10 * 60_000)).toBeNull();
  });
});

describe("unconfirmed operation disappearance", () => {
  it("keeps a disconnect from being reported as confirmed delivery", () => {
    expect(unconfirmedDisappearanceError("Listener removal").code).toBe(
      "desktop.operation_confirmation_missing",
    );
  });
});

// Block 52.5: approved listeners were shown with the raw core state
// "connecting" -- including while playing in sync -- and read as stuck.
describe("listenerProgress", () => {
  it("says an approved listener is waiting for playback while stopped", () => {
    expect(listenerProgress({ syncConfidence: null }, "stopped")).toBe(
      "Approved — waiting for the host to start playback",
    );
  });

  it("says it is waiting for its first sync once a stream is running", () => {
    expect(listenerProgress({ syncConfidence: null }, "playing")).toBe(
      "Approved — waiting for its first clock sync",
    );
    expect(listenerProgress({ syncConfidence: null }, "buffering")).toBe(
      "Approved — waiting for its first clock sync",
    );
  });

  it("reports the listener as connected once it has reported sync", () => {
    expect(listenerProgress({ syncConfidence: "excellent" }, "playing")).toBe(
      "Connected — sync excellent",
    );
  });
});
