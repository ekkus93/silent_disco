import { useCallback, useEffect, useState } from "react";

import { Button } from "../components/Button";
import { Detail } from "../components/Detail";
import { ErrorAlert } from "../components/ErrorAlert";
import { Panel } from "../components/Panel";
import {
  getHostDiagnostics,
  getLabModeAvailable,
  getStorageInspection,
  toDesktopBridgeError,
} from "../core/client";
import type {
  DesktopDiagnosticsDto,
  DesktopErrorDto,
  StorageInspectionDto,
} from "../core/generated/desktop-bindings";
import { HostNetworkPolicyCard } from "./HostNetworkPolicyCard";

/// App-wide configuration.
///
/// This screen exists so configuration that outlives a session is not trapped
/// inside the host-setup form, which unmounts once a session starts. In
/// particular the active binding and mDNS advertising status were previously
/// unreachable while hosting -- "is mDNS advertising working?" could not be
/// answered during the session it mattered for.
///
/// It is reachable in every lifecycle, including when the bridge failed to
/// open, and renders its own failure state rather than being gated away.
export function SettingsScreen() {
  const [storage, setStorage] = useState<StorageInspectionDto | null>(null);
  const [diagnostics, setDiagnostics] = useState<DesktopDiagnosticsDto | null>(null);
  const [storageError, setStorageError] = useState<DesktopErrorDto | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<DesktopErrorDto | null>(null);
  const [labAvailable, setLabAvailable] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(() => {
    setLoading(true);
    // The two sources fail independently: a storage failure must not hide the
    // version information, and vice versa.
    const storageLoad = getStorageInspection()
      .then((next) => {
        setStorage(next);
        setStorageError(null);
      })
      .catch((error: unknown) => {
        setStorage(null);
        setStorageError(toDesktopBridgeError(error, "settings storage inspection"));
      });
    const diagnosticsLoad = getHostDiagnostics()
      .then((next) => {
        setDiagnostics(next);
        setDiagnosticsError(null);
      })
      .catch((error: unknown) => {
        setDiagnostics(null);
        setDiagnosticsError(toDesktopBridgeError(error, "settings diagnostics"));
      });
    // Fail-closed, matching the shell: a transport failure leaves this at
    // `null` and it is never reported as available.
    const labLoad = getLabModeAvailable()
      .then((available) => setLabAvailable(available))
      .catch(() => setLabAvailable(null));
    void Promise.allSettled([storageLoad, diagnosticsLoad, labLoad]).then(() => setLoading(false));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const settings = storage?.settings ?? null;

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-6 p-6 text-slate-100">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-2xl font-semibold">Settings</h2>
        <Button onClick={load} disabled={loading}>
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>

      {/* Network policy is machine configuration, not session configuration.
          Hosting it here is what makes the active binding and mDNS status
          observable during a live session. */}
      <HostNetworkPolicyCard available disabled={false} />

      <Panel title="Tuning">
        {storageError ? (
          <ErrorAlert error={storageError} title="Stored settings unavailable" />
        ) : null}
        {!storageError && settings === null ? (
          <p className="text-sm text-slate-300">
            {loading
              ? "Loading stored settings…"
              : "No tuning values have been persisted for this profile yet."}
          </p>
        ) : null}
        {settings ? (
          <>
            <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              <Detail label="Sync sample window" value={String(settings.syncSampleWindow)} />
              <Detail label="Sync cadence (ms)" value={settings.syncCadenceMs} />
              <Detail label="Startup buffer (ms)" value={settings.startupBufferMs} />
              <Detail label="Late packet threshold (ms)" value={settings.latePacketThresholdMs} />
              <Detail label="Hard resync threshold (ms)" value={settings.hardResyncThresholdMs} />
              <Detail label="Sync drift threshold" value={String(settings.syncDriftThresholdMs)} />
              <Detail label="Scan window (ms)" value={settings.scanWindowMs} />
              <Detail label="Updated at (ms)" value={settings.updatedAtMs} wrap="all" />
            </dl>
            <p className="mt-3 text-xs text-slate-400">
              Read-only. Editing requires a typed tuning command, which does not exist yet.
            </p>
          </>
        ) : null}
      </Panel>

      <Panel title="Trusted devices">
        {storage && storage.trustedDevices.length === 0 ? (
          <p className="text-sm text-slate-300">
            No devices have been trusted for this profile yet.
          </p>
        ) : null}
        {storage && storage.trustedDevices.length > 0 ? (
          <ul className="flex flex-col gap-2">
            {storage.trustedDevices.map((device) => (
              <li
                key={device.deviceId}
                className="rounded-lg bg-slate-900 p-3 text-sm text-slate-100"
              >
                <p className="font-semibold">{device.displayName}</p>
                <p className="mt-1 break-all font-mono text-xs text-slate-400">{device.deviceId}</p>
                <p className="mt-1 text-xs text-slate-300">
                  {device.trustState}
                  {device.hasPublicKey ? " · public key stored" : " · no public key"}
                </p>
              </li>
            ))}
          </ul>
        ) : null}
        {!storage && !storageError ? (
          <p className="text-sm text-slate-300">Loading trusted devices…</p>
        ) : null}
      </Panel>

      <Panel title="About">
        {diagnosticsError ? (
          <ErrorAlert error={diagnosticsError} title="Version information unavailable" />
        ) : null}
        {diagnostics ? (
          <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <Detail label="App version" value={diagnostics.versions.appVersion} />
            <Detail
              label="Export schema version"
              value={String(diagnostics.versions.exportSchemaVersion)}
            />
            <Detail label="Profile" value={diagnostics.profile.profileId} wrap="all" />
            <Detail label="Platform" value={diagnostics.profile.platform} />
            <Detail
              label="Core version"
              value={`${diagnostics.versions.coreVersion.major}.${diagnostics.versions.coreVersion.minor}.${diagnostics.versions.coreVersion.patch}`}
            />
            <Detail
              label="Lab Mode build"
              value={
                labAvailable === null ? "unknown" : labAvailable ? "available" : "not available"
              }
            />
          </dl>
        ) : null}
        {!diagnostics && !diagnosticsError ? (
          <p className="text-sm text-slate-300">Loading version information…</p>
        ) : null}
      </Panel>
    </div>
  );
}
