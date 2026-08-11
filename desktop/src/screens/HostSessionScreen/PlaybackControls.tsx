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
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
      <h2 id="playback-title" className="text-xl font-semibold">
        Playback controls
      </h2>
      <p className="mt-2 text-sm text-slate-400">
        Requires a selected audio source and an active host session.
      </p>
      {audioSource ? (
        <div className="mt-3 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
          <span className="font-semibold text-slate-100">{audioSource.displayName}</span>
          <span className="font-mono text-slate-400">
            {formatTimestamp(playbackPositionMs)} / {formatTimestamp(audioSource.durationMs)}
          </span>
          {streamEndedNaturally ? (
            <span className="rounded-full border border-cyan-500/60 bg-cyan-950/30 px-2 py-0.5 text-xs font-semibold text-cyan-200">
              Finished
            </span>
          ) : null}
        </div>
      ) : null}
      <div className="mt-4 flex gap-2">
        <button
          type="button"
          onClick={() => onControlPlayback(playbackState === "paused" ? "resume" : "start")}
          disabled={
            playbackPending ||
            !playbackControlsEnabled ||
            !["stopped", "ready", "paused"].includes(playbackState)
          }
          className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
        >
          {playbackState === "paused" ? "Resume" : "Play"}
        </button>
        <button
          type="button"
          onClick={() => onControlPlayback("pause")}
          disabled={playbackPending || !playbackControlsEnabled || playbackState !== "playing"}
          className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
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
          className="rounded-lg bg-slate-800 px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-40"
        >
          Stop
        </button>
      </div>

      <div className="mt-4 border-t border-slate-800 pt-4">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={monitor.enabled}
            disabled={monitorPending}
            onChange={(event) => onToggleMonitor(event.target.checked)}
          />
          Local monitor (optional -- never affects listener transmission)
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
