import type { DesktopErrorDto } from "../../core/generated/desktop-bindings";

export function StatusCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-slate-700 bg-slate-900 p-4">
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</p>
      <p className="mt-2 break-words text-lg font-semibold">{value}</p>
    </div>
  );
}

export function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-slate-900 p-3">
      <dt className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</dt>
      <dd className="mt-1 break-all text-sm text-slate-100">{value}</dd>
    </div>
  );
}

export function CopyButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-lg border border-slate-600 px-3 py-2 text-sm font-semibold hover:border-slate-400"
    >
      {label}
    </button>
  );
}

export function ErrorAlert({ error }: { error: DesktopErrorDto }) {
  return (
    <div
      role="alert"
      className="mt-4 rounded-xl border border-rose-500/60 bg-rose-950/40 p-4 text-rose-100"
    >
      <p className="font-semibold">{error.message}</p>
      <p className="mt-1 font-mono text-xs">{error.code}</p>
    </div>
  );
}
