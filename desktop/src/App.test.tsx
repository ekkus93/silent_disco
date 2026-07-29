import { act, render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { createAppStore } from "./app/store";
import type { CoreNotificationDto } from "./core/generated/desktop-bindings";

const { ensureDesktopBridgeMock, subscribeDesktopNotificationsMock, unsubscribeMock } = vi.hoisted(
  () => ({
    ensureDesktopBridgeMock: vi.fn(),
    subscribeDesktopNotificationsMock: vi.fn(),
    unsubscribeMock: vi.fn(),
  }),
);

let notificationListener: ((notification: CoreNotificationDto) => void) | undefined;

vi.mock("./core/bridge", () => ({
  ensureDesktopBridge: ensureDesktopBridgeMock,
  subscribeDesktopNotifications: subscribeDesktopNotificationsMock,
}));

const connection = {
  connectionKind: "opened" as const,
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

function renderApp() {
  const store = createAppStore();
  return {
    store,
    view: render(
      <Provider store={store}>
        <App />
      </Provider>,
    ),
  };
}

describe("App", () => {
  beforeEach(() => {
    notificationListener = undefined;
    ensureDesktopBridgeMock.mockReset();
    subscribeDesktopNotificationsMock.mockReset();
    unsubscribeMock.mockReset();
    subscribeDesktopNotificationsMock.mockImplementation(
      (listener: (notification: CoreNotificationDto) => void) => {
        notificationListener = listener;
        return unsubscribeMock;
      },
    );
  });

  it("opens the authoritative profile bridge and renders the Redux snapshot", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);

    const { store } = renderApp();

    expect(screen.getByRole("status")).toHaveTextContent("Opening or reattaching the main profile");
    expect(await screen.findByText("Opened the main profile")).toBeVisible();
    expect(screen.getByText("17")).toBeVisible();
    expect(screen.getByText("4")).toBeVisible();
    expect(store.getState().core.snapshot).toEqual(connection.snapshot);
    expect(ensureDesktopBridgeMock).toHaveBeenCalledWith("main");
  });

  it("replaces the displayed complete snapshot only for a newer revision", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    const { store } = renderApp();
    await screen.findByText("Opened the main profile");

    act(() => {
      notificationListener?.({
        kind: "snapshot",
        details: {
          ...connection.snapshot,
          revision: "5",
          hostLifecycle: "waiting_for_listeners",
        },
      });
    });

    expect(screen.getByText("5")).toBeVisible();
    expect(screen.getByText("waiting_for_listeners")).toBeVisible();
    expect(store.getState().core.snapshot?.revision).toBe("5");
  });

  it("displays a command failure delivered by the authoritative notification channel", async () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);
    renderApp();
    await screen.findByText("Opened the main profile");

    act(() => {
      notificationListener?.({
        kind: "error",
        details: {
          code: "core.command.rejected",
          subsystem: "runtime",
          severity: "error",
          retryable: false,
          message: "The command was rejected.",
        },
      });
    });

    expect(screen.getByRole("alert")).toHaveTextContent("Core command or bridge error");
    expect(screen.getByRole("alert")).toHaveTextContent("The command was rejected.");
  });

  it("keeps bridge startup failure visible as a structured Redux error", async () => {
    ensureDesktopBridgeMock.mockRejectedValue(new Error("native bridge unavailable"));

    const { store } = renderApp();

    expect(await screen.findByRole("alert")).toHaveTextContent("Desktop bridge startup failed");
    expect(screen.getByRole("alert")).toHaveTextContent("native bridge unavailable");
    expect(store.getState().core.bridgeLifecycle.kind).toBe("failed");
    expect(store.getState().core.errors.at(-1)?.code).toBe(
      "desktop.bridge.invoke_transport_failed",
    );
  });

  it("unsubscribes the React listener without closing the native channel", () => {
    ensureDesktopBridgeMock.mockResolvedValue(connection);

    const { view } = renderApp();
    view.unmount();

    expect(unsubscribeMock).toHaveBeenCalledOnce();
  });
});
