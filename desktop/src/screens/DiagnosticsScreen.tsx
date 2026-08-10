import { useCallback, useEffect, useMemo, useState } from "react";
import { exportHostDiagnostics, getHostDiagnostics } from "../core/client";
import type { DesktopDiagnosticsDto, DesktopErrorDto } from "../core/generated/desktop-bindings";

const POLL_INTERVAL_MS = 2_000;
// A poll that hasn't produced a fresh snapshot within this window is shown
// as stale (Block 35.2 "clear stale-data indicator") -- more than double
// the poll interval, so one slow/dropped tick alone does not flip it.
const STALE_AFTER_MS = 5_000;

type FindingSeverity = "fatal" | "error" | "warning" | "info";

interface Finding {
  id: string;
  subsystem: string;
  severity: FindingSeverity;
  message: string;
}

const SEVERITY_LABEL: Record<FindingSeverity, string> = {
  fatal: "FATAL",
  error: "ERROR",
  warning: "WARNING",
  info: "OK",
};

// Every severity pairs a text label (above) with its own class -- color is
// never the only signal (Block 35.2 "no color-only communication").
const SEVERITY_CLASS: Record<FindingSeverity, string> = {
  fatal: "border-rose-500/70 bg-rose-950/50 text-rose-100",
  error: "border-rose-500/50 bg-rose-950/30 text-rose-100",
  warning: "border-amber-500/50 bg-amber-950/30 text-amber-100",
  info: "border-emerald-500/40 bg-emerald-950/20 text-emerald-100",
};

function asSeverity(value: string): FindingSeverity {
  return value === "fatal" || value === "error" || value === "warning" ? value : "info";
}

/// Derives a bounded list of notable findings from the diagnostics
/// snapshot, each carrying a real subsystem and severity -- the basis for
/// this screen's severity/subsystem filters (Block 35.2). The full detail
/// sections below are never hidden by these filters; only this findings
/// list is.
function deriveFindings(dto: DesktopDiagnosticsDto): Finding[] {
  const findings: Finding[] = [];

  if (dto.lastError) {
    findings.push({
      id: "last-error",
      subsystem: dto.lastError.subsystem,
      severity: asSeverity(dto.lastError.severity),
      message: dto.lastError.message,
    });
  }
  if (dto.notificationBridge.deliveryFailure) {
    const failure = dto.notificationBridge.deliveryFailure;
    findings.push({
      id: "notification-bridge",
      subsystem: failure.subsystem,
      severity: asSeverity(failure.severity),
      message: `Notification delivery failed: ${failure.message}`,
    });
  }
  if (!dto.storage.available) {
    findings.push({
      id: "storage",
      subsystem: "storage",
      severity: "error",
      message: dto.storage.failureReason ?? "Storage is unavailable.",
    });
  }
  if (!dto.identity.deviceIdentityPresent || !dto.identity.signingIdentityPresent) {
    findings.push({
      id: "identity",
      subsystem: "identity",
      severity: "warning",
      message: "Device or signing identity is not available.",
    });
  }
  if (dto.transport.state === "failed" || dto.transport.state === "disconnected") {
    findings.push({
      id: "transport",
      subsystem: "transport",
      severity: dto.transport.state === "failed" ? "error" : "warning",
      message: `Transport is ${dto.transport.state}.`,
    });
  }
  if (dto.transport.broadcast && Number(dto.transport.broadcast.queueOverflows) > 0) {
    findings.push({
      id: "broadcast-overflow",
      subsystem: "transport",
      severity: "warning",
      message: `Broadcast queue overflowed ${dto.transport.broadcast.queueOverflows} time(s).`,
    });
  }
  if (dto.listenersTruncated) {
    findings.push({
      id: "listeners-truncated",
      subsystem: "runtime",
      severity: "warning",
      message: "The listener list was truncated to its bounded display limit.",
    });
  }
  if (dto.synchronization && dto.synchronization.confidence !== "high") {
    findings.push({
      id: "synchronization",
      subsystem: "synchronization",
      severity: dto.synchronization.confidence === "unknown" ? "warning" : "info",
      message: `Synchronization confidence is ${dto.synchronization.confidence}.`,
    });
  }
  if (dto.decodeQueue && Number(dto.decodeQueue.backpressureEvents) > 0) {
    findings.push({
      id: "decode-backpressure",
      subsystem: "audio",
      severity: "warning",
      message: `Decoder backpressure occurred ${dto.decodeQueue.backpressureEvents} time(s).`,
    });
  }
  if (dto.decodeQueue?.state === "failed") {
    findings.push({
      id: "decode-failed",
      subsystem: "audio",
      severity: "error",
      message: "The decoder worker failed.",
    });
  }
  if (dto.packetizeQueue && Number(dto.packetizeQueue.backpressureEvents) > 0) {
    findings.push({
      id: "packetize-backpressure",
      subsystem: "audio",
      severity: "warning",
      message: `Packetizer backpressure occurred ${dto.packetizeQueue.backpressureEvents} time(s).`,
    });
  }
  if (dto.monitor.enabled && !dto.monitor.active) {
    findings.push({
      id: "monitor",
      subsystem: "monitor",
      severity: "warning",
      message: dto.monitor.failureReason
        ? `Local monitor is enabled but not active: ${dto.monitor.failureReason}`
        : "Local monitor is enabled but not active.",
    });
  }
  if (dto.shuttingDown) {
    findings.push({
      id: "shutting-down",
      subsystem: "runtime",
      severity: "info",
      message: "The desktop runtime is shutting down.",
    });
  }

  if (findings.length === 0) {
    findings.push({
      id: "nominal",
      subsystem: "runtime",
      severity: "info",
      message: "No notable findings.",
    });
  }
  return findings;
}

function formatAge(ms: number): string {
  if (ms < 1_000) {
    return `${Math.max(0, Math.round(ms))} ms ago`;
  }
  return `${Math.round(ms / 1_000)} s ago`;
}

export function DiagnosticsScreen() {
  const [diagnostics, setDiagnostics] = useState<DesktopDiagnosticsDto | null>(null);
  const [fetchedAt, setFetchedAt] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [refreshFailure, setRefreshFailure] = useState<DesktopErrorDto | null>(null);
  const [severityFilter, setSeverityFilter] = useState<"all" | FindingSeverity>("all");
  const [subsystemFilter, setSubsystemFilter] = useState<string>("all");
  const [exportPending, setExportPending] = useState(false);
  const [exportStatus, setExportStatus] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await getHostDiagnostics();
      setDiagnostics(next);
      setFetchedAt(Date.now());
      setRefreshFailure(null);
    } catch (error) {
      // A failed fetch does not clear the last-known snapshot -- the stale
      // indicator (below) makes the age of that snapshot explicit instead.
      setRefreshFailure(error as DesktopErrorDto);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [refresh]);

  useEffect(() => {
    const interval = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, []);

  const findings = useMemo(() => (diagnostics ? deriveFindings(diagnostics) : []), [diagnostics]);
  const subsystems = useMemo(
    () => Array.from(new Set(findings.map((finding) => finding.subsystem))).sort(),
    [findings],
  );
  const filteredFindings = findings.filter(
    (finding) =>
      (severityFilter === "all" || finding.severity === severityFilter) &&
      (subsystemFilter === "all" || finding.subsystem === subsystemFilter),
  );

  const ageMs = fetchedAt === null ? null : nowMs - fetchedAt;
  const isStale = ageMs === null || ageMs > STALE_AFTER_MS;

  async function runExport() {
    if (exportPending) {
      return;
    }
    setExportPending(true);
    setExportStatus(null);
    try {
      const outcome = await exportHostDiagnostics();
      setExportStatus(
        outcome.kind === "saved"
          ? "Diagnostics export saved."
          : "Export cancelled; nothing was written.",
      );
    } catch (error) {
      setExportStatus(`Export failed: ${(error as DesktopErrorDto).message}`);
    } finally {
      setExportPending(false);
    }
  }

  async function copyJson() {
    if (!diagnostics) {
      return;
    }
    try {
      await navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2));
      setCopyStatus("Diagnostics JSON copied.");
    } catch {
      setCopyStatus("Could not copy diagnostics JSON.");
    }
  }

  if (!diagnostics) {
    return (
      <section className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5 text-slate-100">
        <h2 className="text-xl font-semibold">Diagnostics</h2>
        <p className="mt-3 text-sm text-slate-400">Loading diagnostics…</p>
        {refreshFailure ? <ErrorAlert error={refreshFailure} /> : null}
      </section>
    );
  }

  return (
    <section
      aria-labelledby="diagnostics-title"
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5 text-slate-100"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 id="diagnostics-title" className="text-xl font-semibold">
            Diagnostics
          </h2>
          <p role="status" className="mt-1 text-sm text-slate-400">
            {ageMs === null ? "No successful fetch yet." : `Snapshot captured ${formatAge(ageMs)}.`}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => void refresh()}
            className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-semibold hover:border-slate-400"
          >
            Refresh
          </button>
          <button
            type="button"
            onClick={() => void copyJson()}
            className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-semibold hover:border-slate-400"
          >
            Copy diagnostics JSON
          </button>
          <button
            type="button"
            onClick={() => void runExport()}
            disabled={exportPending}
            className="rounded-lg border border-cyan-500/60 px-4 py-2 text-sm font-semibold text-cyan-100 hover:bg-cyan-950/40 disabled:opacity-50"
          >
            {exportPending ? "Exporting…" : "Export to file…"}
          </button>
        </div>
      </div>

      <div aria-live="polite" aria-atomic="true" className="mt-2 min-h-5 text-sm text-cyan-200">
        {exportStatus ?? copyStatus}
      </div>

      {isStale ? (
        <p
          role="alert"
          className="mt-4 rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100"
        >
          STALE: this snapshot is {ageMs === null ? "of unknown age" : formatAge(ageMs)} and no
          newer one has arrived. Displayed data may not reflect current state.
        </p>
      ) : null}
      {refreshFailure ? <ErrorAlert error={refreshFailure} /> : null}

      <div className="mt-5 rounded-xl bg-slate-900 p-4">
        <div className="flex flex-wrap items-center gap-3">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-400">Findings</h3>
          <label className="flex items-center gap-2 text-xs text-slate-300">
            Severity
            <select
              value={severityFilter}
              onChange={(event) => setSeverityFilter(event.target.value as "all" | FindingSeverity)}
              className="rounded border border-slate-600 bg-slate-800 px-2 py-1"
            >
              <option value="all">All</option>
              <option value="fatal">Fatal</option>
              <option value="error">Error</option>
              <option value="warning">Warning</option>
              <option value="info">OK / info</option>
            </select>
          </label>
          <label className="flex items-center gap-2 text-xs text-slate-300">
            Subsystem
            <select
              value={subsystemFilter}
              onChange={(event) => setSubsystemFilter(event.target.value)}
              className="rounded border border-slate-600 bg-slate-800 px-2 py-1"
            >
              <option value="all">All</option>
              {subsystems.map((subsystem) => (
                <option key={subsystem} value={subsystem}>
                  {subsystem}
                </option>
              ))}
            </select>
          </label>
        </div>
        <ul className="mt-3 space-y-2">
          {filteredFindings.length === 0 ? (
            <li className="text-sm text-slate-400">No findings match the current filters.</li>
          ) : (
            filteredFindings.map((finding) => (
              <li
                key={finding.id}
                className={`rounded-lg border p-3 text-sm ${SEVERITY_CLASS[finding.severity]}`}
              >
                <span className="mr-2 font-mono text-xs font-semibold">
                  [{SEVERITY_LABEL[finding.severity]}]
                </span>
                <span className="mr-2 font-mono text-xs text-slate-300">{finding.subsystem}</span>
                {finding.message}
              </li>
            ))
          )}
        </ul>
      </div>

      <div className="mt-5 grid gap-4 md:grid-cols-2">
        <DetailCard title="Versions">
          <Detail
            label="Core version"
            value={`${diagnostics.versions.coreVersion.major}.${diagnostics.versions.coreVersion.minor}.${diagnostics.versions.coreVersion.patch}`}
          />
          <Detail label="App version" value={diagnostics.versions.appVersion} />
          <Detail label="Export schema" value={String(diagnostics.versions.exportSchemaVersion)} />
        </DetailCard>

        <DetailCard title="Profile">
          <Detail label="Profile ID" value={diagnostics.profile.profileId} />
          <Detail label="Platform" value={diagnostics.profile.platform} />
        </DetailCard>

        <DetailCard title="Storage">
          <Detail label="Available" value={diagnostics.storage.available ? "yes" : "no"} />
          <Detail
            label="Schema version"
            value={
              diagnostics.storage.schemaVersion === null
                ? "not available"
                : String(diagnostics.storage.schemaVersion)
            }
          />
          <Detail label="Journal mode" value={diagnostics.storage.journalMode ?? "not available"} />
          <Detail
            label="Integrity check"
            value={diagnostics.storage.integrityCheck ?? "not available"}
          />
          {diagnostics.storage.failureReason ? (
            <Detail label="Failure reason" value={diagnostics.storage.failureReason} />
          ) : null}
        </DetailCard>

        <DetailCard title="Identity">
          <Detail
            label="Device identity"
            value={diagnostics.identity.deviceIdentityPresent ? "present" : "not available"}
          />
          <Detail
            label="Signing identity"
            value={diagnostics.identity.signingIdentityPresent ? "present" : "not available"}
          />
          <Detail
            label="Signing key fingerprint"
            value={diagnostics.identity.signingKeyFingerprint ?? "not available"}
          />
        </DetailCard>

        <DetailCard title="Transport">
          <Detail label="State" value={diagnostics.transport.state} />
          {diagnostics.transport.broadcast ? (
            <>
              <Detail label="Queue depth" value={diagnostics.transport.broadcast.queueDepth} />
              <Detail
                label="Queue overflows"
                value={diagnostics.transport.broadcast.queueOverflows}
              />
            </>
          ) : (
            <Detail label="Broadcast" value="not available" />
          )}
        </DetailCard>

        <DetailCard title="Listeners">
          <Detail label="Count" value={String(diagnostics.listeners.length)} />
          <Detail label="Truncated" value={diagnostics.listenersTruncated ? "yes" : "no"} />
          {diagnostics.listeners.length > 0 ? (
            <ul className="mt-2 space-y-1 text-xs text-slate-300">
              {diagnostics.listeners.map((listener) => (
                <li key={listener.deviceId} className="font-mono">
                  {listener.displayName} · {listener.transportState} · {listener.trustState}
                  {listener.syncConfidence ? ` · sync: ${listener.syncConfidence}` : ""}
                </li>
              ))}
            </ul>
          ) : null}
        </DetailCard>

        {diagnostics.synchronization ? (
          <DetailCard title="Synchronization">
            <Detail label="Confidence" value={diagnostics.synchronization.confidence} />
            <Detail label="Offset (ms)" value={diagnostics.synchronization.offsetMs} />
            <Detail label="Round trip (ms)" value={diagnostics.synchronization.roundTripMs} />
            <Detail label="Drift (ppm)" value={diagnostics.synchronization.driftPpm} />
          </DetailCard>
        ) : null}

        {diagnostics.decodeQueue ? (
          <DetailCard title="Decode queue">
            <Detail label="State" value={diagnostics.decodeQueue.state} />
            <Detail
              label="Queued / capacity"
              value={`${diagnostics.decodeQueue.queuedChunks} / ${diagnostics.decodeQueue.queueCapacityChunks}`}
            />
            <Detail
              label="Backpressure events"
              value={diagnostics.decodeQueue.backpressureEvents}
            />
            <Detail label="Emitted frames" value={diagnostics.decodeQueue.emittedFrames} />
          </DetailCard>
        ) : null}

        {diagnostics.packetizeQueue ? (
          <DetailCard title="Packetize queue">
            <Detail
              label="Queued / capacity"
              value={`${diagnostics.packetizeQueue.queuedPackets} / ${diagnostics.packetizeQueue.queueCapacity}`}
            />
            <Detail
              label="Backpressure events"
              value={diagnostics.packetizeQueue.backpressureEvents}
            />
            <Detail label="Emitted packets" value={diagnostics.packetizeQueue.emittedPackets} />
          </DetailCard>
        ) : null}

        <DetailCard title="Local monitor">
          <Detail label="Enabled" value={diagnostics.monitor.enabled ? "yes" : "no"} />
          <Detail label="Active" value={diagnostics.monitor.active ? "yes" : "no"} />
          {diagnostics.monitor.callbackCount ? (
            <Detail label="Callback count" value={diagnostics.monitor.callbackCount} />
          ) : null}
          {diagnostics.monitor.framesWritten ? (
            <Detail label="Frames written" value={diagnostics.monitor.framesWritten} />
          ) : null}
          {diagnostics.monitor.failureReason ? (
            <Detail label="Failure reason" value={diagnostics.monitor.failureReason} />
          ) : null}
        </DetailCard>
      </div>

      {diagnostics.lastError ? (
        <div className="mt-4">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-400">
            Last error
          </h3>
          <ErrorAlert error={diagnostics.lastError} />
        </div>
      ) : null}
    </section>
  );
}

function DetailCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-xl bg-slate-900 p-4">
      <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-400">{title}</h3>
      <dl className="mt-2 space-y-1">{children}</dl>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-wrap justify-between gap-2 text-sm">
      <dt className="text-slate-400">{label}</dt>
      <dd className="break-all font-mono text-slate-100">{value}</dd>
    </div>
  );
}

function ErrorAlert({ error }: { error: DesktopErrorDto }) {
  return (
    <div
      role="alert"
      className="mt-4 rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
    >
      <p className="font-semibold">{error.message}</p>
      <p className="mt-1 font-mono text-xs">
        {error.code} · {error.subsystem} · {error.severity}
      </p>
    </div>
  );
}
