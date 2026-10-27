import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { focusedDescription, tabSequence } from "./test/keyboard";
import {
  coreSnapshot,
  hostSession,
  listener,
  networkSnapshot,
  pendingRequest,
} from "./test/hostWorkflowFixtures";
import { renderHostApp } from "./test/renderHostApp";

const mocks = vi.hoisted(() => ({
  ensureDesktopBridge: vi.fn(),
  subscribeDesktopNotifications: vi.fn(),
  selectHostRole: vi.fn(),
  updateHostDraft: vi.fn(),
  createHostSession: vi.fn(),
  getHostSessionState: vi.fn(),
  endHostSession: vi.fn(),
  getHostNetworkState: vi.fn(),
  getLabModeAvailable: vi.fn(),
  getAppShutdownState: vi.fn(),
  startHostPlayback: vi.fn(),
  pauseHostPlayback: vi.fn(),
  stopHostPlayback: vi.fn(),
  addHostAudioSources: vi.fn(),
}));

vi.mock("./core/bridge", () => ({
  ensureDesktopBridge: mocks.ensureDesktopBridge,
  subscribeDesktopNotifications: mocks.subscribeDesktopNotifications,
}));
vi.mock("./core/client", async () => {
  const actual = await vi.importActual<typeof import("./core/client")>("./core/client");
  return {
    ...actual,
    selectHostRole: mocks.selectHostRole,
    updateHostDraft: mocks.updateHostDraft,
    createHostSession: mocks.createHostSession,
    getHostSessionState: mocks.getHostSessionState,
    endHostSession: mocks.endHostSession,
    getHostNetworkState: mocks.getHostNetworkState,
    getLabModeAvailable: mocks.getLabModeAvailable,
    getAppShutdownState: mocks.getAppShutdownState,
    startHostPlayback: mocks.startHostPlayback,
    pauseHostPlayback: mocks.pauseHostPlayback,
    stopHostPlayback: mocks.stopHostPlayback,
    addHostAudioSources: mocks.addHostAudioSources,
  };
});

function renderWorkflow(initial = coreSnapshot(), session = hostSession()) {
  return renderHostApp({
    initial,
    hostSession: session,
    ensureDesktopBridge: mocks.ensureDesktopBridge,
    subscribeDesktopNotifications: mocks.subscribeDesktopNotifications,
    getHostSessionState: mocks.getHostSessionState,
  });
}

function currentStep(): string | null {
  const nav = screen.getByRole("navigation", { name: "Host setup steps" });
  return nav.querySelector('[aria-current="step"]')?.textContent ?? null;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getHostNetworkState.mockResolvedValue(networkSnapshot());
  mocks.getLabModeAvailable.mockResolvedValue(false);
  mocks.getAppShutdownState.mockResolvedValue({ kind: "notRequested" });
  mocks.updateHostDraft.mockResolvedValue({ operationId: "draft-1", acceptedAtRevision: "4" });
  mocks.createHostSession.mockResolvedValue({ operationId: "create-1", acceptedAtRevision: "4" });
  mocks.endHostSession.mockResolvedValue({ operationId: "end-1", acceptedAtRevision: "21" });
  mocks.startHostPlayback.mockResolvedValue(undefined);
  mocks.pauseHostPlayback.mockResolvedValue(undefined);
  mocks.stopHostPlayback.mockResolvedValue(undefined);
});

describe("desktop host workflow navigation", () => {
  it("walks Music, Access, Ready, live, and back to setup only on authoritative state", async () => {
    const harness = renderWorkflow();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(currentStep()).toBe("1. Music");
    expect(screen.getByRole("heading", { name: "Playlist (3)" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    expect(currentStep()).toBe("2. Access");
    expect(screen.getByRole("combobox", { name: "Listener approval" })).toHaveValue("manual");

    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(currentStep()).toBe("3. Ready");
    const review = screen.getByRole("region", { name: "Session review" });
    expect(review).toHaveTextContent("Friday night");
    expect(review).toHaveTextContent("3 songs queued");
    expect(review).toHaveTextContent("Approve each listener");
    const start = screen.getByRole("button", { name: "Start hosting" });
    await waitFor(() => expect(start).toBeEnabled());

    fireEvent.click(start);
    await waitFor(() => expect(mocks.createHostSession).toHaveBeenCalledWith("4"));
    expect(await screen.findByText("Starting your session…")).toBeVisible();
    // The live dashboard is never shown on the strength of the command alone.
    expect(screen.queryByRole("region", { name: "Live playback and queue controls" })).toBeNull();

    harness.publishCore(coreSnapshot({ revision: "5", hostLifecycle: "waiting_for_listeners" }));
    expect(await screen.findByRole("heading", { name: "Friday night" })).toBeVisible();
    expect(screen.getByText("Waiting for listeners")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Create a silent disco" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(mocks.startHostPlayback).toHaveBeenCalledTimes(1));
    harness.setHostSession(
      hostSession({
        revision: "21",
        hostLifecycle: "streaming",
        transportState: "connected",
        playbackState: "playing",
        playbackPositionMs: "84000",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("Live")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Play" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "End session" }));
    await waitFor(() => expect(mocks.endHostSession).toHaveBeenCalledWith("21"));
    harness.publishCore(coreSnapshot({ revision: "6", hostLifecycle: "idle" }));
    expect(await screen.findByRole("heading", { name: "Create a silent disco" })).toBeVisible();
    expect(currentStep()).toBe("1. Music");
  });

  it("keeps Back navigation and the Music gate faithful to the authoritative playlist", async () => {
    const harness = renderWorkflow(
      coreSnapshot({ hostQueue: { revision: "0", entries: [] }, canCreateHostSession: false }),
    );
    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(screen.getByRole("button", { name: "Next: Access" })).toBeDisabled();
    expect(screen.getByText("No songs added. Add WAV, FLAC, or MP3 files to build the playlist."));

    harness.publishCore(coreSnapshot({ revision: "5" }));
    fireEvent.click(await screen.findByRole("button", { name: "Next: Access" }));
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    fireEvent.click(screen.getByRole("button", { name: "Back to access" }));
    expect(currentStep()).toBe("2. Access");
    fireEvent.click(screen.getByRole("button", { name: "Back to music" }));
    expect(currentStep()).toBe("1. Music");
    expect(screen.getByRole("textbox", { name: "Session name" })).toHaveValue("Friday night");
  });

  it("moves focus into each new setup step instead of dropping it on the page", async () => {
    const user = userEvent.setup();
    renderWorkflow();
    const name = await screen.findByRole("textbox", { name: "Session name" });

    // Enter inside the name field submits the step; the field then unmounts.
    await user.click(name);
    await user.keyboard("{Enter}");
    expect(currentStep()).toBe("2. Access");
    expect(focusedDescription()).toBe('combobox "Listener approval"');

    await user.keyboard("{Enter}");
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(currentStep()).toBe("3. Ready");
    expect(screen.getByRole("heading", { name: "Ready to host" })).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "Back to access" }));
    expect(focusedDescription()).toBe('combobox "Listener approval"');
    fireEvent.click(screen.getByRole("button", { name: "Back to music" }));
    expect(focusedDescription()).toBe('textbox "Session name"');

    fireEvent.click(screen.getByRole("button", { name: "Advanced tuning" }));
    expect(screen.getByRole("heading", { name: "Advanced tuning" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Back to host setup" }));
    expect(focusedDescription()).toBe('textbox "Session name"');
  });

  it("moves focus to the host destination when setup hands off to the live session", async () => {
    const harness = renderWorkflow();
    const start = await screen.findByRole("button", { name: "Next: Access" });
    fireEvent.click(start);
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    const startHosting = screen.getByRole("button", { name: "Start hosting" });
    await waitFor(() => expect(startHosting).toBeEnabled());
    startHosting.focus();
    fireEvent.click(startHosting);
    await waitFor(() => expect(mocks.createHostSession).toHaveBeenCalled());

    harness.publishCore(coreSnapshot({ revision: "5", hostLifecycle: "waiting_for_listeners" }));
    await screen.findByRole("heading", { name: "Friday night" });
    expect(screen.getByRole("tabpanel")).toHaveFocus();

    fireEvent.click(screen.getByRole("button", { name: "End session" }));
    await waitFor(() => expect(mocks.endHostSession).toHaveBeenCalled());
    harness.publishCore(coreSnapshot({ revision: "6", hostLifecycle: "idle" }));
    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(screen.getByRole("tabpanel")).toHaveFocus();
  });

  it("orders keyboard focus through the setup Music step in visual order", async () => {
    const user = userEvent.setup();
    renderWorkflow();
    await screen.findByRole("heading", { name: "Create a silent disco" });
    await waitFor(() => expect(mocks.getHostNetworkState).toHaveBeenCalled());

    expect(await tabSequence(user)).toEqual([
      'tab "Host control"',
      'textbox "Session name"',
      'button "Add songs"',
      'button "Next: Access"',
      'button "Advanced tuning"',
      'button "Move Golden Hour.flac down"',
      'button "Remove Golden Hour.flac"',
      'button "Move Soft Focus.wav up"',
      'button "Move Soft Focus.wav down"',
      'button "Remove Soft Focus.wav"',
      'button "Move Afterglow.mp3 up"',
      'button "Remove Afterglow.mp3"',
      'button "Configure in Settings"',
    ]);
  });

  it("orders keyboard focus through the live dashboard before secondary panels", async () => {
    const user = userEvent.setup();
    renderWorkflow(
      coreSnapshot({ hostLifecycle: "streaming" }),
      hostSession({
        hostLifecycle: "streaming",
        transportState: "connected",
        playbackState: "playing",
        playbackPositionMs: "84000",
        pendingJoinRequests: [pendingRequest("jo", "Jo's Pixel")],
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    await screen.findByText("Sam's Phone");

    expect(await tabSequence(user)).toEqual([
      'tab "Host control"',
      'tab "Diagnostics"',
      'tab "Storage"',
      'tab "Settings"',
      'button "Refresh"',
      'button "End session"',
      'button "Pause"',
      'button "Stop"',
      'checkbox "Local monitor (optional; listeners are unaffected)"',
      'button "Add songs"',
      'button "Move Soft Focus.wav down"',
      'button "Remove Soft Focus.wav"',
      'button "Move Afterglow.mp3 up"',
      'button "Remove Afterglow.mp3"',
      'button "Create invite"',
      'button "Copy host address"',
      'button "Copy session ID"',
      'button "Copy connection payload"',
      'button "Create QR invitation"',
      'checkbox "Remember this device"',
      'button "Approve"',
      'button "Reject"',
      'button "View Sam\'s Phone"',
    ]);
  });
});
