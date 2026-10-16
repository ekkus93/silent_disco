import QRCode from "qrcode";
import { useCallback, useEffect, useRef, useState } from "react";
import { selectLatestCoreError } from "../../app/selectors";
import { useAppSelector } from "../../app/store";
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
  revisionIsNewer,
  unconfirmedDisappearanceError,
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
  const latestCoreError = useAppSelector(selectLatestCoreError);

  const snapshotRef = useRef(snapshot);
  const endAcceptedAtRevisionRef = useRef<string | null>(null);
  const endOperationIdRef = useRef<string | null>(null);
  const endRevisionRetryRef = useRef(false);
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
        if (operation.acceptedAtRevision === null) {
          continue;
        }
        if (!pendingIds.has(requestId)) {
          const approvedListenerIsPresent =
            operation.kind === "approve" &&
            operation.targetDeviceId.length > 0 &&
            listenerIds.has(operation.targetDeviceId);
          delete decisions[requestId];
          if (approvedListenerIsPresent) {
            setDecisionFailures((current) => {
              const failures = { ...current };
              delete failures[requestId];
              return failures;
            });
            setAnnouncement("Listener joined the session.");
          } else if (
            operation.kind === "reject" &&
            operation.operationId !== null &&
            next.lastDeliveryOperationId === operation.operationId &&
            (next.lastDelivery?.successfulPeers ?? 0) > 0
          ) {
            setDecisionFailures((current) => {
              const failures = { ...current };
              delete failures[requestId];
              return failures;
            });
            setAnnouncement("Join request declined.");
          } else {
            const failure =
              operationFailure(next, operation) ??
              unconfirmedDisappearanceError(
                operation.kind === "approve" ? "The join request" : "The rejection",
              );
            setDecisionFailures((current) => ({ ...current, [requestId]: failure }));
            setAnnouncement("The join request disappeared before its decision was confirmed.");
          }
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
        if (operation.acceptedAtRevision === null) {
          continue;
        }
        if (!listenerIds.has(listenerId)) {
          delete removals[listenerId];
          const failure =
            operationFailure(next, operation) ??
            unconfirmedDisappearanceError("The listener removal");
          setRemovalFailures((current) => ({ ...current, [listenerId]: failure }));
          setAnnouncement("The listener disconnected before removal delivery was confirmed.");
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
      const previous = snapshotRef.current;
      if (previous && !revisionIsNewer(next.revision, previous.revision)) {
        return;
      }
      reconcile(next);
      snapshotRef.current = next;
      setSnapshot(next);
      setAnnouncement((current) => {
        const pendingPlaybackAction = current?.match(
          /^Playback (start|pause|resume|stop) requested\.$/,
        )?.[1];
        const completedPlaybackState =
          pendingPlaybackAction === "start" || pendingPlaybackAction === "resume"
            ? "playing"
            : pendingPlaybackAction === "pause"
              ? "paused"
              : pendingPlaybackAction === "stop"
                ? "stopped"
                : null;
        return completedPlaybackState === next.playbackState ? null : current;
      });
      const endAcceptedAtRevision = endAcceptedAtRevisionRef.current;
      if (
        endPending &&
        endAcceptedAtRevision !== null &&
        revisionIsNewer(next.revision, endAcceptedAtRevision) &&
        (next.hostLifecycle === "idle" || next.hostLifecycle === "error")
      ) {
        endAcceptedAtRevisionRef.current = null;
        endOperationIdRef.current = null;
        setEndPending(false);
      }
      setRefreshFailure(null);
    } catch (error) {
      setRefreshFailure(error as DesktopErrorDto);
    }
  }, [endPending, reconcile]);

  useEffect(() => {
    const operationId = endOperationIdRef.current;
    if (!endPending || operationId === null || latestCoreError?.operationId !== operationId) {
      return;
    }
    const isStaleRevision =
      latestCoreError.code === "core.invalid_state_transition" &&
      latestCoreError.message.startsWith("command expected snapshot revision ");
    if (isStaleRevision && !endRevisionRetryRef.current) {
      endRevisionRetryRef.current = true;
      void getHostSessionState()
        .then(async (next) => {
          reconcile(next);
          snapshotRef.current = next;
          setSnapshot(next);
          if (next.hostLifecycle === "idle" || next.hostLifecycle === "error") {
            endOperationIdRef.current = null;
            endAcceptedAtRevisionRef.current = null;
            setEndPending(false);
            setRefreshFailure(latestCoreError);
            return;
          }
          const receipt = await endHostSession(next.revision);
          endAcceptedAtRevisionRef.current = receipt.acceptedAtRevision;
          endOperationIdRef.current = receipt.operationId;
          setAnnouncement("End-session request queued.");
        })
        .catch((error: unknown) => {
          endOperationIdRef.current = null;
          endAcceptedAtRevisionRef.current = null;
          setRefreshFailure(error as DesktopErrorDto);
          setEndPending(false);
        });
      return;
    }
    endOperationIdRef.current = null;
    endAcceptedAtRevisionRef.current = null;
    setRefreshFailure(latestCoreError);
    setEndPending(false);
  }, [endPending, latestCoreError, reconcile]);

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
    QRCode.toDataURL(invitation.payload, {
      errorCorrectionLevel: "M",
      margin: 4,
      width: 1024,
    })
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
        targetDeviceId:
          current.pendingJoinRequests.find((request) => request.requestId === requestId)
            ?.deviceId ?? "",
        operationId: null,
        acceptedAtRevision: null,
        acceptedAtMs: null,
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
          operationId: receipt.operationId,
          acceptedAtRevision: receipt.acceptedAtRevision,
          acceptedAtMs: Date.now(),
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
        const latest = snapshotRef.current;
        if (latest && revisionIsNewer(latest.revision, receipt.acceptedAtRevision)) {
          reconcile(latest);
        }
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
    [remember, reconcile, updateDecisionOperations],
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
        targetDeviceId: listenerId,
        operationId: null,
        acceptedAtRevision: null,
        acceptedAtMs: null,
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
            operationId: receipt.operationId,
            acceptedAtRevision: receipt.acceptedAtRevision,
            acceptedAtMs: Date.now(),
          },
        });
        setAnnouncement("Disconnect queued. Waiting for delivery confirmation.");
        const latest = snapshotRef.current;
        if (latest && revisionIsNewer(latest.revision, receipt.acceptedAtRevision)) {
          reconcile(latest);
        }
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
    [reconcile, updateRemovalOperations],
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
    endRevisionRetryRef.current = false;
    try {
      const receipt = await endHostSession(current.revision);
      endAcceptedAtRevisionRef.current = receipt.acceptedAtRevision;
      endOperationIdRef.current = receipt.operationId;
      setAnnouncement("End-session request queued.");
    } catch (error) {
      setRefreshFailure(error as DesktopErrorDto);
      endAcceptedAtRevisionRef.current = null;
      endOperationIdRef.current = null;
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
