import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as client from "../core/client";
import type { DesktopErrorDto, HostSessionSnapshotDto } from "../core/generated/desktop-bindings";
import { HostSessionScreen } from "./HostSessionScreen";

vi.mock("../core/client");

const invokeError: DesktopErrorDto = {
  code: "core.invalid_state_transition",
  subsystem: "validation",
  severity: "error",
  retryable: false,
  message: "join request is stale or no longer pending",
};

function fixture(overrides: Partial<HostSessionSnapshotDto> = {}): HostSessionSnapshotDto {
  return {
    revision: "12",
    hostLifecycle: "waiting_for_listeners",
    transportState: "advertising",
    playbackState: "stopped",
    sessionName: "Manual Session",
    connection: {
      hostAddress: "192.168.1.50",
      controlPort: 4100,
      syncPort: 4101,
      audioPort: 4102,
      sessionId: "session-1",
      protocolVersion: 2,
      inviteCodeRequired: false,
      expiresAtMs: null,
    },
    pendingJoinRequests: [
      {
        requestId: "request-1",
        deviceId: "listener-1",
        displayName: "Listener One",
        trustState: "session_only",
        inviteCodeValid: true,
        receivedAtMs: "100",
        ageMs: "2500",
      },
    ],
    connectedListeners: [
      {
        deviceId: "listener-2",
        displayName: "Listener Two",
        trustState: "trusted",
        transportState: "connected",
        lastContactMs: "500",
        lastContactAgeMs: "250",
        syncConfidence: "good",
        estimatedOffsetMs: "-2.5",
        roundTripTimeMs: "18",
        driftPpm: "1.2",
        lastControlDeliveryState: "ok",
        retryAvailable: false,
        resyncAvailable: false,
        canRemove: true,
        lastError: null,
      },
    ],
    lastDelivery: {
      intendedPeers: 1,
      successfulPeers: 1,
      failedPeers: 0,
      severity: "ok",
    },
    recoverableAction: null,
    playbackControlsEnabled: false,
    transportWorkerRunning: true,
    transportError: null,
    broadcast: null,
    lastError: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(client.getHostSessionState).mockResolvedValue(fixture());
  vi.mocked(client.approveJoinRequest).mockResolvedValue({
    operationId: "command-1",
    acceptedAtRevision: "12",
  });
  vi.mocked(client.rejectJoinRequest).mockResolvedValue({
    operationId: "command-2",
    acceptedAtRevision: "12",
  });
  vi.mocked(client.removeListener).mockResolvedValue({
    operationId: "command-3",
    acceptedAtRevision: "12",
  });
  vi.mocked(client.endHostSession).mockResolvedValue({
    operationId: "command-4",
    acceptedAtRevision: "12",
  });
  vi.mocked(client.startHostPlayback).mockResolvedValue(undefined);
  vi.mocked(client.pauseHostPlayback).mockResolvedValue(undefined);
  vi.mocked(client.resumeHostPlayback).mockResolvedValue(undefined);
  vi.mocked(client.stopHostPlayback).mockResolvedValue(undefined);
});

describe("HostSessionScreen Block 23", () => {
  it("renders safe request identity, age, trust, invite and accessible decisions", async () => {
    render(<HostSessionScreen />);
    expect(await screen.findByText("Listener One")).toBeInTheDocument();
    expect(screen.getByText("Request request-1")).toBeInTheDocument();
    expect(screen.getByText("Age: 3 s")).toBeInTheDocument();
    expect(screen.getByText("Trust: session_only")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Reject" })).toBeEnabled();
    expect(screen.getByRole("checkbox", { name: "Remember this device" })).toBeEnabled();
  });

  it("does not remove a request optimistically and blocks duplicate approval", async () => {
    let resolveApproval:
      | ((value: { operationId: string; acceptedAtRevision: string }) => void)
      | undefined;
    vi.mocked(client.approveJoinRequest).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveApproval = resolve;
        }),
    );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    expect(client.approveJoinRequest).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Listener One")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();

    resolveApproval?.({
      operationId: "command-approval",
      acceptedAtRevision: "12",
    });
    expect(
      await screen.findByText("Waiting for approval delivery confirmation…"),
    ).toBeInTheDocument();
    expect(screen.getByText("Listener One")).toBeInTheDocument();
  });

  it("removes a request only after delivery-confirmed authoritative state", async () => {
    vi.mocked(client.getHostSessionState)
      .mockResolvedValueOnce(fixture())
      .mockResolvedValueOnce(
        fixture({
          revision: "14",
          pendingJoinRequests: [],
          connectedListeners: [
            ...fixture().connectedListeners,
            {
              deviceId: "listener-1",
              displayName: "Listener One",
              trustState: "session_only",
              transportState: "connected",
              lastContactMs: "500",
              lastContactAgeMs: "250",
              syncConfidence: "good",
              estimatedOffsetMs: "-2.5",
              roundTripTimeMs: "18",
              driftPpm: "1.2",
              lastControlDeliveryState: "ok",
              retryAvailable: false,
              resyncAvailable: false,
              canRemove: true,
              lastError: null,
            },
          ],
        }),
      );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(client.approveJoinRequest).toHaveBeenCalled());
    expect(screen.getByText("Listener One")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Refresh host state" }));
    await waitFor(() =>
      expect(
        screen.getByText("Listener approval was confirmed by the Rust core."),
      ).toBeInTheDocument(),
    );
    expect(screen.queryByText("Request request-1")).not.toBeInTheDocument();
  });

  it("keeps a zero-recipient approval actionable and visible", async () => {
    vi.mocked(client.getHostSessionState)
      .mockResolvedValueOnce(fixture())
      .mockResolvedValueOnce(
        fixture({
          revision: "13",
          lastDelivery: {
            intendedPeers: 0,
            successfulPeers: 0,
            failedPeers: 0,
            severity: "zero_peers",
          },
          lastError: {
            code: "core.transport_delivery_failed",
            subsystem: "transport",
            severity: "error",
            retryable: true,
            message: "transport control delivery had no intended recipients",
          },
        }),
      );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(client.approveJoinRequest).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Refresh host state" }));

    expect(
      await screen.findByText("transport control delivery had no intended recipients"),
    ).toBeInTheDocument();
    expect(screen.getByText("Request request-1")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("commits partial delivery but keeps the warning visible", async () => {
    vi.mocked(client.getHostSessionState)
      .mockResolvedValueOnce(fixture())
      .mockResolvedValueOnce(
        fixture({
          revision: "14",
          pendingJoinRequests: [],
          lastDelivery: {
            intendedPeers: 2,
            successfulPeers: 1,
            failedPeers: 1,
            severity: "partial_failure",
          },
        }),
      );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(client.approveJoinRequest).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: "Refresh host state" }));

    expect(await screen.findByText(/Last delivery: 1 of 2 succeeded/)).toBeInTheDocument();
    expect(screen.queryByText("Request request-1")).not.toBeInTheDocument();
  });

  it("keeps rejection delivery failure and stale request failure visible", async () => {
    vi.mocked(client.rejectJoinRequest).mockRejectedValueOnce(invokeError);
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));

    expect(
      await screen.findByText("join request is stale or no longer pending"),
    ).toBeInTheDocument();
    expect(screen.getByText("Request request-1")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reject" })).toBeEnabled();
  });

  it("passes the explicit trusted-device policy to Rust", async () => {
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("checkbox", { name: "Remember this device" }));
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    await waitFor(() =>
      expect(client.approveJoinRequest).toHaveBeenCalledWith({
        expectedRevision: "12",
        requestId: "request-1",
        rememberForFuture: true,
      }),
    );
  });

  it("shows listener detail metrics and waits for removal confirmation", async () => {
    render(<HostSessionScreen />);
    await screen.findByText("Listener Two");
    fireEvent.click(screen.getByRole("button", { name: "View Listener Two" }));
    expect(screen.getByRole("heading", { name: "Listener Two" })).toBeInTheDocument();
    expect(screen.getByText("good")).toBeInTheDocument();
    expect(screen.getByText("-2.5 ms")).toBeInTheDocument();
    expect(screen.getByText("18 ms")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Remove listener" }));
    expect(client.removeListener).toHaveBeenCalledWith({
      expectedRevision: "12",
      listenerId: "listener-2",
    });
    expect(
      await screen.findByRole("button", {
        name: "Waiting for disconnect delivery…",
      }),
    ).toBeDisabled();
    expect(screen.getByRole("heading", { name: "Listener Two" })).toBeInTheDocument();
  });

  it("keeps command failures visible across successful refreshes", async () => {
    vi.mocked(client.rejectJoinRequest).mockRejectedValueOnce(invokeError);
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    await screen.findByText("join request is stale or no longer pending");

    fireEvent.click(screen.getByRole("button", { name: "Refresh host state" }));
    await waitFor(() => expect(client.getHostSessionState).toHaveBeenCalledTimes(2));
    expect(screen.getByText("join request is stale or no longer pending")).toBeInTheDocument();
  });
});

describe("HostSessionScreen playback controls", () => {
  it("disables all playback buttons when the core reports controls disabled", async () => {
    vi.mocked(client.getHostSessionState).mockResolvedValue(
      fixture({ playbackControlsEnabled: false }),
    );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    expect(screen.getByRole("button", { name: "Play" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  });

  it("starts playback from a stopped, control-enabled session", async () => {
    vi.mocked(client.getHostSessionState).mockResolvedValue(
      fixture({ playbackState: "stopped", playbackControlsEnabled: true }),
    );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    expect(screen.getByRole("button", { name: "Pause" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(client.startHostPlayback).toHaveBeenCalled());
  });

  it("shows Resume instead of Play once paused, and pauses only while playing", async () => {
    vi.mocked(client.getHostSessionState).mockResolvedValue(
      fixture({ playbackState: "playing", playbackControlsEnabled: true }),
    );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    expect(screen.getByRole("button", { name: "Play" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(client.pauseHostPlayback).toHaveBeenCalled());

    vi.mocked(client.getHostSessionState).mockResolvedValue(
      fixture({ playbackState: "paused", playbackControlsEnabled: true }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh host state" }));
    const resumeButton = await screen.findByRole("button", { name: "Resume" });
    expect(resumeButton).toBeEnabled();
    fireEvent.click(resumeButton);
    await waitFor(() => expect(client.resumeHostPlayback).toHaveBeenCalled());
  });

  it("stops playback while playing and surfaces a failed stop", async () => {
    vi.mocked(client.stopHostPlayback).mockRejectedValueOnce(invokeError);
    vi.mocked(client.getHostSessionState).mockResolvedValue(
      fixture({ playbackState: "playing", playbackControlsEnabled: true }),
    );
    render(<HostSessionScreen />);
    await screen.findByText("Listener One");
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(
      await screen.findByText("join request is stale or no longer pending"),
    ).toBeInTheDocument();
  });
});
