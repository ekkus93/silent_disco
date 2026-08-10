import "@testing-library/jest-dom/vitest";

import { act, render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { createAppStore } from "./app/store";
import type {
  CoreNotificationDto,
  CoreSnapshotDto,
  HostSessionSnapshotDto,
  NetworkInterfaceSnapshotDto,
} from "./core/generated/desktop-bindings";

const {
  ensureDesktopBridgeMock,
  subscribeDesktopNotificationsMock,
  unsubscribeMock,
  selectHostRoleMock,
  updateHostDraftMock,
  createHostSessionMock,
  getHostSessionStateMock,
  endHostSessionMock,
  getHostNetworkStateMock,
  setHostNetworkPreferenceMock,
  getLabModeAvailableMock,
} = vi.hoisted(() => ({
  ensureDesktopBridgeMock: vi.fn(),
  subscribeDesktopNotificationsMock: vi.fn(),
  unsubscribeMock: vi.fn(),
  selectHostRoleMock: vi.fn(),
  updateHostDraftMock: vi.fn(),
  createHostSessionMock: vi.fn(),
  getHostSessionStateMock: vi.fn(),
  endHostSessionMock: vi.fn(),
  getHostNetworkStateMock: vi.fn(),
  setHostNetworkPreferenceMock: vi.fn(),
  getLabModeAvailableMock: vi.fn(),
}));

let notificationListener: ((notification: CoreNotificationDto) => void) | undefined;

vi.mock("./core/bridge", () => ({
  ensureDesktopBridge: ensureDesktopBridgeMock,
  subscribeDesktopNotifications: subscribeDesktopNotificationsMock,
}));
vi.mock("./core/client", async () => {
  const actual = await vi.importActual<typeof import("./core/client")>("./core/client");
  return {
    ...actual,
    selectHostRole: selectHostRoleMock,
    updateHostDraft: updateHostDraftMock,
    createHostSession: createHostSessionMock,
    getHostSessionState: getHostSessionStateMock,
    endHostSession: endHostSessionMock,
    getHostNetworkState: getHostNetworkStateMock,
    setHostNetworkPreference: setHostNetworkPreferenceMock,
    getLabModeAvailable: getLabModeAvailableMock,
  };
});

const snapshot: CoreSnapshotDto = {
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
};

const networkCandidate = {
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

const networkSnapshot: NetworkInterfaceSnapshotDto = {
  preference: { mode: "automatic", interfaceName: null, address: null },
  candidates: [networkCandidate],
  automaticSelection: networkCandidate,
  resolvedSelection: networkCandidate,
  requiresExplicitSelection: false,
  selectionError: null,
  activeBinding: null,
  activeBindingValid: false,
  interfaceChange: null,
};

const hostSessionSnapshot: HostSessionSnapshotDto = {
  revision: "5",
  hostLifecycle: "waiting_for_listeners",
  transportState: "connected",
  playbackState: "stopped",
  sessionName: "Oakland Night",
  connection: {
    hostAddress: "192.168.1.20",
    controlPort: 47000,
    syncPort: 47001,
    audioPort: 47002,
    sessionId: "session-app-test",
    protocolVersion: 2,
    inviteCodeRequired: false,
    expiresAtMs: null,
  },
  pendingJoinRequests: [],
  connectedListeners: [],
  lastDelivery: null,
  recoverableAction: null,
  playbackControlsEnabled: false,
  transportWorkerRunning: true,
  transportError: null,
  playbackPositionMs: "0",
  streamEndedNaturally: false,
  audioSource: null,
  broadcast: null,
  lastError: null,
  monitor: { enabled: false, active: false, failureReason: null },
};

const connection = {
  connectionKind: "opened" as const,
  profile: null,
  snapshot,
  notifications: { subscriptionId: "17", channel: {} },
};

function renderApp() {
  const store = createAppStore();
  return {
    store,
    view: render(
      <Provider store={store}>
        <App />
      </Provider>,
    ),
  };
}

describe("App", () => {
  beforeEach(() => {
    notificationListener = undefined;
    ensureDesktopBridgeMock.mockReset();
    subscribeDesktopNotificationsMock.mockReset();
    unsubscribeMock.mockReset();
    selectHostRoleMock.mockReset();
    updateHostDraftMock.mockReset();
    createHostSessionMock.mockReset();
    getHostSessionStateMock.mockReset();
    endHostSessionMock.mockReset();
    getHostNetworkStateMock.mockReset();
    setHostNetworkPreferenceMock.mockReset();
    getLabModeAvailableMock.mockReset();
    getHostSessionStateMock.mockResolvedValue(hostSessionSnapshot);
    endHostSessionMock.mockResolvedValue({ operationId: "end-1", acceptedAtRevision: "5" });
    getHostNetworkStateMock.mockResolvedValue(networkSnapshot);
    setHostNetworkPreferenceMock.mockResolvedValue(networkSnapshot);
    getLabModeAvailableMock.mockResolvedValue(false);
    subscribeDesktopNotificationsMock.mockImplementation(
      (listener: (notification: CoreNotificationDto) => void) => {
        notificationListener = listener;
        return unsubscribeMock;
      },
    );
  });

  it("opens the bridge and renders the authoritative Host Setup screen", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    const { store } = renderApp();

    expect(screen.getByRole("status")).toHaveTextContent("Opening or reattaching");
    expect(await screen.findByRole("heading", { name: "Create a silent disco" })).toBeVisible();
    expect(screen.getByLabelText("Session name")).toHaveValue("Oakland Night");
    expect(store.getState().core.snapshot).toEqual(snapshot);
    expect(ensureDesktopBridgeMock).toHaveBeenCalledWith("main");
  });

  it("renders only a newer complete authoritative snapshot", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    const { store } = renderApp();
    await screen.findByRole("heading", { name: "Create a silent disco" });

    act(() => {
      notificationListener?.({
        kind: "snapshot",
        details: { ...snapshot, revision: "5", hostLifecycle: "waiting_for_listeners" },
      });
    });

    expect(await screen.findByRole("heading", { name: "Oakland Night" })).toBeVisible();
    expect(screen.getByText("session-app-test")).toBeVisible();
    expect(store.getState().core.snapshot?.revision).toBe("5");
  });

  it("keeps a core command failure visible", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();
    await screen.findByRole("heading", { name: "Create a silent disco" });

    act(() => {
      notificationListener?.({
        kind: "error",
        details: {
          code: "core.command.rejected",
          subsystem: "runtime",
          severity: "error",
          retryable: false,
          message: "The command was rejected.",
        },
      });
    });

    expect(screen.getByRole("alert")).toHaveTextContent("Host setup command failed");
    expect(screen.getByRole("alert")).toHaveTextContent("The command was rejected.");
  });

  it("keeps bridge startup failure visible as a structured Redux error", async () => {
    ensureDesktopBridgeMock.mockRejectedValue(new Error("native bridge unavailable"));
    const { store } = renderApp();

    expect(await screen.findByRole("alert")).toHaveTextContent("Desktop bridge startup failed");
    expect(screen.getByRole("alert")).toHaveTextContent("native bridge unavailable");
    expect(store.getState().core.bridgeLifecycle.kind).toBe("failed");
  });

  it("unsubscribes the React listener without closing the native channel", () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    const { view } = renderApp();
    view.unmount();
    expect(unsubscribeMock).toHaveBeenCalledOnce();
  });

  // Block 37.1 "UI is visibly labeled": the badge appears only when the
  // *backend* reports Lab Mode compiled in -- never inferred from a
  // JS-only dev/test environment flag.
  it("shows the Lab Mode badge only when the backend reports it available", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    getLabModeAvailableMock.mockResolvedValue(true);
    renderApp();

    expect(await screen.findByText(/lab mode build/i)).toBeVisible();
  });

  it("shows no Lab Mode badge when the backend reports it unavailable", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    getLabModeAvailableMock.mockResolvedValue(false);
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(screen.queryByText(/lab mode build/i)).not.toBeInTheDocument();
  });
});
