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
  if (!DECIMAL_REVISION.test(value)) {
    return null;
  }
  try {
    return BigInt(value);
  } catch {
    return null;
  }
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

function acceptSnapshot(state: CoreState, snapshot: CoreSnapshotDto): SnapshotAcceptance {
  const incomingRevision = parseSnapshotRevision(snapshot.revision);
  if (incomingRevision === null) {
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
    state.staleNotifications.snapshots += 1;
    return "stale";
  }

  state.snapshot = snapshot;
  return "accepted";
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
      state.bridgeLifecycle = {
        kind: "opening",
        details: { profile_id: action.payload.profileId },
      };
    },
    bridgeReady(
      state,
      action: PayloadAction<{ profileId: string; snapshot: CoreSnapshotDto }>,
    ) {
      const acceptance = acceptSnapshot(state, action.payload.snapshot);
      if (acceptance === "invalid") {
        const error = state.errors.at(-1);
        if (error !== undefined) {
          state.bridgeLifecycle = { kind: "failed", details: { error } };
        }
        return;
      }
      state.bridgeLifecycle = {
        kind: "ready",
        details: { profile_id: action.payload.profileId },
      };
    },
    bridgeFailed(state, action: PayloadAction<DesktopErrorDto>) {
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
          acceptSnapshot(state, notification.details);
          break;
        case "effect":
          observeEffect(state, notification.details);
          break;
        case "error":
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
      action: PayloadAction<{ operationId: string; commandKind: string }>,
    ) {
      const { operationId, commandKind } = action.payload;
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
        submittedAtRevision: state.snapshot?.revision ?? null,
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
