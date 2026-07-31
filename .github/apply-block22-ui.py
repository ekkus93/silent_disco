from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"UI anchor count {count} for {path}: {old[:160]!r}")
    target.write_text(source.replace(old, new, 1))


replace_once(
    "desktop/src/core/client.ts",
    """  CoreSnapshotDto,
  DesktopErrorDto,
  OpenProfileRequest,
""",
    """  CoreSnapshotDto,
  DesktopErrorDto,
  HostSessionSnapshotDto,
  OpenProfileRequest,
""",
)
replace_once(
    "desktop/src/core/client.ts",
    """export async function createHostSession(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("create_host_session", { request });
}

export async function getHostNetworkState(): Promise<NetworkInterfaceSnapshotDto> {
""",
    """export async function createHostSession(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("create_host_session", { request });
}

export async function getHostSessionState(): Promise<HostSessionSnapshotDto> {
  return invokeDesktop<HostSessionSnapshotDto>("get_host_session_state");
}

export async function endHostSession(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("end_host_session", { request });
}

export async function getHostNetworkState(): Promise<NetworkInterfaceSnapshotDto> {
""",
)

replace_once(
    "desktop/src/App.tsx",
    'import { HostSetupScreen } from "./screens/HostSetupScreen";\n',
    'import { HostSessionScreen } from "./screens/HostSessionScreen";\nimport { HostSetupScreen } from "./screens/HostSetupScreen";\n',
)
replace_once(
    "desktop/src/App.tsx",
    """interface ShellConnectionState {
  connectionKind: "opened" | "reattached";
  subscriptionId: string;
}
""",
    """interface ShellConnectionState {
  connectionKind: "opened" | "reattached";
  subscriptionId: string;
}

const ACTIVE_HOST_LIFECYCLES = new Set([
  "creating_session",
  "advertising",
  "waiting_for_listeners",
  "ready",
  "streaming",
  "paused",
  "ending_session",
  "error",
]);
""",
)
replace_once(
    "desktop/src/App.tsx",
    """          {ready ? <HostSetupScreen /> : null}
""",
    """          {ready && ACTIVE_HOST_LIFECYCLES.has(snapshot.hostLifecycle) ? (
            <HostSessionScreen />
          ) : null}
          {ready && !ACTIVE_HOST_LIFECYCLES.has(snapshot.hostLifecycle) ? (
            <HostSetupScreen />
          ) : null}
""",
)

replace_once(
    "desktop/src/App.test.tsx",
    """  CoreNotificationDto,
  CoreSnapshotDto,
  NetworkInterfaceSnapshotDto,
""",
    """  CoreNotificationDto,
  CoreSnapshotDto,
  HostSessionSnapshotDto,
  NetworkInterfaceSnapshotDto,
""",
)
replace_once(
    "desktop/src/App.test.tsx",
    """  createHostSessionMock,
  getHostNetworkStateMock,
  setHostNetworkPreferenceMock,
""",
    """  createHostSessionMock,
  getHostSessionStateMock,
  endHostSessionMock,
  getHostNetworkStateMock,
  setHostNetworkPreferenceMock,
""",
)
replace_once(
    "desktop/src/App.test.tsx",
    """  createHostSessionMock: vi.fn(),
  getHostNetworkStateMock: vi.fn(),
  setHostNetworkPreferenceMock: vi.fn(),
""",
    """  createHostSessionMock: vi.fn(),
  getHostSessionStateMock: vi.fn(),
  endHostSessionMock: vi.fn(),
  getHostNetworkStateMock: vi.fn(),
  setHostNetworkPreferenceMock: vi.fn(),
""",
)
replace_once(
    "desktop/src/App.test.tsx",
    """    createHostSession: createHostSessionMock,
    getHostNetworkState: getHostNetworkStateMock,
""",
    """    createHostSession: createHostSessionMock,
    getHostSessionState: getHostSessionStateMock,
    endHostSession: endHostSessionMock,
    getHostNetworkState: getHostNetworkStateMock,
""",
)
insert_anchor = """const connection = {
"""
host_fixture = """const hostSessionSnapshot: HostSessionSnapshotDto = {
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
  playbackControlsEnabled: false,
  transportWorkerRunning: true,
  transportError: null,
  lastError: null,
};

"""
replace_once("desktop/src/App.test.tsx", insert_anchor, host_fixture + insert_anchor)
replace_once(
    "desktop/src/App.test.tsx",
    """    createHostSessionMock.mockReset();
    getHostNetworkStateMock.mockReset();
""",
    """    createHostSessionMock.mockReset();
    getHostSessionStateMock.mockReset();
    endHostSessionMock.mockReset();
    getHostNetworkStateMock.mockReset();
""",
)
replace_once(
    "desktop/src/App.test.tsx",
    """    getHostNetworkStateMock.mockResolvedValue(networkSnapshot);
    setHostNetworkPreferenceMock.mockResolvedValue(networkSnapshot);
""",
    """    getHostSessionStateMock.mockResolvedValue(hostSessionSnapshot);
    endHostSessionMock.mockResolvedValue({ operationId: "end-1", acceptedAtRevision: "5" });
    getHostNetworkStateMock.mockResolvedValue(networkSnapshot);
    setHostNetworkPreferenceMock.mockResolvedValue(networkSnapshot);
""",
)
replace_once(
    "desktop/src/App.test.tsx",
    """    expect(screen.getByText(/Rust host lifecycle: waiting_for_listeners/)).toBeVisible();
    expect(store.getState().core.snapshot?.revision).toBe("5");
""",
    """    expect(await screen.findByRole("heading", { name: "Host session" })).toBeVisible();
    expect(screen.getByText("session-app-test")).toBeVisible();
    expect(store.getState().core.snapshot?.revision).toBe("5");
""",
)
