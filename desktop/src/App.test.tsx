import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreNotificationDto } from "./core/generated/desktop-bindings";
import { App } from "./App";

const {
  ensureDesktopBridgeMock,
  getCoreSmokeMock,
  subscribeDesktopNotificationsMock,
  unsubscribeMock,
} = vi.hoisted(() => ({
  ensureDesktopBridgeMock: vi.fn(),
  getCoreSmokeMock: vi.fn(),
  subscribeDesktopNotificationsMock: vi.fn(),
  unsubscribeMock: vi.fn(),
}));

let notificationListener: ((notification: CoreNotificationDto) => void) | undefined;

vi.mock("./core/client", () => ({
  getCoreSmoke: getCoreSmokeMock,
}));

vi.mock("./core/bridge", () => ({
  ensureDesktopBridge: ensureDesktopBridgeMock,
  subscribeDesktopNotifications: subscribeDesktopNotificationsMock,
}));

const connection = {
  connectionKind: "opened",
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
    subscriptionId: "17",
    channel: {},
  },
};

describe("App", () => {
  beforeEach(() => {
    notificationListener = undefined;
    ensureDesktopBridgeMock.mockReset();
    getCoreSmokeMock.mockReset();
    subscribeDesktopNotificationsMock.mockReset();
    unsubscribeMock.mockReset();
    subscribeDesktopNotificationsMock.mockImplementation(
      (listener: (notification: CoreNotificationDto) => void) => {
        notificationListener = listener;
        return unsubscribeMock;
      },
    );
  });

  it("opens the authoritative profile bridge and renders its real status", async () => {
    getCoreSmokeMock.mockResolvedValue({
      major: 0,
      minor: 1,
      patch: 0,
      smoke: "6000001225524396033",
    });
    ensureDesktopBridgeMock.mockResolvedValue(connection);

    render(<App />);

    expect(screen.getByRole("status")).toHaveTextContent("Opening or reattaching the main profile");
    expect(await screen.findByText("0.1.0")).toBeVisible();
    expect(screen.getByText("Opened the main profile")).toBeVisible();
    expect(screen.getByText("17")).toBeVisible();
    expect(screen.getByText("4")).toBeVisible();
    expect(screen.getByText("6000001225524396033")).toBeVisible();
    expect(ensureDesktopBridgeMock).toHaveBeenCalledWith("main");
  });

  it("updates only the displayed revision when a newer snapshot arrives", async () => {
    getCoreSmokeMock.mockResolvedValue({ major: 0, minor: 1, patch: 0, smoke: "42" });
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    render(<App />);
    await screen.findByText("Opened the main profile");

    act(() => {
      notificationListener?.({
        kind: "snapshot",
        details: { ...connection.snapshot, revision: "5" },
      });
    });

    expect(screen.getByText("5")).toBeVisible();
  });

  it("keeps bridge startup failure visible", async () => {
    getCoreSmokeMock.mockResolvedValue({ major: 0, minor: 1, patch: 0, smoke: "42" });
    ensureDesktopBridgeMock.mockRejectedValue(new Error("native bridge unavailable"));

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Desktop bridge startup failed");
    expect(screen.getByRole("alert")).toHaveTextContent("native bridge unavailable");
  });

  it("unsubscribes the React listener without closing the native channel", () => {
    getCoreSmokeMock.mockResolvedValue({ major: 0, minor: 1, patch: 0, smoke: "42" });
    ensureDesktopBridgeMock.mockResolvedValue(connection);

    const view = render(<App />);
    view.unmount();

    expect(unsubscribeMock).toHaveBeenCalledOnce();
  });
});
