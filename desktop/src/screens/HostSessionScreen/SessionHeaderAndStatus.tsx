import type { StatusSeverity } from "../../components/StatusCard";
import { StatusCard } from "./shared";

// These map the core's own state names onto severity. They are deliberately
// exhaustive-by-listing rather than defaulting to "good": an unrecognised
// state reads as neutral, never as healthy.
function lifecycleSeverity(value: string): StatusSeverity {
  if (value === "error") return "critical";
  if (value === "streaming" || value === "ready" || value === "waiting_for_listeners") {
    return "good";
  }
  return "neutral";
}

function transportSeverity(value: string): StatusSeverity {
  if (value === "failed" || value === "disconnected") return "critical";
  if (value === "retrying") return "warning";
  if (value === "connected") return "good";
  return "neutral";
}

function playbackSeverity(value: string): StatusSeverity {
  if (value === "error") return "critical";
  if (value === "underrun") return "warning";
  if (value === "playing") return "good";
  return "neutral";
}

function hostStatusLabel(value: string): string {
  switch (value) {
    case "advertising":
    case "creating_session":
      return "Starting";
    case "waiting_for_listeners":
      return "Waiting for listeners";
    case "streaming":
      return "Live";
    case "ending_session":
      return "Ending";
    case "error":
      return "Action needed";
    default:
      return value.replaceAll("_", " ");
  }
}

function connectionLabel(value: string): string {
  switch (value) {
    case "advertising":
      return "Ready to connect";
    case "retrying":
      return "Reconnecting";
    case "failed":
      return "Connection failed";
    case "idle":
      return "Not connected";
    default:
      return value.replaceAll("_", " ");
  }
}

function playbackLabel(value: string): string {
  switch (value) {
    case "buffering":
      return "Preparing audio";
    case "underrun":
      return "Audio interrupted";
    case "error":
      return "Action needed";
    default:
      return value.replaceAll("_", " ");
  }
}

interface SessionHeaderAndStatusProps {
  sessionName: string;
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
            Live host
          </p>
          <h2 className="desktop-host-session-name mt-1 text-3xl font-bold">{sessionName}</h2>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={onRefresh}
            className="sd-workflow-control rounded-sd-control border border-sd-outline px-4 py-2 text-sm font-semibold text-sd-on-surface hover:border-sd-primary"
          >
            Refresh
          </button>
          <button
            type="button"
            onClick={onEndSession}
            disabled={endPending}
            className="sd-workflow-control rounded-sd-control border border-sd-error px-4 py-2 text-sm font-semibold text-sd-error hover:bg-sd-error-container disabled:opacity-50"
          >
            {endPending ? "Ending session…" : "End session"}
          </button>
        </div>
      </div>

      <div
        aria-live="polite"
        aria-atomic="true"
        className="desktop-host-announcement mt-3 min-h-6 text-sm text-cyan-200"
      >
        {announcement ?? copyStatus}
      </div>

      <section
        aria-labelledby="host-status-title"
        className="desktop-host-status-grid mt-6 grid gap-3 md:grid-cols-4"
      >
        <h2 id="host-status-title" className="sr-only">
          Host status
        </h2>
        <StatusCard
          className="desktop-host-status-card"
          label="Host"
          value={hostStatusLabel(hostLifecycle)}
          severity={lifecycleSeverity(hostLifecycle)}
        />
        <StatusCard
          className="desktop-host-status-card"
          label="Connection"
          value={connectionLabel(transportState)}
          severity={transportSeverity(transportState)}
        />
        <StatusCard
          className="desktop-host-status-card"
          label="Playback"
          value={playbackLabel(playbackState)}
          severity={playbackSeverity(playbackState)}
        />
        <StatusCard
          className="desktop-host-status-card"
          label="Session service"
          value={transportWorkerRunning ? "Available" : "Unavailable"}
          severity={transportWorkerRunning ? "good" : "critical"}
        />
      </section>
    </>
  );
}
