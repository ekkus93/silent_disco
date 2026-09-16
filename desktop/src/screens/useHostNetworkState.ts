import { useCallback, useEffect, useState } from "react";

import { getHostNetworkState, toDesktopBridgeError } from "../core/client";
import type { NetworkInterfaceSnapshotDto } from "../core/generated/desktop-bindings";

/// Whether the host may create a session with the current network policy.
///
/// Exported so the readiness rule exists exactly once: host setup gates
/// `canCreateHostSession` on it while Settings owns the control that changes
/// it, and the two must not drift apart.
export function networkReadiness(
  snapshot: NetworkInterfaceSnapshotDto | null,
  available: boolean,
): boolean {
  if (!available || snapshot === null) {
    return false;
  }
  return (
    snapshot.resolvedSelection !== null &&
    !snapshot.requiresExplicitSelection &&
    snapshot.selectionError === null &&
    (snapshot.activeBinding === null || snapshot.activeBindingValid)
  );
}

export interface HostNetworkState {
  snapshot: NetworkInterfaceSnapshotDto | null;
  loading: boolean;
  error: string | null;
  ready: boolean;
  refresh: () => Promise<void>;
  setSnapshot: (snapshot: NetworkInterfaceSnapshotDto) => void;
  setError: (message: string | null) => void;
}

/// Fetches the authoritative host network snapshot.
///
/// Both the Settings control and host setup's read-only summary use this, so
/// neither builds its own idea of what the network state is. Exactly one of
/// those screens is mounted at a time, so this does not double-poll.
export function useHostNetworkState(available: boolean): HostNetworkState {
  const [snapshot, setSnapshot] = useState<NetworkInterfaceSnapshotDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!available) {
      setSnapshot(null);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setSnapshot(await getHostNetworkState());
    } catch (cause: unknown) {
      setSnapshot(null);
      setError(toDesktopBridgeError(cause, "inspect host network interfaces").message);
    } finally {
      setLoading(false);
    }
  }, [available]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return {
    snapshot,
    loading,
    error,
    ready: networkReadiness(snapshot, available),
    refresh,
    setSnapshot,
    setError,
  };
}
