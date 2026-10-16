// Shared harness for App-level host workflow tests. The caller's test file
// must mock "../core/bridge" and "../core/client" (see the workflow tests);
// this module only drives the mocked bridge so authoritative snapshots can be
// published exactly as the Tauri notification channel would.
import { act, render } from "@testing-library/react";
import { Provider } from "react-redux";
import type { Mock } from "vitest";

import { App } from "../App";
import { createAppStore } from "../app/store";
import type {
  CoreNotificationDto,
  CoreSnapshotDto,
  HostSessionSnapshotDto,
} from "../core/generated/desktop-bindings";

export interface HostAppHarness {
  /** Publishes a newer authoritative core snapshot over the notification channel. */
  publishCore: (snapshot: CoreSnapshotDto) => void;
  /** Replaces what the next host-session poll or refresh returns. */
  setHostSession: (snapshot: HostSessionSnapshotDto) => void;
  container: HTMLElement;
}

export function renderHostApp(options: {
  initial: CoreSnapshotDto;
  hostSession: HostSessionSnapshotDto;
  ensureDesktopBridge: Mock;
  subscribeDesktopNotifications: Mock;
  getHostSessionState: Mock;
}): HostAppHarness {
  let listener: ((notification: CoreNotificationDto) => void) | undefined;
  let session = options.hostSession;
  options.subscribeDesktopNotifications.mockImplementation(
    (next: (notification: CoreNotificationDto) => void) => {
      listener = next;
      return () => {};
    },
  );
  options.ensureDesktopBridge.mockResolvedValue({
    connectionKind: "opened",
    profile: null,
    snapshot: options.initial,
    notifications: { subscriptionId: "1", channel: {} },
  });
  options.getHostSessionState.mockImplementation(() => Promise.resolve(session));
  const store = createAppStore();
  const view = render(
    <Provider store={store}>
      <App />
    </Provider>,
  );
  return {
    publishCore: (snapshot) => {
      act(() => {
        listener?.({ kind: "snapshot", details: snapshot });
      });
    },
    setHostSession: (snapshot) => {
      session = snapshot;
    },
    container: view.container,
  };
}
