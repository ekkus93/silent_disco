import { useState } from "react";

import type { HostQueueDto } from "../../core/generated/desktop-bindings";
import {
  addHostAudioSources,
  removeHostQueueEntry,
  reorderHostQueue,
  toDesktopBridgeError,
} from "../../core/client";
import { ErrorAlert } from "../../components/ErrorAlert";

interface HostQueueControlsProps {
  queue: HostQueueDto;
  revision: string;
  playbackState: string;
  onRefresh: () => void;
}

export function HostQueueControls({
  queue,
  revision,
  playbackState,
  onRefresh,
}: HostQueueControlsProps) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<ReturnType<typeof toDesktopBridgeError> | null>(null);
  const locked = ["playing", "paused", "buffering", "underrun"].includes(playbackState);
  const run = async (action: () => Promise<unknown>) => {
    setPending(true);
    setError(null);
    try {
      await action();
      onRefresh();
    } catch (failure: unknown) {
      setError(toDesktopBridgeError(failure, "manage host queue"));
    } finally {
      setPending(false);
    }
  };
  const reorder = (index: number, offset: number) => {
    const upcoming = queue.entries.slice(locked ? 1 : 0);
    const target = index + offset;
    if (target < 0 || target >= upcoming.length) return;
    const swapped = [...upcoming];
    const first = swapped[index];
    const second = swapped[target];
    if (!first || !second) return;
    swapped[index] = second;
    swapped[target] = first;
    const entries = locked ? [...queue.entries.slice(0, 1), ...swapped] : swapped;
    void run(() =>
      reorderHostQueue({
        expectedRevision: revision,
        expectedQueueRevision: queue.revision,
        orderedEntryIds: entries.map((entry) => entry.entryId),
      }),
    );
  };
  return (
    <section
      aria-labelledby="host-queue-title"
      className="sd-workflow-card mt-5 p-5 [@media(max-height:700px)]:mt-2 [@media(max-height:700px)]:p-3"
    >
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 id="host-queue-title" className="text-sd-title [@media(max-height:700px)]:text-lg">
            Manage queue ({queue.entries.length})
          </h2>
          <p className="desktop-queue-hint mt-1 text-sm text-slate-400 [@media(max-height:700px)]:mt-0">
            The current song stays in place while it plays.
          </p>
        </div>
        <button
          type="button"
          disabled={pending}
          onClick={() => void run(addHostAudioSources)}
          className="sd-workflow-control rounded-sd-control bg-sd-secondary-container px-4 py-2 text-sm font-semibold text-sd-on-secondary-container disabled:opacity-50 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-1"
        >
          Add songs
        </button>
      </div>
      {error ? (
        <div className="mt-3 [@media(max-height:700px)]:mt-2">
          <ErrorAlert error={error} />
        </div>
      ) : null}
      <ol className="mt-4 max-h-40 divide-y divide-slate-800 overflow-y-auto overscroll-contain [@media(max-height:700px)]:mt-2 [@media(max-height:700px)]:max-h-20">
        {queue.entries.map((entry, index) => {
          const current = locked && index === 0;
          const upcomingIndex = index - (locked ? 1 : 0);
          return (
            <li
              key={entry.entryId}
              className="flex items-center gap-3 py-3 [@media(max-height:700px)]:py-1"
            >
              <span className="w-10 text-sm text-slate-500">
                {current ? "Now" : playbackState === "error" && index === 0 ? "Failed" : index + 1}
              </span>
              <span className="min-w-0 flex-1 truncate font-medium">
                {entry.source.displayName}
              </span>
              {current ? (
                <span className="text-xs text-cyan-200">Playing</span>
              ) : (
                <>
                  <button
                    type="button"
                    aria-label={`Move ${entry.source.displayName} up`}
                    disabled={pending || upcomingIndex <= 0}
                    onClick={() => reorder(upcomingIndex, -1)}
                    className="rounded border border-slate-600 px-2 py-1 disabled:opacity-30 [@media(max-height:700px)]:px-1 [@media(max-height:700px)]:py-0"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    aria-label={`Move ${entry.source.displayName} down`}
                    disabled={pending || index === queue.entries.length - 1}
                    onClick={() => reorder(upcomingIndex, 1)}
                    className="rounded border border-slate-600 px-2 py-1 disabled:opacity-30 [@media(max-height:700px)]:px-1 [@media(max-height:700px)]:py-0"
                  >
                    ↓
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove ${entry.source.displayName}`}
                    disabled={pending}
                    onClick={() =>
                      void run(() =>
                        removeHostQueueEntry({
                          expectedRevision: revision,
                          expectedQueueRevision: queue.revision,
                          entryId: entry.entryId,
                        }),
                      )
                    }
                    className="rounded border border-slate-600 px-3 py-1 text-sm disabled:opacity-50 [@media(max-height:700px)]:px-2 [@media(max-height:700px)]:py-0"
                  >
                    Remove
                  </button>
                </>
              )}
            </li>
          );
        })}
      </ol>
      {queue.entries.length === 0 ? (
        <p className="mt-4 text-sm text-slate-400 [@media(max-height:700px)]:mt-2">
          No songs queued. Add songs, then press Play.
        </p>
      ) : null}
    </section>
  );
}
