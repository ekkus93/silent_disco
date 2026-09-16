export type StatusSeverity = "neutral" | "good" | "warning" | "critical";

// Every severity pairs a text marker with its own class -- color is never the
// only signal (Block 35.2 "no color-only communication"), matching the
// convention already used by the diagnostics findings list.
const SEVERITY_MARKER: Record<StatusSeverity, string | null> = {
  neutral: null,
  good: "OK",
  warning: "ATTENTION",
  critical: "FAILED",
};

const SEVERITY_CLASS: Record<StatusSeverity, string> = {
  neutral: "border-slate-700 bg-slate-900",
  good: "border-emerald-500/40 bg-emerald-950/20",
  warning: "border-amber-500/50 bg-amber-950/30",
  critical: "border-rose-500/60 bg-rose-950/40",
};

const MARKER_CLASS: Record<StatusSeverity, string> = {
  neutral: "",
  good: "text-emerald-200",
  warning: "text-amber-200",
  critical: "text-rose-200",
};

/// A labeled status value.
///
/// `severity` exists because the previous implementation rendered every state
/// in identical neutral gray -- a failed transport and a healthy streaming
/// transport were visually indistinguishable.
export function StatusCard({
  label,
  value,
  severity = "neutral",
}: {
  label: string;
  value: string;
  severity?: StatusSeverity;
}) {
  const marker = SEVERITY_MARKER[severity];
  return (
    <div className={`rounded-xl border p-4 ${SEVERITY_CLASS[severity]}`}>
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</p>
      <p className="mt-2 break-words text-lg font-semibold">{value}</p>
      {marker ? (
        <p className={`mt-1 text-xs font-bold uppercase tracking-wide ${MARKER_CLASS[severity]}`}>
          {marker}
        </p>
      ) : null}
    </div>
  );
}
