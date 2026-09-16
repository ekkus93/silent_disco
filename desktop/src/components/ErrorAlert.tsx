import type { DesktopErrorDto } from "../core/generated/desktop-bindings";

/// The single error rendering for the whole app.
///
/// It replaces five divergent implementations that each dropped a different
/// subset of `DesktopErrorDto`. Every field is rendered, including
/// `retryable` -- the one field that tells an operator whether trying again
/// is worth anything, and which no previous renderer showed.
export function ErrorAlert({
  error,
  title,
  observedAtMs,
  onDismiss,
}: {
  error: DesktopErrorDto;
  title?: string;
  /// When the failure was observed. A sticky banner with no timestamp cannot
  /// be distinguished from a live one.
  observedAtMs?: number;
  onDismiss?: () => void;
}) {
  return (
    <div
      role="alert"
      className="rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          {title ? <p className="text-sm font-bold uppercase tracking-wide">{title}</p> : null}
          <p className={`font-semibold ${title ? "mt-1" : ""}`}>{error.message}</p>
        </div>
        {onDismiss ? (
          <button
            type="button"
            onClick={onDismiss}
            className="rounded-lg border border-rose-300/50 px-3 py-1 text-xs font-semibold hover:border-rose-200"
          >
            Dismiss
          </button>
        ) : null}
      </div>
      <dl className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-xs">
        <div className="flex gap-1.5">
          <dt className="text-rose-200/70">Code</dt>
          <dd className="font-mono">{error.code}</dd>
        </div>
        <div className="flex gap-1.5">
          <dt className="text-rose-200/70">Subsystem</dt>
          <dd className="font-mono">{error.subsystem}</dd>
        </div>
        <div className="flex gap-1.5">
          <dt className="text-rose-200/70">Severity</dt>
          <dd className="font-mono">{error.severity}</dd>
        </div>
        <div className="flex gap-1.5">
          <dt className="text-rose-200/70">Retryable</dt>
          <dd className="font-mono">{error.retryable ? "yes" : "no"}</dd>
        </div>
        {observedAtMs === undefined ? null : (
          <div className="flex gap-1.5">
            <dt className="text-rose-200/70">Observed</dt>
            <dd className="font-mono">{new Date(observedAtMs).toLocaleTimeString()}</dd>
          </div>
        )}
      </dl>
    </div>
  );
}
