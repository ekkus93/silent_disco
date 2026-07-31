import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { HostSessionSnapshotDto } from "../core/generated/desktop-bindings";
import { HostSessionScreen } from "./HostSessionScreen";

const { getHostSessionState, endHostSession, writeText } = vi.hoisted(() => ({
  getHostSessionState: vi.fn(),
  endHostSession: vi.fn(),
  writeText: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return {
    ...actual,
    getHostSessionState,
    endHostSession,
  };
});

function hostSession(overrides: Partial<HostSessionSnapshotDto> = {}): HostSessionSnapshotDto {
  return {
    revision: "12",
    hostLifecycle: "waiting_for_listeners",
    transportState: "connected",
    playbackState: "stopped",
    sessionName: "Oakland Night",
    connection: {
      hostAddress: "192.168.1.20",
      controlPort: 47000,
      syncPort: 47001,
      audioPort: 47002,
      sessionId: "session-block22",
      protocolVersion: 2,
      inviteCodeRequired: false,
      expiresAtMs: null,
    },
    pendingJoinRequests: [
      {
        requestId: "request-1",
        deviceId: "listener-pending",
        displayName: "Pending phone",
        trustState: "session_only",
        inviteCodeValid: true,
        receivedAtMs: "100",
      },
    ],
    connectedListeners: [
      {
        deviceId: "listener-connected",
        displayName: "Connected phone",
        trustState: "trusted",
        transportState: "connected",
        lastContactMs: "120",
        estimatedOffsetMs: "2",
        roundTripTimeMs: "8",
        lastError: null,
      },
    ],
    playbackControlsEnabled: false,
    transportWorkerRunning: true,
    transportError: null,
    lastError: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getHostSessionState.mockResolvedValue(hostSession());
  endHostSession.mockResolvedValue({ operationId: "end-1", acceptedAtRevision: "12" });
  writeText.mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
});

describe("HostSessionScreen", () => {
  it("renders authoritative connection, listener, and disabled playback state", async () => {
    render(<HostSessionScreen />);

    expect(await screen.findByRole("heading", { name: "Host session" })).toBeVisible();
    expect(screen.getByText("192.168.1.20")).toBeVisible();
    expect(screen.getByText("47000")).toBeVisible();
    expect(screen.getByText("47001")).toBeVisible();
    expect(screen.getByText("47002")).toBeVisible();
    expect(screen.getByText("session-block22")).toBeVisible();
    expect(screen.getByText("Pending phone")).toBeVisible();
    expect(screen.getByText("Connected phone")).toBeVisible();
    expect(screen.getByRole("button", { name: "Play" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  });

  it("copies the host address and bounded connection details", async () => {
    render(<HostSessionScreen />);
    await screen.findByText("session-block22");

    fireEvent.click(screen.getByRole("button", { name: "Copy host address" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("192.168.1.20"));
    expect(screen.getByRole("status")).toHaveTextContent("Host address copied");

    fireEvent.click(screen.getByRole("button", { name: "Copy connection details" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(2));
    expect(writeText.mock.calls[1]?.[0]).toContain('"controlPort": 47000');
    expect(writeText.mock.calls[1]?.[0]).toContain('"protocolVersion": 2');
  });

  it("submits the end command once with the authoritative revision", async () => {
    render(<HostSessionScreen />);
    const end = await screen.findByRole("button", { name: "End session" });

    fireEvent.click(end);
    fireEvent.click(end);
    await waitFor(() => expect(endHostSession).toHaveBeenCalledWith("12"));
    expect(endHostSession).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Ending session…" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for a newer Rust lifecycle");
  });

  it("keeps transport and core failures visible", async () => {
    getHostSessionState.mockResolvedValue(
      hostSession({
        transportError: "control worker stopped",
        lastError: {
          code: "core.transport.failed",
          subsystem: "transport",
          severity: "error",
          retryable: true,
          message: "listener delivery failed",
        },
      }),
    );
    render(<HostSessionScreen />);

    expect(await screen.findByText("control worker stopped")).toBeVisible();
    expect(screen.getByText("listener delivery failed")).toBeVisible();
  });

  it("shows an end-session rejection and re-enables the action", async () => {
    endHostSession.mockRejectedValue(new Error("stale revision"));
    render(<HostSessionScreen />);
    const end = await screen.findByRole("button", { name: "End session" });

    fireEvent.click(end);
    expect(await screen.findByRole("alert")).toHaveTextContent("stale revision");
    expect(screen.getByRole("button", { name: "End session" })).toBeEnabled();
  });

  it("does not claim connection success before an endpoint exists", async () => {
    getHostSessionState.mockResolvedValue(
      hostSession({ connection: null, transportWorkerRunning: false }),
    );
    render(<HostSessionScreen />);

    expect(
      await screen.findByText(
        /Waiting for the shared transport to report a successfully bound endpoint/,
      ),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Copy connection details" }),
    ).not.toBeInTheDocument();
  });
});
