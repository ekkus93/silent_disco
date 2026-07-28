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

export async function getCoreSmoke(input: number): Promise<CoreSmokeDto> {
  if (!Number.isSafeInteger(input) || input < 0) {
    throw new Error("Core smoke input must be a non-negative safe integer.");
  }

  return invoke<CoreSmokeDto>("get_core_smoke", { input });
}

export async function getCurrentSnapshot(): Promise<CoreSnapshotDto> {
  return invoke<CoreSnapshotDto>("get_current_snapshot");
}

export async function attachNotifications(
  onNotification: (notification: CoreNotificationDto) => void,
): Promise<DesktopNotificationSubscription> {
  const channel = new Channel<CoreNotificationDto>();
  channel.onmessage = onNotification;
  const response = await invoke<AttachNotificationResponse>("attach_notifications", { channel });
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
  const profile = await invoke<OpenProfileResponse>("open_profile", { request });

  try {
    const notifications = await attachNotifications(onNotification);
    return { profile, notifications };
  } catch (attachmentError: unknown) {
    try {
      await closeProfile();
    } catch (closeError: unknown) {
      throw new AggregateError(
        [attachmentError, closeError],
        "Notification attachment failed and the opened profile could not be closed cleanly.",
      );
    }
    throw attachmentError;
  }
}

export async function connectProfileWithNotifications(
  profileId: string,
  onNotification: (notification: CoreNotificationDto) => void,
): Promise<DesktopProfileConnection> {
  try {
    const notifications = await attachNotifications(onNotification);
    const snapshot = await getCurrentSnapshot();
    return {
      connectionKind: "reattached",
      profile: null,
      snapshot,
      notifications,
    };
  } catch (error: unknown) {
    if (!hasDesktopErrorCode(error, "desktop.profile.not_ready")) {
      throw error;
    }
  }

  const session = await openProfileWithNotifications(profileId, onNotification);
  return {
    connectionKind: "opened",
    profile: session.profile,
    snapshot: session.profile.snapshot,
    notifications: session.notifications,
  };
}

export async function closeProfile(): Promise<BridgeLifecycleDto> {
  return invoke<BridgeLifecycleDto>("close_profile");
}

function hasDesktopErrorCode(error: unknown, expectedCode: string): error is DesktopErrorDto {
  if (typeof error !== "object" || error === null || !("code" in error)) {
    return false;
  }
  return (error as { code: unknown }).code === expectedCode;
}
