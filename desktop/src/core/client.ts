import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  AttachNotificationResponse,
  BridgeLifecycleDto,
  CoreNotificationDto,
  CoreSnapshotDto,
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

export async function closeProfile(): Promise<BridgeLifecycleDto> {
  return invoke<BridgeLifecycleDto>("close_profile");
}
