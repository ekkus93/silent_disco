import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  ApproveJoinRequest,
  AttachNotificationResponse,
  BridgeLifecycleDto,
  CommandReceiptDto,
  CoreNotificationDto,
  CoreSnapshotDto,
  DesktopErrorDto,
  HostSessionSnapshotDto,
  JoinRequestCommandRequest,
  ListenerCommandRequest,
  OpenProfileRequest,
  OpenProfileResponse,
  RevisionCommandRequest,
  NetworkInterfaceSnapshotDto,
  SetNetworkBindPreferenceRequest,
  UpdateHostDraftRequest,
} from "./generated/desktop-bindings";

const MAX_ERROR_DETAIL_LENGTH = 320;
const MAX_BRIDGE_ERROR_MESSAGE_LENGTH = 512;
const MAX_COMBINED_ERROR_CODE_LENGTH = 64;
const MAX_COMBINED_ERROR_DETAIL_LENGTH = 128;
const ERROR_WHITESPACE = /\s+/g;

class DesktopBridgeInvocationError extends Error implements DesktopErrorDto {
  readonly code: string;
  readonly subsystem: string;
  readonly severity: string;
  readonly retryable: boolean;

  constructor(error: DesktopErrorDto) {
    super(error.message);
    this.name = "DesktopBridgeInvocationError";
    this.code = error.code;
    this.subsystem = error.subsystem;
    this.severity = error.severity;
    this.retryable = error.retryable;
  }

  toDto(): DesktopErrorDto {
    return {
      code: this.code,
      subsystem: this.subsystem,
      severity: this.severity,
      retryable: this.retryable,
      message: this.message,
    };
  }
}

export interface CoreSmokeDto {
  major: number;
  minor: number;
  patch: number;
  smoke: string;
}

export interface DesktopNotificationSubscription {
  subscriptionId: string;
  channel: Channel<CoreNotificationDto>;
}

export interface OpenProfileSession {
  profile: OpenProfileResponse;
  notifications: DesktopNotificationSubscription;
}

export interface DesktopProfileConnection {
  connectionKind: "opened" | "reattached";
  profile: OpenProfileResponse | null;
  snapshot: CoreSnapshotDto;
  notifications: DesktopNotificationSubscription;
}

function isDesktopErrorDto(error: unknown): error is DesktopErrorDto {
  if (typeof error !== "object" || error === null) {
    return false;
  }
  const candidate = error as Partial<DesktopErrorDto>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.subsystem === "string" &&
    typeof candidate.severity === "string" &&
    typeof candidate.retryable === "boolean" &&
    typeof candidate.message === "string"
  );
}

function singleLineText(value: string): string {
  return Array.from(value, (character) => {
    const codePoint = character.codePointAt(0);
    if (
      codePoint !== undefined &&
      ((codePoint >= 0 && codePoint <= 31) || (codePoint >= 127 && codePoint <= 159))
    ) {
      return " ";
    }
    return character;
  })
    .join("")
    .replaceAll(ERROR_WHITESPACE, " ")
    .trim();
}

function boundedText(value: string, fallback: string, maximumLength: number): string {
  const singleLine = singleLineText(value);
  if (singleLine.length === 0) {
    return fallback;
  }
  return Array.from(singleLine).slice(0, maximumLength).join("");
}

function boundedErrorDetail(error: unknown): string {
  if (error instanceof Error) {
    return boundedText(
      error.message,
      "The native invocation transport failed without a message.",
      MAX_ERROR_DETAIL_LENGTH,
    );
  }
  if (typeof error === "string") {
    return boundedText(
      error,
      "The native invocation transport failed without a message.",
      MAX_ERROR_DETAIL_LENGTH,
    );
  }
  return "The native invocation transport failed without a structured error.";
}

function boundedBridgeMessage(message: string): string {
  return boundedText(
    message,
    "The desktop bridge failed without a usable error message.",
    MAX_BRIDGE_ERROR_MESSAGE_LENGTH,
  );
}

function attachmentCleanupMessage(
  attachmentFailure: DesktopErrorDto,
  closeFailure: DesktopErrorDto,
): string {
  const attachmentCode = boundedText(
    attachmentFailure.code,
    "unknown_attachment_failure",
    MAX_COMBINED_ERROR_CODE_LENGTH,
  );
  const closeCode = boundedText(
    closeFailure.code,
    "unknown_cleanup_failure",
    MAX_COMBINED_ERROR_CODE_LENGTH,
  );
  const attachmentDetail = boundedText(
    attachmentFailure.message,
    "notification attachment failed",
    MAX_COMBINED_ERROR_DETAIL_LENGTH,
  );
  const closeDetail = boundedText(
    closeFailure.message,
    "profile cleanup failed",
    MAX_COMBINED_ERROR_DETAIL_LENGTH,
  );
  return boundedBridgeMessage(
    `Notification attachment failed (${attachmentCode}: ${attachmentDetail}); profile cleanup also failed (${closeCode}: ${closeDetail}).`,
  );
}

export function toDesktopBridgeError(error: unknown, operation: string): DesktopErrorDto {
  if (error instanceof DesktopBridgeInvocationError) {
    return error.toDto();
  }
  if (isDesktopErrorDto(error)) {
    return {
      code: error.code,
      subsystem: error.subsystem,
      severity: error.severity,
      retryable: error.retryable,
      message: boundedBridgeMessage(error.message),
    };
  }
  return {
    code: "desktop.bridge.invoke_transport_failed",
    subsystem: "bridge",
    severity: "error",
    retryable: true,
    message: boundedBridgeMessage(
      `${operation} failed before returning a structured desktop error: ${boundedErrorDetail(error)}`,
    ),
  };
}

function throwDesktopBridgeError(error: DesktopErrorDto): never {
  throw new DesktopBridgeInvocationError(error);
}

async function invokeDesktop<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throwDesktopBridgeError(toDesktopBridgeError(error, command));
  }
}

export async function getCoreSmoke(input: number): Promise<CoreSmokeDto> {
  if (!Number.isSafeInteger(input) || input < 0) {
    throw new Error("Core smoke input must be a non-negative safe integer.");
  }

  return invokeDesktop<CoreSmokeDto>("get_core_smoke", { input });
}

export async function getCurrentSnapshot(): Promise<CoreSnapshotDto> {
  return invokeDesktop<CoreSnapshotDto>("get_current_snapshot");
}

export async function attachNotifications(
  onNotification: (notification: CoreNotificationDto) => void,
): Promise<DesktopNotificationSubscription> {
  let channel: Channel<CoreNotificationDto>;
  try {
    channel = new Channel<CoreNotificationDto>();
    channel.onmessage = onNotification;
  } catch (error: unknown) {
    throwDesktopBridgeError(toDesktopBridgeError(error, "create notification channel"));
  }

  const response = await invokeDesktop<AttachNotificationResponse>("attach_notifications", {
    channel,
  });
  return {
    subscriptionId: response.subscriptionId,
    channel,
  };
}

export async function openProfileWithNotifications(
  profileId: string,
  onNotification: (notification: CoreNotificationDto) => void,
): Promise<OpenProfileSession> {
  const request: OpenProfileRequest = { profileId };
  const profile = await invokeDesktop<OpenProfileResponse>("open_profile", { request });

  try {
    const notifications = await attachNotifications(onNotification);
    return { profile, notifications };
  } catch (attachmentError: unknown) {
    const attachmentFailure = toDesktopBridgeError(
      attachmentError,
      "attach notifications after profile open",
    );
    try {
      await closeProfile();
    } catch (closeError: unknown) {
      const closeFailure = toDesktopBridgeError(
        closeError,
        "close profile after attachment failure",
      );
      throwDesktopBridgeError({
        code: "desktop.bridge.attach_cleanup_failed",
        subsystem: "bridge",
        severity: "fatal",
        retryable: false,
        message: attachmentCleanupMessage(attachmentFailure, closeFailure),
      });
    }
    throwDesktopBridgeError(attachmentFailure);
  }
}

export async function connectProfileWithNotifications(
  profileId: string,
  onNotification: (notification: CoreNotificationDto) => void,
): Promise<DesktopProfileConnection> {
  let existingNotifications: DesktopNotificationSubscription;
  try {
    existingNotifications = await attachNotifications(onNotification);
  } catch (error: unknown) {
    const failure = toDesktopBridgeError(error, "attach to existing profile");
    if (failure.code !== "desktop.profile.not_ready") {
      throwDesktopBridgeError(failure);
    }
    const session = await openProfileWithNotifications(profileId, onNotification);
    return {
      connectionKind: "opened",
      profile: session.profile,
      snapshot: session.profile.snapshot,
      notifications: session.notifications,
    };
  }

  const snapshot = await getCurrentSnapshot();
  return {
    connectionKind: "reattached",
    profile: null,
    snapshot,
    notifications: existingNotifications,
  };
}

export async function selectHostRole(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("select_host_role", { request });
}

export async function updateHostDraft(request: UpdateHostDraftRequest): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("update_host_draft", { request });
}

export async function createHostSession(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("create_host_session", { request });
}

export async function getHostSessionState(): Promise<HostSessionSnapshotDto> {
  return invokeDesktop<HostSessionSnapshotDto>("get_host_session_state");
}

export async function startHostPlayback(): Promise<void> {
  return invokeDesktop<void>("start_host_playback");
}

export async function pauseHostPlayback(): Promise<void> {
  return invokeDesktop<void>("pause_host_playback");
}

export async function resumeHostPlayback(): Promise<void> {
  return invokeDesktop<void>("resume_host_playback");
}

export async function stopHostPlayback(): Promise<void> {
  return invokeDesktop<void>("stop_host_playback");
}

export async function approveJoinRequest(request: ApproveJoinRequest): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("approve_join_request", { request });
}

export async function rejectJoinRequest(
  request: JoinRequestCommandRequest,
): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("reject_join_request", { request });
}

export async function removeListener(request: ListenerCommandRequest): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("remove_listener", { request });
}

export async function endHostSession(expectedRevision: string): Promise<CommandReceiptDto> {
  const request: RevisionCommandRequest = { expectedRevision };
  return invokeDesktop<CommandReceiptDto>("end_host_session", { request });
}

export async function getHostNetworkState(): Promise<NetworkInterfaceSnapshotDto> {
  return invokeDesktop<NetworkInterfaceSnapshotDto>("get_host_network_state");
}

export async function setHostNetworkPreference(
  request: SetNetworkBindPreferenceRequest,
): Promise<NetworkInterfaceSnapshotDto> {
  return invokeDesktop<NetworkInterfaceSnapshotDto>("set_host_network_preference", { request });
}

export async function closeProfile(): Promise<BridgeLifecycleDto> {
  return invokeDesktop<BridgeLifecycleDto>("close_profile");
}
