// Golden coverage for each reviewed desktop host workflow state
// (docs/mockups/SILENT_DISCO_WORKFLOW_MOCKUPS_2026-10-15.html: Host setup
// Music/Access/Ready, Host live, Manage queue, and the host side of approval).
// Each golden is an accessibility outline, not pixels or raw HTML: it records
// the structure, copy, control names, states, and live regions a reviewer
// needs to compare against the mockups, and stays stable across class churn.
// Client join/approval-wait/listening have no desktop surface yet.
import { fireEvent, render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { coreActions } from "./app/coreSlice";
import { createAppStore } from "./app/store";
import * as client from "./core/client";
import type { CoreSnapshotDto, HostSessionSnapshotDto } from "./core/generated/desktop-bindings";
import { HostSessionScreen } from "./screens/HostSessionScreen";
import { HostSetupScreen } from "./screens/HostSetupScreen";
import { accessibilityOutline } from "./test/accessibilityOutline";
import {
  AFTERGLOW,
  coreSnapshot,
  hostSession,
  listener,
  networkSnapshot,
  pendingRequest,
  SOFT_FOCUS,
} from "./test/hostWorkflowFixtures";

vi.mock("./core/client", async () => {
  const actual = await vi.importActual<typeof import("./core/client")>("./core/client");
  return {
    ...actual,
    getHostNetworkState: vi.fn(),
    getHostSessionState: vi.fn(),
    approveJoinRequest: vi.fn(),
  };
});

async function renderSetup(snapshot: CoreSnapshotDto) {
  const store = createAppStore();
  store.dispatch(coreActions.bridgeOpening({ profileId: "main" }));
  store.dispatch(coreActions.bridgeReady({ profileId: "main", snapshot }));
  const view = render(
    <Provider store={store}>
      <HostSetupScreen />
    </Provider>,
  );
  await screen.findByText("Ready to bind enp1s0 at 192.168.1.20.");
  return view.container;
}

async function renderLive(snapshot: HostSessionSnapshotDto) {
  vi.mocked(client.getHostSessionState).mockResolvedValue(snapshot);
  const view = render(
    <Provider store={createAppStore()}>
      <HostSessionScreen />
    </Provider>,
  );
  await screen.findByRole("heading", { name: snapshot.sessionName });
  await screen.findByText("Advertising");
  return view.container;
}

const PLAYING = {
  hostLifecycle: "streaming",
  transportState: "connected",
  playbackState: "playing",
  playbackPositionMs: "84000",
} as const;

beforeEach(() => {
  vi.mocked(client.getHostNetworkState).mockResolvedValue(networkSnapshot());
  vi.mocked(client.approveJoinRequest).mockResolvedValue({
    operationId: "approve-jo",
    acceptedAtRevision: "20",
  });
});

describe("reviewed host setup states", () => {
  it("Music step with an empty playlist", async () => {
    const container = await renderSetup(
      coreSnapshot({ hostQueue: { revision: "0", entries: [] }, canCreateHostSession: false }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("Music step with an ordered playlist", async () => {
    const container = await renderSetup(coreSnapshot());
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("Access step with invite code and field validation", async () => {
    const container = await renderSetup(
      coreSnapshot({
        hostDraft: { ...coreSnapshot().hostDraft, approvalMode: "invite_code", inviteCode: "" },
        hostDraftValidation: [
          {
            field: "inviteCode",
            code: "invite_code_required",
            message: "Enter an invite code.",
          },
        ],
        canCreateHostSession: false,
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("Ready step summarizing the session", async () => {
    const container = await renderSetup(coreSnapshot());
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });
});

describe("reviewed live host states", () => {
  it("waiting for listeners with a pending join request", async () => {
    const container = await renderLive(
      hostSession({ pendingJoinRequests: [pendingRequest("jo", "Jo's Pixel")] }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("approval sent and awaiting delivery confirmation", async () => {
    const container = await renderLive(
      hostSession({ pendingJoinRequests: [pendingRequest("jo", "Jo's Pixel")] }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await screen.findByText("Waiting for approval delivery confirmation…");
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("playing with a locked current song, upcoming queue, and mixed listener health", async () => {
    const container = await renderLive(
      hostSession({
        ...PLAYING,
        connectedListeners: [
          listener("sam", "Sam's Phone", { trustState: "trusted" }),
          listener("jo", "Jo's Pixel", { transportState: "connecting", syncConfidence: null }),
          listener("kai", "Kai's Tablet", { transportState: "retrying", syncConfidence: null }),
        ],
      }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("paused mid-song", async () => {
    const container = await renderLive(
      hostSession({
        ...PLAYING,
        hostLifecycle: "paused",
        playbackState: "paused",
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("queue ended with the session still open", async () => {
    const container = await renderLive(
      hostSession({
        hostLifecycle: "ready",
        transportState: "connected",
        playbackState: "stopped",
        playbackPositionMs: "259000",
        streamEndedNaturally: true,
        audioSource: AFTERGLOW.source,
        hostQueue: { revision: "9", entries: [] },
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });

  it("current song failed with Retry and the queue retained", async () => {
    const container = await renderLive(
      hostSession({
        hostLifecycle: "error",
        transportState: "connected",
        playbackState: "error",
        playbackPositionMs: "1000",
        audioSource: SOFT_FOCUS.source,
        hostQueue: { revision: "8", entries: [SOFT_FOCUS, AFTERGLOW] },
        connectedListeners: [listener("sam", "Sam's Phone")],
        lastError: {
          code: "core.audio_source_read_failed",
          subsystem: "audio",
          severity: "error",
          retryable: true,
          message: "Soft Focus.wav could not be read.",
        },
      }),
    );
    expect(accessibilityOutline(container)).toMatchSnapshot();
  });
});
