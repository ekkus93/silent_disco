import type { CoreNotificationDto } from "./generated/desktop-bindings";
import {
  connectProfileWithNotifications,
  type DesktopProfileConnection,
} from "./client";

type DesktopNotificationListener = (notification: CoreNotificationDto) => void;

const listeners = new Set<DesktopNotificationListener>();
let connectionPromise: Promise<DesktopProfileConnection> | null = null;

function dispatchNotification(notification: CoreNotificationDto): void {
  for (const listener of listeners) {
    listener(notification);
  }
}

export function subscribeDesktopNotifications(
  listener: DesktopNotificationListener,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function ensureDesktopBridge(
  profileId = "main",
): Promise<DesktopProfileConnection> {
  if (connectionPromise === null) {
    connectionPromise = connectProfileWithNotifications(profileId, dispatchNotification).catch(
      (error: unknown) => {
        connectionPromise = null;
        throw error;
      },
    );
  }
  return connectionPromise;
}
