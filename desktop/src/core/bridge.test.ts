import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DesktopProfileConnection } from "./client";
import type { CoreNotificationDto, CoreSnapshotDto } from "./generated/desktop-bindings";

const { connectMock, getCurrentSnapshotMock } = vi.hoisted(() => ({
  connectMock: vi.fn(),
  getCurrentSnapshotMock: vi.fn(),
}));

vi.mock("./client", () => ({
  connectProfileWithNotifications: connectMock,
  getCurrentSnapshot: getCurrentSnapshotMock,
}));

const snapshot: CoreSnapshotDto = {
  revision: "4",
  selectedRole: null,
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

const connection = {
  connectionKind: "reattached",
  profile: null,
  snapshot,
  notifications: {
    subscriptionId: "11",
    channel: {},
  },
} as unknown as DesktopProfileConnection;

const diagnostic: CoreNotificationDto = {
  kind: "diagnostic",
  details: { name: "bridge-ready", fields: [] },
};

describe("desktop bridge connection", () => {
  beforeEach(() => {
    connectMock.mockReset();
    getCurrentSnapshotMock.mockReset();
    vi.resetModules();
  });

  it("shares one native connection and one refresh across concurrent subscribers", async () => {
    const refreshed = { ...snapshot, revision: "5" };
    connectMock.mockResolvedValue(connection);
    getCurrentSnapshotMock.mockResolvedValue(refreshed);
    const { ensureDesktopBridge } = await import("./bridge");

    const first = ensureDesktopBridge("main");
    const second = ensureDesktopBridge("main");

    await expect(first).resolves.toEqual({ ...connection, snapshot: refreshed });
    await expect(second).resolves.toEqual({ ...connection, snapshot: refreshed });
    expect(connectMock).toHaveBeenCalledTimes(1);
    expect(getCurrentSnapshotMock).toHaveBeenCalledTimes(1);
  });

  it("delivers notifications only to currently mounted subscribers", async () => {
    let dispatch: ((notification: CoreNotificationDto) => void) | undefined;
    connectMock.mockImplementation(
      (_profileId: string, onNotification: (notification: CoreNotificationDto) => void) => {
        dispatch = onNotification;
        return Promise.resolve(connection);
      },
    );
    getCurrentSnapshotMock.mockResolvedValue(snapshot);
    const { ensureDesktopBridge, subscribeDesktopNotifications } = await import("./bridge");
    const first = vi.fn();
    const second = vi.fn();

    const unsubscribeFirst = subscribeDesktopNotifications(first);
    await ensureDesktopBridge("main");
    unsubscribeFirst();
    subscribeDesktopNotifications(second);
    dispatch?.(diagnostic);

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith(diagnostic);
  });

  it("refreshes the authoritative snapshot for a later frontend reconnect", async () => {
    connectMock.mockResolvedValue(connection);
    getCurrentSnapshotMock
      .mockResolvedValueOnce({ ...snapshot, revision: "5" })
      .mockResolvedValueOnce({ ...snapshot, revision: "9" });
    const { ensureDesktopBridge } = await import("./bridge");

    await expect(ensureDesktopBridge("main")).resolves.toMatchObject({
      snapshot: { revision: "5" },
    });
    await expect(ensureDesktopBridge("main")).resolves.toMatchObject({
      snapshot: { revision: "9" },
    });

    expect(connectMock).toHaveBeenCalledTimes(1);
    expect(getCurrentSnapshotMock).toHaveBeenCalledTimes(2);
  });

  it("allows an explicit retry after connection failure", async () => {
    const failure = new Error("bridge unavailable");
    connectMock.mockRejectedValueOnce(failure).mockResolvedValueOnce(connection);
    getCurrentSnapshotMock.mockResolvedValue(snapshot);
    const { ensureDesktopBridge } = await import("./bridge");

    await expect(ensureDesktopBridge("main")).rejects.toBe(failure);
    await expect(ensureDesktopBridge("main")).resolves.toMatchObject({ snapshot });
    expect(connectMock).toHaveBeenCalledTimes(2);
    expect(getCurrentSnapshotMock).toHaveBeenCalledTimes(1);
  });

  it("retries a failed snapshot refresh without reopening the native connection", async () => {
    const failure = new Error("snapshot unavailable");
    connectMock.mockResolvedValue(connection);
    getCurrentSnapshotMock.mockRejectedValueOnce(failure).mockResolvedValueOnce(snapshot);
    const { ensureDesktopBridge } = await import("./bridge");

    await expect(ensureDesktopBridge("main")).rejects.toBe(failure);
    await expect(ensureDesktopBridge("main")).resolves.toMatchObject({ snapshot });
    expect(connectMock).toHaveBeenCalledTimes(1);
    expect(getCurrentSnapshotMock).toHaveBeenCalledTimes(2);
  });
});
