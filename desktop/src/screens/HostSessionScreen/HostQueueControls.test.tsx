import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { addHostAudioSources, removeHostQueueEntry, reorderHostQueue } from "../../core/client";
import type { HostQueueDto } from "../../core/generated/desktop-bindings";
import { HostQueueControls } from "./HostQueueControls";

vi.mock("../../core/client", async () => {
  const actual = await vi.importActual<typeof import("../../core/client")>("../../core/client");
  return {
    ...actual,
    addHostAudioSources: vi.fn(),
    removeHostQueueEntry: vi.fn(),
    reorderHostQueue: vi.fn(),
  };
});

const queue: HostQueueDto = {
  revision: "7",
  entries: [
    {
      entryId: "entry-1",
      source: { sourceId: "src-1", displayName: "First.flac", byteLength: "8", durationMs: "10" },
    },
    {
      entryId: "entry-2",
      source: { sourceId: "src-2", displayName: "Second.wav", byteLength: "8", durationMs: "10" },
    },
    {
      entryId: "entry-3",
      source: { sourceId: "src-3", displayName: "Third.mp3", byteLength: "8", durationMs: "10" },
    },
  ],
};

describe("HostQueueControls", () => {
  const onRefresh = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(addHostAudioSources).mockResolvedValue(null);
    vi.mocked(removeHostQueueEntry).mockResolvedValue({
      operationId: "remove",
      acceptedAtRevision: "8",
    });
    vi.mocked(reorderHostQueue).mockResolvedValue({
      operationId: "reorder",
      acceptedAtRevision: "8",
    });
  });

  it("adds songs through the transactional picker and refreshes authoritative state", async () => {
    render(
      <HostQueueControls
        queue={queue}
        revision="12"
        playbackState="stopped"
        onRefresh={onRefresh}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Add songs" }));
    await waitFor(() => expect(addHostAudioSources).toHaveBeenCalledOnce());
    expect(onRefresh).toHaveBeenCalledOnce();
  });

  it("locks the current entry and moves/removes upcoming entries by queue revision", async () => {
    render(
      <HostQueueControls
        queue={queue}
        revision="12"
        playbackState="playing"
        onRefresh={onRefresh}
      />,
    );
    expect(screen.getByText("First.flac").parentElement).toHaveTextContent("Playing");
    fireEvent.click(screen.getByRole("button", { name: "Move Third.mp3 up" }));
    await waitFor(() =>
      expect(reorderHostQueue).toHaveBeenCalledWith({
        expectedRevision: "12",
        expectedQueueRevision: "7",
        orderedEntryIds: ["entry-1", "entry-3", "entry-2"],
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove Second.wav" }));
    await waitFor(() =>
      expect(removeHostQueueEntry).toHaveBeenCalledWith({
        expectedRevision: "12",
        expectedQueueRevision: "7",
        entryId: "entry-2",
      }),
    );
  });

  it("marks the failed current source and lets the host remove it", async () => {
    render(
      <HostQueueControls queue={queue} revision="12" playbackState="error" onRefresh={onRefresh} />,
    );
    expect(screen.getByText("First.flac").parentElement).toHaveTextContent("Failed");
    fireEvent.click(screen.getByRole("button", { name: "Remove First.flac" }));
    await waitFor(() =>
      expect(removeHostQueueEntry).toHaveBeenCalledWith({
        expectedRevision: "12",
        expectedQueueRevision: "7",
        entryId: "entry-1",
      }),
    );
  });

  it("keeps queue keyboard focus in visible action order and skips disabled moves", async () => {
    const user = userEvent.setup();
    render(
      <HostQueueControls
        queue={queue}
        revision="12"
        playbackState="playing"
        onRefresh={onRefresh}
      />,
    );

    await user.tab();
    expect(screen.getByRole("button", { name: "Add songs" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Move Second.wav down" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Remove Second.wav" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Move Third.mp3 up" })).toHaveFocus();
  });
});
