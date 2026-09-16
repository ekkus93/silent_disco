import type { ReactNode } from "react";

/// The panel chrome that was previously repeated verbatim as an inline
/// utility string in every screen that needed a bordered section.
export function Panel({
  title,
  headingLevel = 2,
  actions,
  children,
  className = "",
}: {
  title?: string;
  /// Panels nested inside another panel's heading scope pass `3` so the
  /// document keeps a single descending outline.
  headingLevel?: 2 | 3;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const Heading = headingLevel === 2 ? "h2" : "h3";
  return (
    <section className={`rounded-2xl border border-slate-700 bg-slate-950/70 p-5 ${className}`}>
      {title || actions ? (
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          {title ? <Heading className="text-lg font-semibold">{title}</Heading> : null}
          {actions ? <div className="flex items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
      {children}
    </section>
  );
}
