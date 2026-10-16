import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createAppStore } from "../../app/store";
import * as client from "../../core/client";
import type { HostSessionSnapshotDto } from "../../core/generated/desktop-bindings";
import {
  AFTERGLOW,
  GOLDEN_HOUR,
  hostSession,
  listener,
  networkSnapshot,
  SOFT_FOCUS,
} from "../../test/hostWorkflowFixtures";
import { HostSessionScreen } from ".";

vi.mock("../../core/client");

let current: HostSessionSnapshotDto;

function publish(next: HostSessionSnapshotDto) {
  current = next;
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
}

function renderScreen(initial: HostSessionSnapshotDto) {
  current = initial;
  return render(
    <Provider store={createAppStore()}>
      <HostSessionScreen />
    </Provider>,
  );
}

function dashboard(): HTMLElement {
  return screen.getByRole("region", { name: "Live playback and queue controls" });
}

function queueRows(): string[] {
  const queue = screen.getByRole("region", { name: /^Manage queue/ });
  return within(queue)
    .getAllByRole("listitem")
    .map((item) => {
      const cells = item.querySelectorAll("span");
      return Array.from(cells)
        .map((cell) => cell.textContent?.trim())
        .filter(Boolean)
        .join(" | ");
    });
}

const PLAYING = {
  hostLifecycle: "streaming",
  transportState: "connected",
  playbackState: "playing",
} as const;

beforeEach(() => {
  vi.mocked(client.getHostSessionState).mockImplementation(() => Promise.resolve(current));
  vi.mocked(client.getHostNetworkState).mockResolvedValue(networkSnapshot());
  vi.mocked(client.startHostPlayback).mockResolvedValue(undefined);
  vi.mocked(client.addHostAudioSources).mockResolvedValue({
    operationId: "append-1",
    acceptedAtRevision: "30",
  });
});

describe("live dashboard track boundaries", () => {
  it("updates the current and next track at a boundary without remounting or moving focus", async () => {
    renderScreen(
      hostSession({
        ...PLAYING,
        playbackPositionMs: "221000",
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    const nowPlaying = await screen.findByRole("status", { name: "Now playing" });
    expect(nowPlaying).toHaveTextContent("Golden Hour.flac");
    expect(queueRows()).toEqual([
      "Now | Golden Hour.flac | Playing",
      "2 | Soft Focus.wav",
      "3 | Afterglow.mp3",
    ]);
    const region = dashboard();
    const pause = screen.getByRole("button", { name: "Pause" });
    pause.focus();

    publish(
      hostSession({
        ...PLAYING,
        revision: "21",
        playbackPositionMs: "1000",
        audioSource: SOFT_FOCUS.source,
        hostQueue: { revision: "4", entries: [SOFT_FOCUS, AFTERGLOW] },
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );

    await waitFor(() => expect(nowPlaying).toHaveTextContent("Soft Focus.wav"));
    expect(screen.getByRole("status", { name: "Now playing" })).toBe(nowPlaying);
    expect(nowPlaying).toHaveAttribute("aria-live", "polite");
    expect(screen.getByText("0:01 / 4:08")).toBeVisible();
    const progress = screen.getByRole("progressbar", { name: "Playback progress" });
    expect(progress).toHaveAttribute("max", "248000");
    expect(progress).toHaveAttribute("value", "1000");
    expect(queueRows()).toEqual(["Now | Soft Focus.wav | Playing", "2 | Afterglow.mp3"]);
    expect(screen.getByRole("heading", { name: "Manage queue (2)" })).toBeVisible();
    expect(screen.queryByText("Golden Hour.flac")).toBeNull();
    // Same mounted dashboard, same focused control: a boundary is not a navigation.
    expect(dashboard()).toBe(region);
    expect(pause).toHaveFocus();
    expect(pause).toBeEnabled();
    expect(screen.queryByText("Finished")).toBeNull();
  });

  it("stays in the session at queue end and does not auto-start songs added afterwards", async () => {
    renderScreen(
      hostSession({
        ...PLAYING,
        playbackPositionMs: "258000",
        audioSource: AFTERGLOW.source,
        hostQueue: { revision: "5", entries: [AFTERGLOW] },
      }),
    );
    await screen.findByRole("status", { name: "Now playing" });

    publish(
      hostSession({
        revision: "21",
        hostLifecycle: "ready",
        transportState: "connected",
        playbackState: "stopped",
        playbackPositionMs: "259000",
        streamEndedNaturally: true,
        audioSource: AFTERGLOW.source,
        hostQueue: { revision: "6", entries: [] },
      }),
    );
    expect(await screen.findByText("Finished")).toBeVisible();
    expect(screen.getByText("No songs queued. Add songs, then press Play.")).toBeVisible();
    expect(screen.getByRole("heading", { name: "Friday night" })).toBeVisible();
    expect(screen.getByRole("button", { name: "End session" })).toBeEnabled();

    fireEvent.click(within(dashboard()).getByRole("button", { name: "Add songs" }));
    await waitFor(() => expect(client.addHostAudioSources).toHaveBeenCalledTimes(1));
    current = hostSession({
      revision: "22",
      hostLifecycle: "ready",
      transportState: "connected",
      playbackState: "stopped",
      audioSource: GOLDEN_HOUR.source,
      hostQueue: { revision: "7", entries: [GOLDEN_HOUR] },
    });
    await waitFor(() => expect(queueRows()).toEqual(["1 | Golden Hour.flac"]));
    expect(screen.getByRole("button", { name: "Play" })).toBeEnabled();
    expect(client.startHostPlayback).not.toHaveBeenCalled();
  });
});

describe("live dashboard accessibility semantics", () => {
  it("names every primary action and exposes state changes through live regions", async () => {
    renderScreen(
      hostSession({
        ...PLAYING,
        playbackPositionMs: "84000",
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    await screen.findByRole("status", { name: "Now playing" });

    const live = dashboard();
    for (const name of ["Play", "Pause", "Stop", "Add songs", "Create invite"]) {
      expect(within(live).getByRole("button", { name })).toBeVisible();
    }
    expect(within(live).getByRole("checkbox", { name: /Local monitor/ })).toBeVisible();
    expect(within(live).getByRole("progressbar", { name: "Playback progress" })).toBeVisible();
    for (const name of ["Refresh", "End session"]) {
      expect(screen.getByRole("button", { name })).toBeVisible();
    }
    for (const name of [
      "Playback controls",
      "Manage queue (3)",
      "Pending join requests",
      "Connected listeners",
    ]) {
      expect(screen.getByRole("heading", { name })).toBeVisible();
    }

    const status = screen.getByRole("region", { name: "Host status" });
    expect(status).toHaveAttribute("aria-live", "polite");
    expect(within(status).getByText("Live")).toBeVisible();

    publish(
      hostSession({
        revision: "21",
        hostLifecycle: "error",
        transportState: "connected",
        playbackState: "error",
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    await waitFor(() => expect(within(status).getAllByText("Action needed")).toHaveLength(2));
    expect(within(status).getAllByText("FAILED")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  });
});
