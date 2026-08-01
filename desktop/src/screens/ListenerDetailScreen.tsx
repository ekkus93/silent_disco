import type {
  ConnectedListenerDto,
  DeliveryReportDto,
  DesktopErrorDto,
} from "../core/generated/desktop-bindings";

interface ListenerDetailScreenProps {
  listener: ConnectedListenerDto;
  lastDelivery: DeliveryReportDto | null;
  pending: boolean;
  failure: DesktopErrorDto | null;
  onRemove: () => void;
  onBack: () => void;
}

function formatDuration(value: string | null): string {
  if (value === null) {
    return "Not observed";
  }
  const milliseconds = Number(value);
  if (!Number.isFinite(milliseconds)) {
    return `${value} ms`;
  }
  if (milliseconds < 1_000) {
    return `${Math.max(0, Math.round(milliseconds))} ms ago`;
  }
  return `${Math.max(0, Math.round(milliseconds / 1_000))} s ago`;
}

export function ListenerDetailScreen({
  listener,
  lastDelivery,
  pending,
  failure,
  onRemove,
  onBack,
}: ListenerDetailScreenProps) {
  return (
    <section
      aria-labelledby="listener-detail-title"
      className="rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.2em] text-cyan-300">
            Listener detail
          </p>
          <h2 id="listener-detail-title" className="mt-1 text-xl font-semibold text-white">
            {listener.displayName}
          </h2>
          <p className="mt-1 font-mono text-xs text-slate-400">{listener.deviceId}</p>
        </div>
        <button
          type="button"
          onClick={onBack}
          className="rounded-lg border border-slate-600 px-3 py-2 text-sm font-semibold text-slate-100 hover:border-slate-400"
        >
          Back to listeners
        </button>
      </div>

      <dl className="mt-5 grid gap-3 sm:grid-cols-2">
        <Detail label="Lifecycle" value={listener.transportState} />
        <Detail label="Trust" value={listener.trustState} />
        <Detail label="Last contact" value={formatDuration(listener.lastContactAgeMs)} />
        <Detail label="Sync confidence" value={listener.syncConfidence ?? "Not available"} />
        <Detail
          label="Estimated offset"
          value={
            listener.estimatedOffsetMs === null
              ? "Not available"
              : `${listener.estimatedOffsetMs} ms`
          }
        />
        <Detail
          label="Round-trip time"
          value={
            listener.roundTripTimeMs === null ? "Not available" : `${listener.roundTripTimeMs} ms`
          }
        />
        <Detail
          label="Clock drift"
          value={listener.driftPpm === null ? "Not available" : `${listener.driftPpm} ppm`}
        />
        <Detail label="Delivery state" value={listener.lastControlDeliveryState} />
        <Detail
          label="Retry capability"
          value={listener.retryAvailable ? "Available" : "Unavailable"}
        />
        <Detail
          label="Resync capability"
          value={listener.resyncAvailable ? "Available" : "Unavailable"}
        />
      </dl>

      {lastDelivery ? (
        <p className="mt-4 rounded-lg bg-slate-900 p-3 text-sm text-slate-300">
          Last host control delivery: {lastDelivery.successfulPeers} of {lastDelivery.intendedPeers}{" "}
          succeeded; {lastDelivery.failedPeers} failed ({lastDelivery.severity}).
        </p>
      ) : null}

      {listener.lastError ? (
        <ErrorAlert title="Listener failure" error={listener.lastError} />
      ) : null}
      {failure ? <ErrorAlert title="Removal failure" error={failure} /> : null}

      <div className="mt-5">
        <button
          type="button"
          onClick={onRemove}
          disabled={!listener.canRemove || pending}
          aria-describedby="remove-listener-help"
          className="rounded-lg border border-rose-500/70 px-4 py-2 text-sm font-semibold text-rose-100 hover:bg-rose-950/50 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {pending ? "Waiting for disconnect delivery…" : "Remove listener"}
        </button>
        <p id="remove-listener-help" className="mt-2 text-xs text-slate-400">
          The listener remains visible until the Rust core confirms delivery of the disconnect
          command.
        </p>
      </div>
    </section>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-slate-900 p-3">
      <dt className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</dt>
      <dd className="mt-1 break-words text-sm text-slate-100">{value}</dd>
    </div>
  );
}

function ErrorAlert({ title, error }: { title: string; error: DesktopErrorDto }) {
  return (
    <div
      role="alert"
      className="mt-4 rounded-lg border border-rose-500/60 bg-rose-950/40 p-3 text-sm text-rose-100"
    >
      <p className="font-semibold">{title}</p>
      <p className="mt-1">{error.message}</p>
      <p className="mt-1 font-mono text-xs">{error.code}</p>
    </div>
  );
}
