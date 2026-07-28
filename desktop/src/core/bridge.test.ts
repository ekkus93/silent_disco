import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DesktopProfileConnection } from "./client";
import type { CoreNotificationDto } from "./generated/desktop-bindings";

const { connectMock } = vi.hoisted(() => ({
  connectMock: vi.fn(),
}));

vi.mock("./client", () => ({
  connectProfileWithNotifications: connectMock,
}));

const connection = {
  connectionKind: "reattached",
  profile: null,
  snapshot: {
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
  },
  notifications: {
    subscriptionId: "11",
    channel: {},
  },
} as DesktopProfileConnection;

const diagnostic: CoreNotificationDto = {
  kind: "diagnostic",
  details: { name: "bridge-ready", fields: [] },
};

describe("desktop bridge connection", () => {
  beforeEach(() => {
    connectMock.mockReset();
    vi.resetModules();
  });

  it("shares one in-flight connection across React subscribers", async () => {
    connectMock.mockResolvedValue(connection);
    const { ensureDesktopBridge } = await import("./bridge");

    const first = ensureDesktopBridge("main");
    const second = ensureDesktopBridge("main");

    await expect(first).resolves.toBe(connection);
    await expect(second).resolves.toBe(connection);
    expect(connectMock).toHaveBeenCalledTimes(1);
  });

  it("delivers notifications only to currently mounted subscribers", async () => {
    let dispatch: ((notification: CoreNotificationDto) => void) | undefined;
    connectMock.mockImplementation(
      (_profileId: string, onNotification: (notification: CoreNotificationDto) => void) => {
        dispatch = onNotification;
        return Promise.resolve(connection);
      },
    );
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

  it("allows an explicit retry after connection failure", async () => {
    const failure = new Error("bridge unavailable");
    connectMock.mockRejectedValueOnce(failure).mockResolvedValueOnce(connection);
    const { ensureDesktopBridge } = await import("./bridge");

    await expect(ensureDesktopBridge("main")).rejects.toBe(failure);
    await expect(ensureDesktopBridge("main")).resolves.toBe(connection);
    expect(connectMock).toHaveBeenCalledTimes(2);
  });
});
