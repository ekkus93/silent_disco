import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreNotificationDto, OpenProfileResponse } from "./generated/desktop-bindings";
import {
  attachNotifications,
  connectProfileWithNotifications,
  openProfileWithNotifications,
} from "./client";

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

  it("reattaches before obtaining the authoritative current snapshot", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "attach_notifications") {
        return Promise.resolve({ subscriptionId: "9" });
      }
      if (command === "get_current_snapshot") {
        return Promise.resolve({ ...openResponse.snapshot, revision: "12" });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const connection = await connectProfileWithNotifications("main", vi.fn());

    expect(connection.connectionKind).toBe("reattached");
    expect(connection.profile).toBeNull();
    expect(connection.snapshot.revision).toBe("12");
    expect(connection.notifications.subscriptionId).toBe("9");
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "attach_notifications",
      "get_current_snapshot",
    ]);
  });

  it("opens and attaches when no profile is ready", async () => {
    let attachmentCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "attach_notifications") {
        attachmentCount += 1;
        if (attachmentCount === 1) {
          return Promise.reject({
            code: "desktop.profile.not_ready",
            subsystem: "runtime",
            severity: "error",
            retryable: true,
            message: "No desktop profile is ready.",
          });
        }
        return Promise.resolve({ subscriptionId: "10" });
      }
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const connection = await connectProfileWithNotifications("main", vi.fn());

    expect(connection.connectionKind).toBe("opened");
    expect(connection.profile).toEqual(openResponse);
    expect(connection.snapshot).toEqual(openResponse.snapshot);
    expect(connection.notifications.subscriptionId).toBe("10");
    expect(channelInstances).toHaveLength(2);
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "attach_notifications",
      "open_profile",
      "attach_notifications",
    ]);
  });

  it(
    "converts unexpected invocation transport failure into a structured bridge error",
    async () => {
      invokeMock.mockRejectedValue(new Error("channel permission denied"));

      await expect(connectProfileWithNotifications("main", vi.fn())).rejects.toMatchObject({
        code: "desktop.bridge.invoke_transport_failed",
        subsystem: "bridge",
        retryable: true,
        message: expect.stringContaining("channel permission denied"),
      });
      expect(invokeMock).toHaveBeenCalledTimes(1);
      expect(invokeMock.mock.calls[0]?.[0]).toBe("attach_notifications");
    },
  );

  it("does not retry a failed non-idempotent profile open", async () => {
    let attachmentCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "attach_notifications") {
        attachmentCount += 1;
        return Promise.reject({
          code: "desktop.profile.not_ready",
          subsystem: "runtime",
          severity: "error",
          retryable: true,
          message: "No desktop profile is ready.",
        });
      }
      if (command === "open_profile") {
        return Promise.reject(new Error("credential store unavailable"));
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    await expect(connectProfileWithNotifications("main", vi.fn())).rejects.toMatchObject({
      code: "desktop.bridge.invoke_transport_failed",
    });
    expect(attachmentCount).toBe(1);
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "open_profile"),
    ).toHaveLength(1);
  });

  it("closes an opened profile when notification attachment fails", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      if (command === "attach_notifications") {
        return Promise.reject(new Error("channel unavailable"));
      }
      if (command === "close_profile") {
        return Promise.resolve({ kind: "closed" });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    await expect(openProfileWithNotifications("main", vi.fn())).rejects.toMatchObject({
      code: "desktop.bridge.invoke_transport_failed",
      message: expect.stringContaining("channel unavailable"),
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "open_profile",
      "attach_notifications",
      "close_profile",
    ]);
  });

  it("preserves both failures in a bounded, single-line cleanup error", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "open_profile") {
        return Promise.resolve(openResponse);
      }
      if (command === "attach_notifications") {
        return Promise.reject(new Error(`channel unavailable\n${"a".repeat(900)}`));
      }
      if (command === "close_profile") {
        return Promise.reject(new Error(`profile close failed\u0000${"b".repeat(900)}`));
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const failure = await openProfileWithNotifications("main", vi.fn()).catch(
      (error: unknown) => error,
    );

    expect(failure).toMatchObject({
      code: "desktop.bridge.attach_cleanup_failed",
      severity: "fatal",
      retryable: false,
      message: expect.stringContaining("profile cleanup also failed"),
    });
    const message = (failure as { message: string }).message;
    expect(message.length).toBeLessThanOrEqual(512);
    expect(message).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/u);
    expect(message).toContain("channel unavailable");
    expect(message).toContain("profile close failed");
  });
});
