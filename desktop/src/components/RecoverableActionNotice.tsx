/// Renders the core's own recovery hint.
///
/// `recoverableAction` is published on both `CoreSnapshotDto` and
/// `HostSessionSnapshotDto` and was previously discarded by every screen: the
/// core was telling the operator how to recover and nothing displayed it.
export function RecoverableActionNotice({ action }: { action: string | null }) {
  if (!action) return null;
  return (
    <div
      role="status"
      className="rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100"
    >
      <p className="text-xs font-bold uppercase tracking-wide text-amber-200">Recovery available</p>
      <p className="mt-1 break-words text-sm">{action}</p>
    </div>
  );
}
