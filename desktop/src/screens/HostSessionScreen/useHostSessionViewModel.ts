import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import {
  approveJoinRequest,
  createHostInvitation,
  endHostSession,
  getHostSessionState,
  pauseHostPlayback,
  rejectJoinRequest,
  removeListener,
  resumeHostPlayback,
  setHostMonitorEnabled,
  startHostPlayback,
  stopHostPlayback,
} from "../../core/client";
import type {
  CommandReceiptDto,
  DesktopErrorDto,
  HostInvitationDto,
  HostSessionSnapshotDto,
} from "../../core/generated/desktop-bindings";
import {
  type DecisionKind,
  deliveryKey,
  errorKey,
  operationFailure,
  type PendingOperation,
  POLL_INTERVAL_MS,
} from "./domain";

export interface HostSessionViewModel {
  snapshot: HostSessionSnapshotDto | null;
  refreshFailure: DesktopErrorDto | null;
  refresh: () => void;
  endPending: boolean;
  endSession: () => void;
  announcement: string | null;
  copyStatus: string | null;
  copyValue: (label: string, value: string) => void;

  remember: Record<string, boolean>;
  setRemember: (updater: (current: Record<string, boolean>) => Record<string, boolean>) => void;
  decisionOperations: Record<string, PendingOperation>;
  decisionFailures: Record<string, DesktopErrorDto>;
  decide: (requestId: string, kind: DecisionKind) => void;

  selectedListenerId: string | null;
  setSelectedListenerId: (id: string | null) => void;
  removalOperations: Record<string, PendingOperation>;
  removalFailures: Record<string, DesktopErrorDto>;
  requestRemoval: (listenerId: string) => void;

  invitation: HostInvitationDto | null;
  invitationQrDataUrl: string | null;
  invitationPending: boolean;
  invitationError: DesktopErrorDto | null;
  refreshInvitation: () => void;

  playbackPending: boolean;
  controlPlayback: (action: "start" | "pause" | "resume" | "stop") => void;

  monitorPending: boolean;
  toggleMonitor: (enabled: boolean) => void;
}

export function useHostSessionViewModel(): HostSessionViewModel {
  const [snapshot, setSnapshot] = useState<HostSessionSnapshotDto | null>(null);
  const [refreshFailure, setRefreshFailure] = useState<DesktopErrorDto | null>(null);
  const [endPending, setEndPending] = useState(false);
  const [playbackPending, setPlaybackPending] = useState(false);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState<string | null>(null);
  const [remember, setRememberState] = useState<Record<string, boolean>>({});
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
  const [monitorPending, setMonitorPending] = useState(false);

  const snapshotRef = useRef(snapshot);
  const decisionOperationsRef = useRef(decisionOperations);
  const removalOperationsRef = useRef(removalOperations);

  const setRemember = useCallback(
    (updater: (current: Record<string, boolean>) => Record<string, boolean>) => {
      setRememberState(updater);
    },
    [],
  );

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

  const decide = useCallback(
    async (requestId: string, kind: DecisionKind) => {
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
    },
    [remember, updateDecisionOperations],
  );

  const requestRemoval = useCallback(
    async (listenerId: string) => {
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
    },
    [updateRemovalOperations],
  );

  const copyValue = useCallback(async (label: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopyStatus(`${label} copied.`);
    } catch {
      setCopyStatus(`Could not copy ${label.toLowerCase()}.`);
    }
  }, []);

  const refreshInvitation = useCallback(async () => {
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
  }, [invitationPending]);

  const endSession = useCallback(async () => {
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
  }, [endPending]);

  const controlPlayback = useCallback(
    async (action: "start" | "pause" | "resume" | "stop") => {
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
    },
    [playbackPending],
  );

  // Never affects whether playback itself keeps streaming to listeners --
  // a failed or disabled monitor is purely a local-listening concern.
  const toggleMonitor = useCallback(
    async (enabled: boolean) => {
      if (monitorPending) {
        return;
      }
      setMonitorPending(true);
      try {
        await setHostMonitorEnabled(enabled);
        setAnnouncement(enabled ? "Local monitor enabled." : "Local monitor disabled.");
      } catch (error) {
        setRefreshFailure(error as DesktopErrorDto);
        setAnnouncement("Local monitor preference failed to update.");
      } finally {
        setMonitorPending(false);
      }
    },
    [monitorPending],
  );

  return {
    snapshot,
    refreshFailure,
    refresh: () => void refresh(),
    endPending,
    endSession: () => void endSession(),
    announcement,
    copyStatus,
    copyValue: (label, value) => void copyValue(label, value),

    remember,
    setRemember,
    decisionOperations,
    decisionFailures,
    decide: (requestId, kind) => void decide(requestId, kind),

    selectedListenerId,
    setSelectedListenerId,
    removalOperations,
    removalFailures,
    requestRemoval: (listenerId) => void requestRemoval(listenerId),

    invitation,
    invitationQrDataUrl,
    invitationPending,
    invitationError,
    refreshInvitation: () => void refreshInvitation(),

    playbackPending,
    controlPlayback: (action) => void controlPlayback(action),

    monitorPending,
    toggleMonitor: (enabled) => void toggleMonitor(enabled),
  };
}
