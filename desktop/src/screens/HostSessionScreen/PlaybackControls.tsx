import type {
  AudioSourceSummaryDto,
  MonitorStatusDto,
} from "../../core/generated/desktop-bindings";
import { formatTimestamp } from "./domain";

interface PlaybackControlsProps {
  audioSource: AudioSourceSummaryDto | null;
  playbackPositionMs: string;
  streamEndedNaturally: boolean;
  playbackState: string;
  playbackControlsEnabled: boolean;
  playbackPending: boolean;
  onControlPlayback: (action: "start" | "pause" | "resume" | "stop") => void;
  monitor: MonitorStatusDto;
  monitorPending: boolean;
  onToggleMonitor: (enabled: boolean) => void;
}

export function PlaybackControls({
  audioSource,
  playbackPositionMs,
  streamEndedNaturally,
  playbackState,
  playbackControlsEnabled,
  playbackPending,
  onControlPlayback,
  monitor,
  monitorPending,
  onToggleMonitor,
}: PlaybackControlsProps) {
  return (
    <section
      aria-labelledby="playback-title"
      className="sd-workflow-card mt-6 p-5 [@media(max-height:700px)]:mt-2 [@media(max-height:700px)]:p-3"
    >
      <h2 id="playback-title" className="text-sd-title [@media(max-height:700px)]:text-lg">
        Playback controls
      </h2>
      <p className="desktop-playback-hint mt-2 text-sm text-slate-400 [@media(max-height:700px)]:mt-1">
        Requires a selected audio source and an active host session.
      </p>
      {audioSource ? (
        <div className="mt-3 [@media(max-height:700px)]:mt-1">
          <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
            {/* Polite live region: a gapless track boundary changes this title
                without any navigation, so it is the only cue a screen-reader
                user gets that the current song changed. */}
            <span
              role="status"
              aria-live="polite"
              aria-label="Now playing"
              className="font-semibold text-slate-100"
            >
              {audioSource.displayName}
            </span>
            <span className="font-mono text-slate-400">
              {formatTimestamp(playbackPositionMs)} / {formatTimestamp(audioSource.durationMs)}
            </span>
            {streamEndedNaturally ? (
              <span className="rounded-full border border-sd-outline-variant bg-sd-secondary-container px-2 py-0.5 text-xs font-semibold text-sd-on-secondary-container">
                Finished
              </span>
            ) : null}
          </div>
          {Number(audioSource.durationMs) > 0 ? (
            <progress
              aria-label="Playback progress"
              className="mt-2 h-2 w-full accent-sd-secondary [@media(max-height:700px)]:mt-1"
              max={Number(audioSource.durationMs)}
              value={Math.min(Number(playbackPositionMs), Number(audioSource.durationMs))}
            />
          ) : null}
        </div>
      ) : null}
      <div className="mt-4 flex gap-2 [@media(max-height:700px)]:mt-2">
        <button
          type="button"
          onClick={() => onControlPlayback(playbackState === "paused" ? "resume" : "start")}
          disabled={
            playbackPending ||
            !playbackControlsEnabled ||
            !["stopped", "ready", "paused", "error"].includes(playbackState)
          }
          className="sd-workflow-control rounded-sd-control bg-sd-surface-variant px-4 py-2 text-sm font-semibold text-sd-on-surface disabled:cursor-not-allowed disabled:opacity-40 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-1"
        >
          {playbackState === "paused" ? "Resume" : playbackState === "error" ? "Retry" : "Play"}
        </button>
        <button
          type="button"
          onClick={() => onControlPlayback("pause")}
          disabled={playbackPending || !playbackControlsEnabled || playbackState !== "playing"}
          className="sd-workflow-control rounded-sd-control bg-sd-surface-variant px-4 py-2 text-sm font-semibold text-sd-on-surface disabled:cursor-not-allowed disabled:opacity-40 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-1"
        >
          Pause
        </button>
        <button
          type="button"
          onClick={() => onControlPlayback("stop")}
          disabled={
            playbackPending ||
            !playbackControlsEnabled ||
            !["playing", "paused", "buffering"].includes(playbackState)
          }
          className="sd-workflow-control rounded-sd-control bg-sd-surface-variant px-4 py-2 text-sm font-semibold text-sd-on-surface disabled:cursor-not-allowed disabled:opacity-40 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-1"
        >
          Stop
        </button>
      </div>

      <div className="desktop-host-monitor mt-4 border-t border-slate-800 pt-4 [@media(max-height:700px)]:mt-2 [@media(max-height:700px)]:pt-2">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={monitor.enabled}
            disabled={monitorPending}
            onChange={(event) => onToggleMonitor(event.target.checked)}
          />
          <span>Local monitor</span>
          <span className="desktop-monitor-note">(optional; listeners are unaffected)</span>
        </label>
        {monitor.enabled ? (
          <p
            role={monitor.active ? "status" : "alert"}
            className={`mt-2 text-sm ${monitor.active ? "text-emerald-200" : "text-amber-200"}`}
          >
            {monitor.active
              ? "Monitor is playing through the local audio device."
              : `Monitor is enabled but not active${
                  monitor.failureReason ? `: ${monitor.failureReason}` : "."
                } Listener transmission is unaffected.`}
          </p>
        ) : null}
      </div>
    </section>
  );
}
