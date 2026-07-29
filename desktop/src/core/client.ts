import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  AttachNotificationResponse,
  BridgeLifecycleDto,
  CoreNotificationDto,
  CoreSnapshotDto,
  DesktopErrorDto,
  OpenProfileRequest,
  OpenProfileResponse,
} from "./generated/desktop-bindings";

const MAX_ERROR_DETAIL_LENGTH = 320;
const MAX_BRIDGE_ERROR_MESSAGE_LENGTH = 768;

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

function boundedErrorDetail(error: unknown): string {
  let detail: string;
  if (error instanceof Error) {
    detail = error.message;
  } else if (typeof error === "string") {
    detail = error;
  } else {
    detail = "The native invocation transport failed without a structured error.";
  }
  const singleLine = detail.replaceAll(/\s+/g, " ").trim();
  if (singleLine.length === 0) {
    return "The native invocation transport failed without a message.";
  }
  return singleLine.slice(0, MAX_ERROR_DETAIL_LENGTH);
}

function boundedBridgeMessage(message: string): string {
  return message.slice(0, MAX_BRIDGE_ERROR_MESSAGE_LENGTH);
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
      message: error.message,
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

async function invokeDesktop<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
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
        message: boundedBridgeMessage(
          `Notification attachment failed (${attachmentFailure.code}: ${attachmentFailure.message}); profile cleanup also failed (${closeFailure.code}: ${closeFailure.message}).`,
        ),
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

export async function closeProfile(): Promise<BridgeLifecycleDto> {
  return invokeDesktop<BridgeLifecycleDto>("close_profile");
}
