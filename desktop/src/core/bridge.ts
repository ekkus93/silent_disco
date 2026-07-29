import {
  connectProfileWithNotifications,
  getCurrentSnapshot,
  type DesktopProfileConnection,
} from "./client";
import type { CoreNotificationDto } from "./generated/desktop-bindings";

type DesktopNotificationListener = (notification: CoreNotificationDto) => void;

const listeners = new Set<DesktopNotificationListener>();
let connectionPromise: Promise<DesktopProfileConnection> | null = null;
let connectionProfileId: string | null = null;
let snapshotRefreshPromise: Promise<DesktopProfileConnection> | null = null;

function dispatchNotification(notification: CoreNotificationDto): void {
  for (const listener of listeners) {
    listener(notification);
  }
}

function establishDesktopBridge(profileId: string): Promise<DesktopProfileConnection> {
  if (connectionPromise !== null && connectionProfileId !== profileId) {
    return Promise.reject(
      new Error(
        `The desktop bridge is already connected to profile ${connectionProfileId}; it cannot silently reuse that connection for profile ${profileId}.`,
      ),
    );
  }

  if (connectionPromise === null) {
    connectionProfileId = profileId;
    connectionPromise = connectProfileWithNotifications(profileId, dispatchNotification).catch(
      (error: unknown) => {
        connectionPromise = null;
        connectionProfileId = null;
        throw error;
      },
    );
  }
  return connectionPromise;
}

export function subscribeDesktopNotifications(listener: DesktopNotificationListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export async function ensureDesktopBridge(
  profileId = "main",
): Promise<DesktopProfileConnection> {
  const connection = await establishDesktopBridge(profileId);
  if (snapshotRefreshPromise === null) {
    snapshotRefreshPromise = getCurrentSnapshot()
      .then((snapshot) => ({ ...connection, snapshot }))
      .finally(() => {
        snapshotRefreshPromise = null;
      });
  }
  return snapshotRefreshPromise;
}
