import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { coreActions } from "../app/coreSlice";
import { createAppStore } from "../app/store";
import type { CoreSnapshotDto } from "../core/generated/desktop-bindings";
import { HostSetupScreen } from "./HostSetupScreen";

const { selectAudioSource, selectHostRole, updateHostDraft, createHostSession } = vi.hoisted(() => ({
  selectAudioSource: vi.fn(),
  selectHostRole: vi.fn(),
  updateHostDraft: vi.fn(),
  createHostSession: vi.fn(),
}));

vi.mock("../core/audioSourceClient", () => ({ selectAudioSource }));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return { ...actual, selectHostRole, updateHostDraft, createHostSession };
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

beforeEach(() => {
  vi.clearAllMocks();
  selectAudioSource.mockResolvedValue({ operationId: "source-2", acceptedAtRevision: "4" });
  selectHostRole.mockResolvedValue({ operationId: "role-1", acceptedAtRevision: "4" });
  updateHostDraft.mockResolvedValue({ operationId: "draft-1", acceptedAtRevision: "4" });
  createHostSession.mockResolvedValue({ operationId: "create-1", acceptedAtRevision: "4" });
});

describe("HostSetupScreen", () => {
  it("uses native labelled controls in keyboard order", () => {
    renderScreen();
    const session = screen.getByLabelText("Session name");
    const approval = screen.getByLabelText("Listener approval");
    const remember = screen.getByLabelText("Remember approved devices");
    const select = screen.getByRole("button", { name: "Select audio file" });
    const validate = screen.getByRole("button", { name: "Validate settings" });
    expect(
      session.compareDocumentPosition(approval) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      approval.compareDocumentPosition(remember) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      remember.compareDocumentPosition(select) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      select.compareDocumentPosition(validate) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("shows invite-code controls only for invite-code approval", () => {
    renderScreen();
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
    expect(screen.getByText("session name is invalid")).toBeInTheDocument();
    expect(screen.getByLabelText("Session name")).toHaveAttribute(
      "aria-describedby",
      "session-name-error",
    );
  });

  it("registers an inspected source without displaying it optimistically", async () => {
    const initial = snapshot({
      canCreateHostSession: false,
      hostDraft: { ...snapshot().hostDraft, audioSource: null },
      hostDraftValidation: [
        { field: "audioSource", code: "audio_source_required", message: "audio source is required" },
      ],
    });
    const store = renderScreen(initial);
    fireEvent.click(screen.getByRole("button", { name: "Select audio file" }));
    await waitFor(() => expect(selectAudioSource).toHaveBeenCalledWith("4"));
    expect(screen.getByText("No inspected source selected.")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Waiting for a newer Rust snapshot"),
    );

    store.dispatch(
      coreActions.notificationReceived({
        kind: "snapshot",
        details: snapshot({ revision: "5" }),
      }),
    );
    await waitFor(() => expect(screen.getByText("set.wav")).toBeInTheDocument());
    expect(store.getState().core.pendingCommandReceipts).toEqual({});
  });

  it("treats native dialog cancellation as no command and no error", async () => {
    selectAudioSource.mockResolvedValue(null);
    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Select audio file" }));
    await waitFor(() => expect(selectAudioSource).toHaveBeenCalledWith("4"));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Select audio file" })).toBeEnabled(),
    );
    expect(store.getState().core.pendingCommandReceipts).toEqual({});
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows source-selection failure without changing the authoritative source", async () => {
    selectAudioSource.mockRejectedValue(new Error("permission denied"));
    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Select audio file" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("permission denied"));
    expect(store.getState().core.snapshot?.hostDraft.audioSource?.sourceId).toBe("source-1");
  });

  it("keeps create pending until a newer Rust snapshot is observed", async () => {
    const store = renderScreen();
    fireEvent.click(screen.getByRole("button", { name: "Create session" }));
    await waitFor(() => expect(createHostSession).toHaveBeenCalledWith("4"));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Waiting for a newer Rust snapshot"),
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
    fireEvent.click(screen.getByRole("button", { name: "Create session" }));
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
