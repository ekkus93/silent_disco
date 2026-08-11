import { StatusCard } from "./shared";

interface SessionHeaderAndStatusProps {
  sessionName: string;
  revision: string;
  onRefresh: () => void;
  onEndSession: () => void;
  endPending: boolean;
  announcement: string | null;
  copyStatus: string | null;
  hostLifecycle: string;
  transportState: string;
  playbackState: string;
  transportWorkerRunning: boolean;
}

export function SessionHeaderAndStatus({
  sessionName,
  revision,
  onRefresh,
  onEndSession,
  endPending,
  announcement,
  copyStatus,
  hostLifecycle,
  transportState,
  playbackState,
  transportWorkerRunning,
}: SessionHeaderAndStatusProps) {
  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.2em] text-cyan-300">
            Rust-authoritative desktop host
          </p>
          <h1 className="mt-1 text-3xl font-bold">{sessionName}</h1>
          <p className="mt-2 text-sm text-slate-400">Snapshot revision {revision}</p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={onRefresh}
            className="rounded-lg border border-slate-600 px-4 py-2 text-sm font-semibold hover:border-slate-400"
          >
            Refresh host state
          </button>
          <button
            type="button"
            onClick={onEndSession}
            disabled={endPending}
            className="rounded-lg border border-rose-500/70 px-4 py-2 text-sm font-semibold text-rose-100 hover:bg-rose-950/50 disabled:opacity-50"
          >
            {endPending ? "Ending session…" : "End session"}
          </button>
        </div>
      </div>

      <div aria-live="polite" aria-atomic="true" className="mt-3 min-h-6 text-sm text-cyan-200">
        {announcement ?? copyStatus}
      </div>

      <section aria-labelledby="host-status-title" className="mt-6 grid gap-3 md:grid-cols-4">
        <h2 id="host-status-title" className="sr-only">
          Host status
        </h2>
        <StatusCard label="Host lifecycle" value={hostLifecycle} />
        <StatusCard label="Transport" value={transportState} />
        <StatusCard label="Playback" value={playbackState} />
        <StatusCard
          label="Transport worker"
          value={transportWorkerRunning ? "running" : "not running"}
        />
      </section>
    </>
  );
}
