import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import type { CoreSnapshotDto, DesktopErrorDto } from "../core/generated/desktop-bindings";
import coreReducer, { coreActions, shouldAcceptSnapshot } from "./coreSlice";

function snapshot(revision: string, hostLifecycle = "idle"): CoreSnapshotDto {
  return {
    revision,
    selectedRole: null,
    hostLifecycle,
    listenerLifecycle: "idle",
    transportState: "idle",
    discoveryActive: false,
    discoveredSessionCount: 0,
    pendingJoinRequestCount: 0,
    listenerCount: 0,
    playbackState: "stopped",
    playbackPositionMs: "0",
    recoverableAction: null,
    lastError: null,
    shuttingDown: false,
  };
}

function readyState(initialSnapshot: CoreSnapshotDto) {
  const opening = coreReducer(undefined, coreActions.bridgeOpening({ profileId: "main" }));
  return coreReducer(
    opening,
    coreActions.bridgeReady({ profileId: "main", snapshot: initialSnapshot }),
  );
}

const commandError: DesktopErrorDto = {
  code: "core.command.rejected",
  subsystem: "runtime",
  severity: "error",
  retryable: false,
  message: "The command was rejected.",
};

describe("authoritative core slice", () => {
  it("stores the complete initial authoritative snapshot", () => {
    const state = readyState(snapshot("0"));

    expect(state.snapshot).toEqual(snapshot("0"));
    expect(state.bridgeLifecycle).toEqual({
      kind: "ready",
      details: { profile_id: "main" },
    });
  });

  it("accepts a newer revision and replaces the complete snapshot", () => {
    let state = readyState(snapshot("4", "idle"));
    state = coreReducer(
      state,
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot("5", "waiting_for_listeners"),
      }),
    );

    expect(state.snapshot).toEqual(snapshot("5", "waiting_for_listeners"));
    expect(state.staleNotifications.snapshots).toBe(0);
  });

  it("rejects an equal revision and increments the stale counter", () => {
    let state = readyState(snapshot("8"));
    state = coreReducer(
      state,
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot("8", "streaming"),
      }),
    );

    expect(state.snapshot).toEqual(snapshot("8"));
    expect(state.staleNotifications.snapshots).toBe(1);
    expect(shouldAcceptSnapshot("8", "8")).toBe(false);
  });

  it("rejects an older revision without regressing lifecycle state", () => {
    let state = readyState(snapshot("10", "streaming"));
    state = coreReducer(
      state,
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot("9", "idle"),
      }),
    );

    expect(state.snapshot).toEqual(snapshot("10", "streaming"));
    expect(state.staleNotifications.snapshots).toBe(1);
  });

  it("does not count the duplicate bootstrap snapshot as a stale notification", () => {
    let state = coreReducer(undefined, coreActions.bridgeOpening({ profileId: "main" }));
    state = coreReducer(
      state,
      coreActions.notificationReceived({ kind: "snapshot", details: snapshot("12") }),
    );
    state = coreReducer(
      state,
      coreActions.bridgeReady({ profileId: "main", snapshot: snapshot("12") }),
    );

    expect(state.bridgeLifecycle.kind).toBe("ready");
    expect(state.snapshot?.revision).toBe("12");
    expect(state.staleNotifications.snapshots).toBe(0);
  });

  it("accepts revision zero after a fresh bridge open resets the prior session", () => {
    let state = readyState(snapshot("31", "streaming"));
    state = coreReducer(
      state,
      coreActions.commandPending({
        operationId: "old-operation",
        commandKind: "end_host",
      }),
    );

    state = coreReducer(state, coreActions.bridgeOpening({ profileId: "main" }));
    expect(state.snapshot).toBeNull();
    expect(state.pendingCommandReceipts).toEqual({});

    state = coreReducer(
      state,
      coreActions.bridgeReady({ profileId: "main", snapshot: snapshot("0") }),
    );
    expect(state.snapshot?.revision).toBe("0");
    expect(state.bridgeLifecycle.kind).toBe("ready");
  });

  it("fails visibly when a notification has an invalid revision", () => {
    let state = readyState(snapshot("1"));
    state = coreReducer(
      state,
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot("18446744073709551616"),
      }),
    );

    expect(state.bridgeLifecycle.kind).toBe("failed");
    expect(state.errors.at(-1)?.code).toBe("desktop.frontend.invalid_snapshot_revision");
    expect(state.snapshot?.revision).toBe("1");
  });

  it("keeps a command pending until explicit core receipt evidence arrives", () => {
    let state = coreReducer(
      undefined,
      coreActions.commandPending({
        operationId: "operation-1",
        commandKind: "start_host",
      }),
    );
    state = coreReducer(
      state,
      coreActions.notificationReceived({ kind: "snapshot", details: snapshot("1") }),
    );
    state = coreReducer(
      state,
      coreActions.notificationReceived({
        kind: "effect",
        details: { operationId: "operation-1", effectKind: "start_advertising" },
      }),
    );

    expect(state.pendingCommandReceipts["operation-1"]?.observedEffectKind).toBe(
      "start_advertising",
    );

    state = coreReducer(state, coreActions.commandReceiptObserved({ operationId: "operation-1" }));
    expect(state.pendingCommandReceipts["operation-1"]).toBeUndefined();
  });

  it("records command failure visibly and removes the pending command", () => {
    let state = coreReducer(
      undefined,
      coreActions.commandPending({
        operationId: "operation-2",
        commandKind: "start_host",
      }),
    );
    state = coreReducer(
      state,
      coreActions.commandFailureObserved({
        operationId: "operation-2",
        error: commandError,
      }),
    );

    expect(state.pendingCommandReceipts["operation-2"]).toBeUndefined();
    expect(state.errors.at(-1)).toEqual(commandError);
  });

  it("rejects an out-of-sequence ready result with a visible error", () => {
    const state = coreReducer(
      undefined,
      coreActions.bridgeReady({ profileId: "main", snapshot: snapshot("0") }),
    );

    expect(state.bridgeLifecycle.kind).toBe("closed");
    expect(state.errors.at(-1)?.code).toBe("desktop.frontend.unexpected_bridge_ready");
  });

  it("keeps host lifecycle mutation out of frontend actions and transition helpers", () => {
    expect(Object.keys(coreActions)).not.toContain("setHostLifecycle");
    const source = readFileSync(new URL("./coreSlice.ts", import.meta.url), "utf8");
    expect(source).not.toMatch(/\b(?:advance|set|transition)HostLifecycle\b/);
  });
});
