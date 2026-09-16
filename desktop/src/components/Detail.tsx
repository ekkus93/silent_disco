import type { ReactNode } from "react";

/// A labeled value inside a definition list.
///
/// Replaces three near-identical implementations that differed only in
/// `break-all` vs `break-words`; `wrap` selects that explicitly instead of
/// forking the component.
export function Detail({
  label,
  value,
  wrap = "words",
}: {
  label: string;
  value: ReactNode;
  wrap?: "words" | "all";
}) {
  return (
    <div className="rounded-lg bg-slate-900 p-3">
      <dt className="text-xs font-semibold uppercase tracking-wide text-slate-400">{label}</dt>
      <dd className={`mt-1 text-sm text-slate-100 ${wrap === "all" ? "break-all" : "break-words"}`}>
        {value}
      </dd>
    </div>
  );
}
