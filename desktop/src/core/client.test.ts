import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreNotificationDto, OpenProfileResponse } from "./generated/desktop-bindings";
import { attachNotifications, openProfileWithNotifications } from "./client";

const { channelInstances, invokeMock } = vi.hoisted(() => ({
  channelInstances: [] as Array<{ onmessage: (message: unknown) => void }>,
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class<T> {
    onmessage: (message: T) => void = () => undefined;

    constructor() {
      channelInstances.push(this as { onmessage: (message: unknown) => void });
    }
  },
  invoke: invokeMock,
}));

const openResponse: OpenProfileResponse = {
  lifecycle: { kind: "ready", details: { profile_id: "main" } },
  coreVersion: { major: 0, minor: 1, patch: 0 },
  snapshot: {
    revision: "0",
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
};

const diagnostic: CoreNotificationDto = {
  kind: "diagnostic",
  details: { name: "desktop-ready", fields: [] },
};

describe("desktop core client", () => {
  beforeEach(() => {
    channelInstances.length = 0;
    invokeMock.mockReset();
  });

  it("attaches one typed channel and keeps the callback live", async () => {
    invokeMock.mockResolvedValue({ subscriptionId: "7" });
    const onNotification = vi.fn();

    const subscription = await attachNotifications(onNotification);

    expect(subscription.subscriptionId).toBe("7");
    expect(invokeMock).toHaveBeenCalledWith("attach_notifications", {
      channel: subscription.channel,
    });
    expect(channelInstances).toHaveLength(1);
    channelInstances[0]?.onmessage(diagnostic);
    expect(onNotification).toHaveBeenCalledWith(diagnostic);
  });

  it("opens the profile before attaching notifications", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      if (command === "attach_notifications") {
        return Promise.resolve({ subscriptionId: "8" });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const session = await openProfileWithNotifications("main", vi.fn());

    expect(session.profile).toEqual(openResponse);
    expect(session.notifications.subscriptionId).toBe("8");
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "open_profile",
      "attach_notifications",
    ]);
  });

  it("closes an opened profile when notification attachment fails", async () => {
    const attachmentError = new Error("channel unavailable");
    invokeMock.mockImplementation((command: string) => {
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      if (command === "attach_notifications") {
        return Promise.reject(attachmentError);
      }
      if (command === "close_profile") {
        return Promise.resolve({ kind: "closed" });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    await expect(openProfileWithNotifications("main", vi.fn())).rejects.toBe(attachmentError);
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "open_profile",
      "attach_notifications",
      "close_profile",
    ]);
  });

  it("preserves attachment and cleanup failures together", async () => {
    const attachmentError = new Error("channel unavailable");
    const closeError = new Error("profile close failed");
    invokeMock.mockImplementation((command: string) => {
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      if (command === "attach_notifications") {
        return Promise.reject(attachmentError);
      }
      if (command === "close_profile") {
        return Promise.reject(closeError);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const failure = await openProfileWithNotifications("main", vi.fn()).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(AggregateError);
    expect((failure as AggregateError).errors).toEqual([attachmentError, closeError]);
  });
});
