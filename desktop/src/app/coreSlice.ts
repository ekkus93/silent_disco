import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type {
  BridgeLifecycleDto,
  CoreDiagnosticDto,
  CoreNotificationDto,
  CoreSnapshotDto,
  DesktopErrorDto,
  PlatformEffectDto,
} from "../core/generated/desktop-bindings";

const MAX_PENDING_COMMANDS = 128;
const MAX_ERRORS = 32;
const MAX_DIAGNOSTICS = 64;
const MAX_SNAPSHOT_REVISION = 18_446_744_073_709_551_615n;
const DECIMAL_REVISION = /^(0|[1-9]\d*)$/;

export interface PendingCommandReceipt {
  operationId: string;
  commandKind: string;
  submittedAtRevision: string | null;
  observedEffectKind: string | null;
}

export interface StaleNotificationCounters {
  snapshots: number;
  effects: number;
  commandReceipts: number;
  droppedErrors: number;
  droppedDiagnostics: number;
}

export interface CoreState {
  snapshot: CoreSnapshotDto | null;
  bridgeLifecycle: BridgeLifecycleDto;
  pendingCommandReceipts: Record<string, PendingCommandReceipt>;
  errors: DesktopErrorDto[];
  diagnostics: CoreDiagnosticDto[];
  staleNotifications: StaleNotificationCounters;
}

const initialState: CoreState = {
  snapshot: null,
  bridgeLifecycle: { kind: "closed" },
  pendingCommandReceipts: {},
  errors: [],
  diagnostics: [],
  staleNotifications: {
    snapshots: 0,
    effects: 0,
    commandReceipts: 0,
    droppedErrors: 0,
    droppedDiagnostics: 0,
  },
};

export function parseSnapshotRevision(value: string): bigint | null {
  if (!DECIMAL_REVISION.test(value) || value.length > 20) {
    return null;
  }
  const revision = BigInt(value);
  return revision <= MAX_SNAPSHOT_REVISION ? revision : null;
}

export function shouldAcceptSnapshot(current: string | null, incoming: string): boolean {
  const incomingRevision = parseSnapshotRevision(incoming);
  if (incomingRevision === null) {
    return false;
  }
  if (current === null) {
    return true;
  }
  const currentRevision = parseSnapshotRevision(current);
  return currentRevision !== null && incomingRevision > currentRevision;
}

function frontendError(
  code: string,
  severity: string,
  retryable: boolean,
  message: string,
): DesktopErrorDto {
  return {
    code,
    subsystem: "frontend_bridge",
    severity,
    retryable,
    message,
  };
}

function pushBounded<T>(items: T[], value: T, limit: number): boolean {
  const dropped = items.length >= limit;
  if (dropped) {
    items.shift();
  }
  items.push(value);
  return dropped;
}

function recordError(state: CoreState, error: DesktopErrorDto): void {
  if (pushBounded(state.errors, error, MAX_ERRORS)) {
    state.staleNotifications.droppedErrors += 1;
  }
}

type SnapshotAcceptance = "accepted" | "stale" | "invalid";

function settleCommandsFromSnapshot(state: CoreState, revision: string): void {
  const incomingRevision = parseSnapshotRevision(revision);
  if (incomingRevision === null) {
    return;
  }
  for (const [operationId, pending] of Object.entries(state.pendingCommandReceipts)) {
    const submittedRevision =
      pending.submittedAtRevision === null
        ? null
        : parseSnapshotRevision(pending.submittedAtRevision);
    if (submittedRevision !== null && incomingRevision > submittedRevision) {
      delete state.pendingCommandReceipts[operationId];
    }
  }
}

function acceptSnapshot(
  state: CoreState,
  snapshot: CoreSnapshotDto,
  countStaleNotification: boolean,
): SnapshotAcceptance {
  if (parseSnapshotRevision(snapshot.revision) === null) {
    recordError(
      state,
      frontendError(
        "desktop.frontend.invalid_snapshot_revision",
        "fatal",
        false,
        "The desktop bridge delivered a snapshot with an invalid revision.",
      ),
    );
    return "invalid";
  }

  const currentRevision = state.snapshot?.revision ?? null;
  if (!shouldAcceptSnapshot(currentRevision, snapshot.revision)) {
    if (countStaleNotification) {
      state.staleNotifications.snapshots += 1;
    }
    return "stale";
  }

  state.snapshot = snapshot;
  settleCommandsFromSnapshot(state, snapshot.revision);
  return "accepted";
}

function failFromLatestError(state: CoreState): void {
  const error =
    state.errors.at(-1) ??
    frontendError(
      "desktop.frontend.missing_failure_detail",
      "fatal",
      false,
      "The frontend entered a failed bridge state without an error detail.",
    );
  state.bridgeLifecycle = { kind: "failed", details: { error } };
}

function observeEffect(state: CoreState, effect: PlatformEffectDto): void {
  const pending = state.pendingCommandReceipts[effect.operationId];
  if (pending === undefined) {
    state.staleNotifications.effects += 1;
    return;
  }
  pending.observedEffectKind = effect.effectKind;
}

export const coreSlice = createSlice({
  name: "core",
  initialState,
  reducers: {
    bridgeOpening(state, action: PayloadAction<{ profileId: string }>) {
      state.snapshot = null;
      state.pendingCommandReceipts = {};
      state.bridgeLifecycle = {
        kind: "opening",
        details: { profile_id: action.payload.profileId },
      };
    },
    bridgeReady(state, action: PayloadAction<{ profileId: string; snapshot: CoreSnapshotDto }>) {
      const lifecycle = state.bridgeLifecycle;
      if (
        lifecycle.kind !== "opening" ||
        lifecycle.details.profile_id !== action.payload.profileId
      ) {
        recordError(
          state,
          frontendError(
            "desktop.frontend.unexpected_bridge_ready",
            "error",
            false,
            "An out-of-sequence bridge-ready result was rejected.",
          ),
        );
        return;
      }
      const acceptance = acceptSnapshot(state, action.payload.snapshot, false);
      if (acceptance === "invalid") {
        failFromLatestError(state);
        return;
      }
      state.bridgeLifecycle = {
        kind: "ready",
        details: { profile_id: action.payload.profileId },
      };
    },
    bridgeFailed(state, action: PayloadAction<DesktopErrorDto>) {
      state.snapshot = null;
      state.pendingCommandReceipts = {};
      state.bridgeLifecycle = { kind: "failed", details: { error: action.payload } };
      recordError(state, action.payload);
    },
    bridgeClosed(state) {
      state.bridgeLifecycle = { kind: "closed" };
      state.snapshot = null;
      state.pendingCommandReceipts = {};
    },
    notificationReceived(state, action: PayloadAction<CoreNotificationDto>) {
      const notification = action.payload;
      switch (notification.kind) {
        case "snapshot":
          if (acceptSnapshot(state, notification.details, true) === "invalid") {
            failFromLatestError(state);
          }
          break;
        case "effect":
          observeEffect(state, notification.details);
          break;
        case "error":
          state.pendingCommandReceipts = {};
          recordError(state, notification.details);
          break;
        case "diagnostic":
          if (pushBounded(state.diagnostics, notification.details, MAX_DIAGNOSTICS)) {
            state.staleNotifications.droppedDiagnostics += 1;
          }
          break;
      }
    },
    commandPending(
      state,
      action: PayloadAction<{
        operationId: string;
        commandKind: string;
        submittedAtRevision?: string;
      }>,
    ) {
      const { operationId, commandKind } = action.payload;
      const submittedAtRevision =
        action.payload.submittedAtRevision ?? state.snapshot?.revision ?? null;
      const currentRevision = state.snapshot?.revision ?? null;
      if (
        submittedAtRevision !== null &&
        currentRevision !== null &&
        shouldAcceptSnapshot(submittedAtRevision, currentRevision)
      ) {
        return;
      }
      if (state.pendingCommandReceipts[operationId] !== undefined) {
        state.staleNotifications.commandReceipts += 1;
        recordError(
          state,
          frontendError(
            "desktop.frontend.duplicate_pending_command",
            "error",
            false,
            "A pending command was registered more than once.",
          ),
        );
        return;
      }
      if (Object.keys(state.pendingCommandReceipts).length >= MAX_PENDING_COMMANDS) {
        recordError(
          state,
          frontendError(
            "desktop.frontend.pending_command_overflow",
            "fatal",
            false,
            "The bounded pending-command table is full.",
          ),
        );
        return;
      }
      state.pendingCommandReceipts[operationId] = {
        operationId,
        commandKind,
        submittedAtRevision,
        observedEffectKind: null,
      };
    },
    commandReceiptObserved(state, action: PayloadAction<{ operationId: string }>) {
      const { operationId } = action.payload;
      if (state.pendingCommandReceipts[operationId] === undefined) {
        state.staleNotifications.commandReceipts += 1;
        return;
      }
      delete state.pendingCommandReceipts[operationId];
    },
    commandInvocationFailed(state, action: PayloadAction<DesktopErrorDto>) {
      state.pendingCommandReceipts = {};
      recordError(state, action.payload);
    },
    commandFailureObserved(
      state,
      action: PayloadAction<{ operationId: string; error: DesktopErrorDto }>,
    ) {
      const { operationId, error } = action.payload;
      if (state.pendingCommandReceipts[operationId] === undefined) {
        state.staleNotifications.commandReceipts += 1;
      } else {
        delete state.pendingCommandReceipts[operationId];
      }
      recordError(state, error);
    },
  },
});

export const coreActions = coreSlice.actions;
export default coreSlice.reducer;
