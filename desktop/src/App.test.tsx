import "@testing-library/jest-dom/vitest";

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  getLabStateMock,
  getStorageInspectionMock,
  getHostDiagnosticsMock,
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
  getLabStateMock: vi.fn(),
  getStorageInspectionMock: vi.fn(),
  getHostDiagnosticsMock: vi.fn(),
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
    getLabState: getLabStateMock,
    getStorageInspection: getStorageInspectionMock,
    getHostDiagnostics: getHostDiagnosticsMock,
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
  hostQueue: { revision: "0", entries: [] },
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
  allowSharedAddressSpace: false,
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
  lastDeliveryOperationId: null,
  recoverableAction: null,
  playbackControlsEnabled: false,
  transportWorkerRunning: true,
  transportError: null,
  playbackPositionMs: "0",
  streamEndedNaturally: false,
  audioSource: null,
  hostQueue: { revision: "0", entries: [] },
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

const storageInspection = {
  sqliteVersion: "3.45.0",
  foreignKeysEnabled: true,
  journalMode: "wal",
  busyTimeoutMs: 5000,
  synchronousPolicy: "full",
  schemaVersion: 2,
  appliedMigrations: [],
  integrityCheck: "ok",
  settings: null,
  trustedDevices: [],
  recentSessions: [],
  p2StoreApplicable: true,
};

const hostDiagnostics = {
  versions: {
    coreVersion: { major: 0, minor: 1, patch: 0 },
    appVersion: "0.1.0",
    exportSchemaVersion: 1,
  },
  profile: { profileId: "main", platform: "linux" },
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
    window.localStorage.clear();
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
    getLabStateMock.mockReset();
    getStorageInspectionMock.mockReset();
    getHostDiagnosticsMock.mockReset();
    getStorageInspectionMock.mockResolvedValue(storageInspection);
    getHostDiagnosticsMock.mockResolvedValue(hostDiagnostics);
    getLabStateMock.mockResolvedValue({
      nowMs: "0",
      running: false,
      paused: false,
      nodes: [],
      loadedScenario: null,
      lastRun: null,
    });
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

  it("opens the profile ID saved for this startup", async () => {
    window.localStorage.setItem("silent-disco.profile-id", "oakland-night");
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(ensureDesktopBridgeMock).toHaveBeenCalledWith("oakland-night");
  });

  it("saves a valid next-start profile ID in Settings", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();
    fireEvent.click(await screen.findByRole("tab", { name: "Settings" }));

    const field = await screen.findByRole("textbox", { name: "Profile ID for next startup" });
    fireEvent.change(field, { target: { value: "oakland-night" } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile ID" }));

    expect(window.localStorage.getItem("silent-disco.profile-id")).toBe("oakland-night");
    expect(screen.getByText(/Restart Silent Disco to open profile/)).toHaveTextContent(
      "The current session stays on main.",
    );
  });

  it("rejects an invalid profile ID in Settings without saving it", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();
    fireEvent.click(await screen.findByRole("tab", { name: "Settings" }));

    const field = await screen.findByRole("textbox", { name: "Profile ID for next startup" });
    fireEvent.change(field, { target: { value: "Oakland Night" } });
    fireEvent.click(screen.getByRole("button", { name: "Save profile ID" }));

    expect(screen.getByRole("alert")).toHaveTextContent("Use 1–64 lowercase letters");
    expect(window.localStorage.getItem("silent-disco.profile-id")).toBeNull();
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

  // Block 42 runtime gate: when the backend reports Lab unavailable there
  // is no way to reach LabScreen -- no nav control renders it and no Lab
  // command is called. Physical *bundle absence* is a separate build-time
  // property enforced by scripts/verify-production-lab-absence.mjs.
  it("has no Lab Mode entry point at all when the backend reports it unavailable", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    getLabModeAvailableMock.mockResolvedValue(false);
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(screen.queryByRole("tab", { name: /lab mode/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/developer testing tool/i)).not.toBeInTheDocument();
    expect(getLabStateMock).not.toHaveBeenCalled();
  });

  // The mirror image: when the backend reports Lab Mode compiled in, the
  // nav control exists and actually reveals `LabScreen`'s own content --
  // proving the gate is a real conditional render, not merely an unused
  // button.
  it("reveals LabScreen content only after Lab Mode is available and requested", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    getLabModeAvailableMock.mockResolvedValue(true);
    renderApp();

    const labButton = await screen.findByRole("tab", { name: "Lab Mode" });
    expect(screen.queryByText(/developer testing tool/i)).not.toBeInTheDocument();

    fireEvent.click(labButton);

    expect(await screen.findByText(/developer testing tool/i)).toBeVisible();
  });
  // Block 49.1: activating a destination used to append it below the active
  // screen, so on a tall session page the click appeared to do nothing.
  // Exactly one destination renders, and focus follows the switch.
  it("renders exactly one destination at a time and moves focus on switch", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    expect(screen.queryByRole("heading", { name: "Settings" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "Settings" }));

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeVisible();
    // The host destination is gone, not merely scrolled past.
    expect(
      screen.queryByRole("heading", { name: "Create a silent disco" }),
    ).not.toBeInTheDocument();

    const panel = screen.getByRole("tabpanel");
    expect(panel).toHaveFocus();
    expect(screen.getByRole("tab", { name: "Settings" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Host control" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  // Block 49.4: the previous render gated this screen on `ready`, contradicting
  // its own code comment -- it was unavailable in exactly the state it was
  // written for.
  it("keeps storage inspection reachable when the bridge failed to open", async () => {
    ensureDesktopBridgeMock.mockRejectedValue(new Error("native bridge unavailable"));
    renderApp();

    expect(await screen.findByRole("alert")).toHaveTextContent("Desktop bridge startup failed");

    fireEvent.click(screen.getByRole("tab", { name: "Storage" }));

    expect(await screen.findByRole("tabpanel")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Storage" })).toHaveAttribute("aria-selected", "true");
  });

  // Block 49.4: the core publishes a recovery hint on the snapshot and every
  // screen previously discarded it.
  it("renders the core's recoverable action when one is published", async () => {
    ensureDesktopBridgeMock.mockResolvedValue({
      ...connection,
      snapshot: { ...snapshot, recoverableAction: "Reconnect the audio output device" },
    });
    renderApp();

    expect(await screen.findByText("Reconnect the audio output device")).toBeVisible();
  });

  // Block 49.3: `retryable` tells an operator whether trying again is worth
  // anything, and no previous error renderer showed it.
  it("renders every error field including retryable on bridge failure", async () => {
    ensureDesktopBridgeMock.mockRejectedValue(new Error("native bridge unavailable"));
    renderApp();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Retryable");
    expect(alert).toHaveTextContent("Subsystem");
    expect(alert).toHaveTextContent("Severity");
  });

  // Block 50.2: network policy and mDNS status were unreachable during a live
  // session because they lived inside the host-setup form, which unmounts.
  it("exposes network policy from Settings during an active session", async () => {
    ensureDesktopBridgeMock.mockResolvedValue({
      ...connection,
      snapshot: { ...snapshot, hostLifecycle: "streaming" },
    });
    renderApp();

    fireEvent.click(await screen.findByRole("tab", { name: "Settings" }));

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeVisible();
    expect(getHostNetworkStateMock).toHaveBeenCalled();
  });

  // Block 50.2: the control lives on Settings now, but host setup still gates
  // session creation on network readiness, so the handoff must actually work.
  it("hands off from host setup to Settings for network configuration", async () => {
    ensureDesktopBridgeMock.mockResolvedValue({
      ...connection,
      snapshot: {
        ...snapshot,
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
      },
    });
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    // Setup shows the outcome, not a second copy of the control.
    expect(await screen.findByText(/Ready to bind enp1s0 at 192\.168\.1\.20/)).toBeVisible();
    expect(screen.queryByLabelText("Host network interface")).not.toBeInTheDocument();
    // Paired with the unresolved-policy test below: together these prove the
    // readiness gate actually toggles rather than being permanently disabled.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /start hosting/i })).toBeEnabled(),
    );

    fireEvent.click(screen.getByRole("button", { name: "Configure in Settings" }));

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeVisible();
    // The actual control is here.
    expect(await screen.findByLabelText("Host network interface")).toBeVisible();
  });

  // The readiness gate must still block creation when the policy does not
  // resolve -- that is the only reason setup keeps a readiness signal at all.
  it("blocks session creation while network policy does not resolve", async () => {
    getHostNetworkStateMock.mockResolvedValue({
      ...networkSnapshot,
      automaticSelection: null,
      resolvedSelection: null,
      requiresExplicitSelection: true,
      selectionError: "multiple private-LAN addresses require an explicit selection",
    });
    ensureDesktopBridgeMock.mockResolvedValue({
      ...connection,
      snapshot: {
        ...snapshot,
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
      },
    });
    renderApp();

    await screen.findByRole("heading", { name: "Create a silent disco" });
    fireEvent.click(screen.getByRole("button", { name: "Next: Access" }));
    fireEvent.click(screen.getByRole("button", { name: "Review session" }));
    expect(
      await screen.findByText(/multiple private-LAN addresses require an explicit selection/),
    ).toBeVisible();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /start hosting/i })).toBeDisabled(),
    );
  });

  it("reports Lab Mode availability in Settings", async () => {
    getLabModeAvailableMock.mockResolvedValue(true);
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();

    fireEvent.click(await screen.findByRole("tab", { name: "Settings" }));

    expect(await screen.findByText("Lab Mode build")).toBeVisible();
    expect(await screen.findByText("available")).toBeVisible();
  });
});
