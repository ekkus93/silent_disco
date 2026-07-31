import { useCallback, useEffect, useMemo, useState } from "react";

import {
  getHostNetworkState,
  setHostNetworkPreference,
  toDesktopBridgeError,
} from "../core/client";
import type {
  NetworkAddressCandidateDto,
  NetworkInterfaceSnapshotDto,
  SetNetworkBindPreferenceRequest,
} from "../core/generated/desktop-bindings";

interface HostNetworkPolicyCardProps {
  available: boolean;
  disabled: boolean;
  onReadinessChange: (ready: boolean) => void;
}

const AUTOMATIC_VALUE = "automatic";

function candidateValue(candidate: NetworkAddressCandidateDto): string {
  return `${candidate.interfaceName}\u0000${candidate.address}`;
}

function candidateLabel(candidate: NetworkAddressCandidateDto): string {
  const route = candidate.isDefaultRoute ? " · default route" : "";
  const classLabel = candidate.classification.replaceAll("_", " ");
  return `${candidate.interfaceName} · ${candidate.address}/${candidate.prefixLength} · ${classLabel}${route}`;
}

function selectedValue(snapshot: NetworkInterfaceSnapshotDto | null): string {
  if (snapshot?.preference.mode !== "explicit") {
    return AUTOMATIC_VALUE;
  }
  const interfaceName = snapshot.preference.interfaceName;
  const address = snapshot.preference.address;
  if (interfaceName === null || address === null) {
    return AUTOMATIC_VALUE;
  }
  return `${interfaceName}\u0000${address}`;
}

function readiness(snapshot: NetworkInterfaceSnapshotDto | null, available: boolean): boolean {
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

export function HostNetworkPolicyCard({
  available,
  disabled,
  onReadinessChange,
}: HostNetworkPolicyCardProps) {
  const [snapshot, setSnapshot] = useState<NetworkInterfaceSnapshotDto | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
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

  useEffect(() => {
    onReadinessChange(readiness(snapshot, available));
  }, [available, onReadinessChange, snapshot]);

  const candidatesByValue = useMemo(() => {
    const map = new Map<string, NetworkAddressCandidateDto>();
    for (const candidate of snapshot?.candidates ?? []) {
      map.set(candidateValue(candidate), candidate);
    }
    return map;
  }, [snapshot]);

  const changePreference = async (value: string) => {
    const request: SetNetworkBindPreferenceRequest =
      value === AUTOMATIC_VALUE
        ? { mode: "automatic", interfaceName: null, address: null }
        : (() => {
            const candidate = candidatesByValue.get(value);
            if (candidate === undefined || !candidate.selectable) {
              throw new Error("The selected network address is no longer available.");
            }
            return {
              mode: "explicit",
              interfaceName: candidate.interfaceName,
              address: candidate.address,
            };
          })();

    setSaving(true);
    setError(null);
    try {
      setSnapshot(await setHostNetworkPreference(request));
    } catch (cause: unknown) {
      setError(toDesktopBridgeError(cause, "set host network preference").message);
      await refresh();
    } finally {
      setSaving(false);
    }
  };

  const controlsDisabled = disabled || loading || saving || snapshot?.activeBinding !== null;
  const resolved = snapshot?.resolvedSelection ?? null;

  return (
    <section
      aria-labelledby="host-network-policy-title"
      className="rounded-2xl border border-violet-200/15 bg-black/25 p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="host-network-policy-title" className="font-semibold text-cyan-200">
            Network interface policy
          </h3>
          <p className="mt-1 text-sm text-violet-100/65">
            Only an intentional private-LAN IPv4 address can be advertised. VPN, container,
            loopback, link-local, public, and IPv6 addresses are not selected automatically.
          </p>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={!available || controlsDisabled}
          className="rounded-lg border border-violet-300/30 px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-40"
        >
          Refresh
        </button>
      </div>

      {!available ? (
        <p className="mt-3 text-sm text-amber-200">
          Local-network capability has not been confirmed by the platform runner.
        </p>
      ) : loading && snapshot === null ? (
        <p role="status" className="mt-3 text-sm text-violet-100/70">
          Inspecting active network interfaces…
        </p>
      ) : snapshot !== null ? (
        <div className="mt-4 space-y-3">
          <div>
            <label htmlFor="host-network-address" className="block text-sm font-semibold">
              Host network interface
            </label>
            <select
              id="host-network-address"
              value={selectedValue(snapshot)}
              disabled={controlsDisabled}
              onChange={(event) => void changePreference(event.target.value)}
              className="mt-2 w-full rounded-xl border border-violet-200/25 bg-slate-950 px-3 py-2 text-sm"
            >
              <option value={AUTOMATIC_VALUE}>
                Automatic private-LAN selection
                {snapshot.requiresExplicitSelection ? " — explicit choice required" : ""}
              </option>
              {snapshot.candidates.map((candidate) => (
                <option
                  key={`${candidate.interfaceIndex}:${candidateValue(candidate)}`}
                  value={candidateValue(candidate)}
                  disabled={!candidate.selectable}
                >
                  {candidateLabel(candidate)}
                  {candidate.rejectionReason === null ? "" : ` — ${candidate.rejectionReason}`}
                </option>
              ))}
            </select>
          </div>

          {saving ? (
            <p role="status" className="text-sm text-violet-100/70">
              Validating the selected address against the current interface snapshot…
            </p>
          ) : null}

          {resolved !== null ? (
            <p className="text-sm text-emerald-200">
              Ready to bind {resolved.interfaceName} at {resolved.address}. Actual TCP and UDP ports
              are reported only after the shared transport binds successfully.
            </p>
          ) : null}

          {snapshot.activeBinding !== null ? (
            <p className="text-sm text-emerald-200">
              Bound {snapshot.activeBinding.interfaceName} at {snapshot.activeBinding.address} · TCP{" "}
              {snapshot.activeBinding.controlPort} · sync UDP {snapshot.activeBinding.syncPort} ·
              audio UDP {snapshot.activeBinding.audioPort}
            </p>
          ) : null}

          {snapshot.selectionError !== null ? (
            <p role="alert" className="text-sm text-amber-200">
              {snapshot.selectionError}
            </p>
          ) : null}

          {snapshot.interfaceChange !== null ? (
            <p role="alert" className="text-sm text-amber-200">
              {snapshot.interfaceChange}
            </p>
          ) : null}
        </div>
      ) : null}

      {error !== null ? (
        <p role="alert" className="mt-3 text-sm text-red-200">
          {error}
        </p>
      ) : null}
    </section>
  );
}
