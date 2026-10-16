// Deterministic DTO builders shared by the host workflow, accessibility, and
// golden tests. Every value is fixed so rendered output never depends on the
// wall clock, the host network, or ordering of a real core.
import type {
  ConnectedListenerDto,
  CoreSnapshotDto,
  HostQueueEntryDto,
  HostSessionSnapshotDto,
  NetworkInterfaceSnapshotDto,
  PendingJoinRequestDto,
} from "../core/generated/desktop-bindings";

export function queueEntry(
  id: string,
  displayName: string,
  durationMs = "222000",
): HostQueueEntryDto {
  return {
    entryId: `entry-${id}`,
    source: { sourceId: `source-${id}`, displayName, byteLength: "4096", durationMs },
  };
}

export const GOLDEN_HOUR = queueEntry("golden", "Golden Hour.flac", "222000");
export const SOFT_FOCUS = queueEntry("soft", "Soft Focus.wav", "248000");
export const AFTERGLOW = queueEntry("after", "Afterglow.mp3", "259000");

export function coreSnapshot(overrides: Partial<CoreSnapshotDto> = {}): CoreSnapshotDto {
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
      sessionName: "Friday night",
      approvalMode: "manual",
      inviteCode: null,
      audioSource: GOLDEN_HOUR.source,
      rememberApprovedDevices: false,
    },
    hostQueue: { revision: "3", entries: [GOLDEN_HOUR, SOFT_FOCUS, AFTERGLOW] },
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

export function pendingRequest(
  id: string,
  displayName: string,
  overrides: Partial<PendingJoinRequestDto> = {},
): PendingJoinRequestDto {
  return {
    requestId: `request-${id}`,
    deviceId: `device-${id}`,
    displayName,
    trustState: "session_only",
    inviteCodeValid: true,
    receivedAtMs: "100",
    ageMs: "2000",
    ...overrides,
  };
}

export function listener(
  id: string,
  displayName: string,
  overrides: Partial<ConnectedListenerDto> = {},
): ConnectedListenerDto {
  return {
    deviceId: `device-${id}`,
    displayName,
    trustState: "session_only",
    transportState: "connected",
    lastContactMs: "500",
    lastContactAgeMs: "250",
    syncConfidence: "good",
    estimatedOffsetMs: "-2.5",
    roundTripTimeMs: "18",
    driftPpm: "1.2",
    lastControlDeliveryState: "ok",
    retryAvailable: false,
    resyncAvailable: false,
    canRemove: true,
    lastError: null,
    ...overrides,
  };
}

export function hostSession(
  overrides: Partial<HostSessionSnapshotDto> = {},
): HostSessionSnapshotDto {
  return {
    revision: "20",
    hostLifecycle: "waiting_for_listeners",
    transportState: "advertising",
    playbackState: "stopped",
    playbackPositionMs: "0",
    streamEndedNaturally: false,
    audioSource: GOLDEN_HOUR.source,
    hostQueue: { revision: "3", entries: [GOLDEN_HOUR, SOFT_FOCUS, AFTERGLOW] },
    sessionName: "Friday night",
    connection: {
      hostAddress: "192.168.1.20",
      controlPort: 47000,
      syncPort: 47001,
      audioPort: 47002,
      sessionId: "session-friday",
      protocolVersion: 2,
      inviteCodeRequired: false,
      expiresAtMs: null,
    },
    pendingJoinRequests: [],
    connectedListeners: [],
    lastDelivery: null,
    lastDeliveryOperationId: null,
    recoverableAction: null,
    playbackControlsEnabled: true,
    transportWorkerRunning: true,
    transportError: null,
    broadcast: null,
    lastError: null,
    monitor: { enabled: false, active: false, failureReason: null },
    ...overrides,
  };
}

const NETWORK_CANDIDATE = {
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

export function networkSnapshot(): NetworkInterfaceSnapshotDto {
  return {
    preference: { mode: "automatic", interfaceName: null, address: null },
    candidates: [NETWORK_CANDIDATE],
    automaticSelection: NETWORK_CANDIDATE,
    resolvedSelection: NETWORK_CANDIDATE,
    requiresExplicitSelection: false,
    selectionError: null,
    activeBinding: {
      interfaceName: "enp1s0",
      address: "192.168.1.20",
      controlPort: 47000,
      syncPort: 47001,
      audioPort: 47002,
      mdns: { active: true, failureReason: null },
    },
    activeBindingValid: true,
    interfaceChange: null,
    allowSharedAddressSpace: false,
  };
}
