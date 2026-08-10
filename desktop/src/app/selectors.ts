import { createSelector } from "@reduxjs/toolkit";

import type { RootState } from "./store";

export const selectCoreSnapshot = (state: RootState) => state.core.snapshot;
export const selectBridgeLifecycle = (state: RootState) => state.core.bridgeLifecycle;
const selectPendingCommandReceiptMap = (state: RootState) => state.core.pendingCommandReceipts;
export const selectPendingCommandReceipts = createSelector(
  [selectPendingCommandReceiptMap],
  (receipts) => Object.values(receipts),
);
export const selectCoreErrors = (state: RootState) => state.core.errors;
export const selectCoreDiagnostics = (state: RootState) => state.core.diagnostics;
export const selectStaleNotificationCounters = (state: RootState) => state.core.staleNotifications;
export const selectLatestCoreError = (state: RootState) => state.core.errors.at(-1) ?? null;
export const selectAuthoritativeRevision = (state: RootState) =>
  state.core.snapshot?.revision ?? null;
export const selectActivePanel = (state: RootState) => state.ui.activePanel;
export const selectDiagnosticsExpanded = (state: RootState) => state.ui.diagnosticsExpanded;
export const selectLabModeAvailable = (state: RootState) => state.lab.available;
