import "@testing-library/jest-dom/vitest";

import { StrictMode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { coreActions } from "../app/coreSlice";
import { createAppStore } from "../app/store";
import type { CoreSnapshotDto } from "../core/generated/desktop-bindings";
import { HostSetupScreen } from "./HostSetupScreen";

const {
  addHostAudioSources,
  selectHostRole,
  updateHostDraft,
  createHostSession,
  getHostNetworkState,
  setHostNetworkPreference,
} = vi.hoisted(() => ({
  addHostAudioSources: vi.fn(),
  selectHostRole: vi.fn(),
  updateHostDraft: vi.fn(),
  createHostSession: vi.fn(),
  getHostNetworkState: vi.fn(),
  setHostNetworkPreference: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return {
    ...actual,
    addHostAudioSources,
    selectHostRole,
    updateHostDraft,
    createHostSession,
    getHostNetworkState,
    setHostNetworkPreference,
  };
});

function snapshot(overrides: Partial<CoreSnapshotDto> = {}): CoreSnapshotDto {
  return {
    revision: "4",
    selectedRole: "host",
    capabilities: {
      localNetworkAvailable: true,
      audioSourceSelectionAvailable: true,
      audioOutputAvailable: true,
      secureStoreAvailable: true,
    },
    hostDraft: {
      sessionName: "Oakland Night",
      approvalMode: "manual",
      inviteCode: null,
      audioSource: {
        sourceId: "source-1",
        displayName: "set.wav",
        byteLength: "1024",
        durationMs: "5000",
      },
      rememberApprovedDevices: false,
    },
    hostQueue: {
      revision: "0",
      entries: [
        {
          entryId: "entry-1",
          source: {
            sourceId: "source-1",
            displayName: "set.wav",
            byteLength: "1024",
            durationMs: "5000",
          },
        },
      ],
    },
    hostDraftValidation: [],
    canCreateHostSession: true,
    hostLifecycle: "idle",
    listenerLifecycle: "idle",
    transportState: "idle",
    discoveryActive: false,
    discoveredSessionCount: 0,
    pendingJoinRequestCount: 0,
    listenerCount: 0,
    playbackState: "stopped",
    playbackPositionMs: "0",
    recoverableAction: null,
    lastError: null,
    shuttingDown: false,
    ...overrides,
  };
}

function networkSnapshot() {
  const selected = {
    interfaceName: "enp1s0",
    interfaceIndex: 2,
    address: "192.168.1.20",
    prefixLength: 24,
    classification: "private_lan" as const,
    isDefaultRoute: true,
    isActive: true,
    isPhysical: true,
    selectable: true,
    rejectionReason: null,
  };
  return {
    preference: { mode: "automatic", interfaceName: null, address: null },
    candidates: [selected],
    automaticSelection: selected,
    resolvedSelection: selected,
    requiresExplicitSelection: false,
    selectionError: null,
    activeBinding: null,
    activeBindingValid: false,
    interfaceChange: null,
  };
}

function renderScreen(initial = snapshot()) {
  const store = createAppStore();
  store.dispatch(coreActions.bridgeOpening({ profileId: "main" }));
  store.dispatch(coreActions.bridgeReady({ profileId: "main", snapshot: initial }));
  render(
    <Provider store={store}>
      <HostSetupScreen />
    </Provider>,
  );
  return store;
}

function advanceToReady() {
  fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
  fireEvent.click(screen.getByRole("button", { name: "Review session" }));
}

beforeEach(() => {
  vi.clearAllMocks();
  addHostAudioSources.mockResolvedValue({ operationId: "source-2", acceptedAtRevision: "4" });
  selectHostRole.mockResolvedValue({ operationId: "role-1", acceptedAtRevision: "4" });
  updateHostDraft.mockResolvedValue({ operationId: "draft-1", acceptedAtRevision: "4" });
  createHostSession.mockResolvedValue({ operationId: "create-1", acceptedAtRevision: "4" });
  getHostNetworkState.mockResolvedValue(networkSnapshot());
  setHostNetworkPreference.mockResolvedValue(networkSnapshot());
});

describe("HostSetupScreen", () => {
  it("selects the host role only once when StrictMode replays effects", async () => {
    const store = createAppStore();
    store.dispatch(coreActions.bridgeOpening({ profileId: "main" }));
    store.dispatch(
      coreActions.bridgeReady({
        profileId: "main",
        snapshot: snapshot({ selectedRole: null }),
      }),
    );

    render(
      <StrictMode>
        <Provider store={store}>
          <HostSetupScreen />
        </Provider>
      </StrictMode>,
    );

    await waitFor(() => expect(selectHostRole).toHaveBeenCalledTimes(1));
    expect(selectHostRole).toHaveBeenCalledWith("4");
  });

  it("presents Music, Access, and Ready as ordered wizard stages", () => {
    renderScreen();
    expect(screen.getByLabelText("Session name")).toBeInTheDocument();
    expect(screen.queryByLabelText("Listener approval")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    const approval = screen.getByLabelText("Listener approval");
    const remember = screen.getByLabelText("Remember approved devices");
    expect(
      approval.compareDocumentPosition(remember) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(screen.getByRole("heading", { name: "Ready to host" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start hosting" })).toBeInTheDocument();
  });

  it("discloses plaintext transport in access and review, even for trusted-device mode", () => {
    renderScreen();
    expect(screen.queryByRole("note", { name: "Local network security limitation" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    const warning = screen.getByRole("note", { name: "Local network security limitation" });
    expect(warning).toHaveTextContent("does not encrypt or authenticate live session traffic");
    expect(warning).toHaveTextContent("signed QR or approved-device");
    fireEvent.change(screen.getByLabelText("Listener approval"), {
      target: { value: "trusted_devices" },
    });
    expect(screen.getByRole("note", { name: "Local network security limitation" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(screen.getByRole("heading", { name: "Ready to host" })).toBeInTheDocument();
    expect(screen.getByRole("note", { name: "Local network security limitation" })).toBeVisible();
  });

  it("shows invite-code controls only for invite-code approval", () => {
    renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    expect(screen.queryByLabelText("Invite code")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Listener approval"), {
      target: { value: "invite_code" },
    });
    expect(screen.getByLabelText("Invite code")).toBeInTheDocument();
  });

  it("renders authoritative core validation beside its field", () => {
    renderScreen(
      snapshot({
        canCreateHostSession: false,
        hostDraftValidation: [
          { field: "sessionName", code: "session_name", message: "session name is invalid" },
        ],
      }),
    );
    expect(screen.queryByText("session name is invalid")).not.toBeInTheDocument();
    fireEvent.blur(screen.getByLabelText("Session name"));
    expect(screen.getByText("session name is invalid")).toBeInTheDocument();
    expect(screen.getByLabelText("Session name")).toHaveAttribute(
      "aria-describedby",
      "session-name-error",
    );
  });

  it("timestamps and dismisses a transient core error", async () => {
    const store = renderScreen();
    store.dispatch(
      coreActions.commandInvocationFailed({
        code: "core.source.failed",
        subsystem: "audio",
        severity: "error",
        retryable: true,
        message: "The source could not be opened.",
      }),
    );

    expect(await screen.findByText("The source could not be opened.")).toBeInTheDocument();
    expect(screen.getByText("Observed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("The source could not be opened.")).not.toBeInTheDocument();
  });

  it("registers an inspected source without displaying it optimistically", async () => {
    const initial = snapshot({
      canCreateHostSession: false,
      hostDraft: { ...snapshot().hostDraft, audioSource: null },
      hostQueue: { revision: "0", entries: [] },
      hostDraftValidation: [
        {
          field: "audioSource",
          code: "audio_source_required",
          message: "audio source is required",
        },
      ],
    });
    const store = renderScreen(initial);
    fireEvent.click(screen.getByRole("button", { name: "Add songs" }));
    await waitFor(() => expect(addHostAudioSources).toHaveBeenCalled());
    expect(
      screen.getByText("No songs added. Add WAV, FLAC, or MP3 files to build the playlist."),
    ).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Adding songs…"));

    store.dispatch(
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot({ revision: "5" }),
      }),
    );
    await waitFor(() => expect(screen.getByText(/set\.wav/)).toBeInTheDocument());
    expect(store.getState().core.pendingCommandReceipts).toEqual({});
  });

  it("treats native dialog cancellation as no command and no error", async () => {
    addHostAudioSources.mockResolvedValue(null);
    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Add songs" }));
    await waitFor(() => expect(addHostAudioSources).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByRole("button", { name: "Add songs" })).toBeEnabled());
    expect(store.getState().core.pendingCommandReceipts).toEqual({});
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows source-selection failure without changing the authoritative source", async () => {
    addHostAudioSources.mockRejectedValue(new Error("permission denied"));
    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Add songs" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("permission denied"));
    expect(store.getState().core.snapshot?.hostDraft.audioSource?.sourceId).toBe("source-1");
  });

  it("keeps create pending until a newer Rust snapshot is observed", async () => {
    const store = renderScreen();
    advanceToReady();
    const create = screen.getByRole("button", { name: "Start hosting" });
    await waitFor(() => expect(create).toBeEnabled());
    fireEvent.click(create);
    await waitFor(() => expect(createHostSession).toHaveBeenCalledWith("4"));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Starting your session…"),
    );
    expect(store.getState().core.snapshot?.hostLifecycle).toBe("idle");

    store.dispatch(
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot({ revision: "5", hostLifecycle: "creating_session" }),
      }),
    );
    await waitFor(() => expect(store.getState().core.pendingCommandReceipts).toEqual({}));
  });

  it("shows invocation rejection and never advances lifecycle locally", async () => {
    createHostSession.mockRejectedValue(new Error("stale revision"));
    const store = renderScreen();
    advanceToReady();
    const create = screen.getByRole("button", { name: "Start hosting" });
    await waitFor(() => expect(create).toBeEnabled());
    fireEvent.click(create);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("stale revision"));
    expect(store.getState().core.snapshot?.hostLifecycle).toBe("idle");
  });

  it("preserves unsaved safe edits when a newer snapshot arrives", () => {
    const store = renderScreen();
    fireEvent.change(screen.getByLabelText("Session name"), {
      target: { value: "Unsaved local edit" },
    });
    store.dispatch(
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot({ revision: "5" }),
      }),
    );
    expect(screen.getByLabelText("Session name")).toHaveValue("Unsaved local edit");
  });
});
