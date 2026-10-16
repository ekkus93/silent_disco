import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Provider } from "react-redux";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createAppStore } from "../../app/store";
import * as client from "../../core/client";
import type { HostSessionSnapshotDto } from "../../core/generated/desktop-bindings";
import {
  hostSession,
  listener,
  networkSnapshot,
  pendingRequest,
} from "../../test/hostWorkflowFixtures";
import { HostSessionScreen } from ".";
import { DELIVERY_CONFIRMATION_TIMEOUT_MS } from "./domain";

vi.mock("../../core/client");

let current: HostSessionSnapshotDto;

function publish(next: HostSessionSnapshotDto) {
  current = next;
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
}

function renderScreen(initial: HostSessionSnapshotDto) {
  current = initial;
  return render(
    <Provider store={createAppStore()}>
      <HostSessionScreen />
    </Provider>,
  );
}

function announcementRegion(): HTMLElement {
  const region = document.querySelector<HTMLElement>(".desktop-host-announcement");
  if (!region) throw new Error("announcement region missing");
  return region;
}

function listenerCard(name: string): HTMLElement {
  const item = screen.getByText(name).closest("li");
  if (!item) throw new Error(`no listener card for ${name}`);
  return item;
}

const JO = pendingRequest("jo", "Jo's Pixel");
const PLAYING = {
  hostLifecycle: "streaming",
  transportState: "connected",
  playbackState: "playing",
  playbackPositionMs: "84000",
} as const;

beforeEach(() => {
  vi.mocked(client.getHostSessionState).mockImplementation(() => Promise.resolve(current));
  vi.mocked(client.getHostNetworkState).mockResolvedValue(networkSnapshot());
  vi.mocked(client.approveJoinRequest).mockResolvedValue({
    operationId: "approve-jo",
    acceptedAtRevision: "20",
  });
  vi.mocked(client.rejectJoinRequest).mockResolvedValue({
    operationId: "reject-jo",
    acceptedAtRevision: "20",
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("host presentation of listener approval outcomes", () => {
  it("announces a late joiner and shows it waiting for sync, then in sync", async () => {
    renderScreen(
      hostSession({
        ...PLAYING,
        pendingJoinRequests: [JO],
        connectedListeners: [listener("sam", "Sam's Phone")],
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(announcementRegion()).toHaveTextContent(
        "Approval queued. Waiting for delivery confirmation.",
      ),
    );
    expect(screen.getByText("Waiting for approval delivery confirmation…")).toBeVisible();

    publish(
      hostSession({
        ...PLAYING,
        revision: "21",
        connectedListeners: [
          listener("sam", "Sam's Phone"),
          listener("jo", "Jo's Pixel", { transportState: "connecting", syncConfidence: null }),
        ],
      }),
    );
    await waitFor(() =>
      expect(announcementRegion()).toHaveTextContent("Listener joined the session."),
    );
    expect(announcementRegion()).toHaveAttribute("aria-live", "polite");
    expect(screen.queryByRole("group", { name: "Decision for Jo's Pixel" })).toBeNull();
    expect(screen.getByText("No pending requests.")).toBeVisible();
    expect(
      within(listenerCard("Jo's Pixel")).getByText("Approved — waiting for its first clock sync"),
    ).toBeVisible();
    expect(
      within(screen.getByRole("region", { name: "Live playback and queue controls" })).getByText(
        "2 listeners",
      ),
    ).toBeVisible();

    publish(
      hostSession({
        ...PLAYING,
        revision: "22",
        connectedListeners: [listener("sam", "Sam's Phone"), listener("jo", "Jo's Pixel")],
      }),
    );
    await waitFor(() =>
      expect(within(listenerCard("Jo's Pixel")).getByText("Connected — sync good")).toBeVisible(),
    );
  });

  it("confirms a rejection only after delivery and announces it", async () => {
    renderScreen(hostSession({ pendingJoinRequests: [JO] }));
    fireEvent.click(await screen.findByRole("button", { name: "Reject" }));
    expect(await screen.findByText("Waiting for rejection delivery confirmation…")).toBeVisible();
    expect(screen.getByRole("group", { name: "Decision for Jo's Pixel" })).toBeDisabled();

    publish(
      hostSession({
        revision: "21",
        lastDelivery: { intendedPeers: 1, successfulPeers: 1, failedPeers: 0, severity: "ok" },
        lastDeliveryOperationId: "reject-jo",
      }),
    );
    await waitFor(() => expect(announcementRegion()).toHaveTextContent("Join request declined."));
    expect(screen.queryByText("Jo's Pixel")).toBeNull();
    expect(screen.getByText("No listeners have joined yet.")).toBeVisible();
  });

  it("times out an unconfirmed approval visibly and leaves the request actionable", async () => {
    let nowMs = 1_000_000;
    vi.spyOn(Date, "now").mockImplementation(() => nowMs);
    renderScreen(hostSession({ pendingJoinRequests: [JO] }));
    fireEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText("Waiting for approval delivery confirmation…")).toBeVisible();

    // A newer snapshot that still has no confirmation keeps waiting...
    nowMs += DELIVERY_CONFIRMATION_TIMEOUT_MS - 1;
    const pollsBefore = vi.mocked(client.getHostSessionState).mock.calls.length;
    publish(hostSession({ revision: "21", pendingJoinRequests: [JO] }));
    await waitFor(() =>
      expect(vi.mocked(client.getHostSessionState).mock.calls.length).toBeGreaterThan(pollsBefore),
    );
    await act(async () => {});
    expect(screen.getByText("Waiting for approval delivery confirmation…")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();

    // ...until the bound elapses, when the failure is shown and Approve returns.
    nowMs += 1;
    publish(hostSession({ revision: "22", pendingJoinRequests: [JO] }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("No delivery confirmation arrived within 15 seconds");
    expect(announcementRegion()).toHaveTextContent(
      "The join decision failed and remains actionable.",
    );
    expect(screen.queryByText("Waiting for approval delivery confirmation…")).toBeNull();
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Reject" })).toBeEnabled();
  });
});

describe("host presentation of listener reconnects", () => {
  it("shows a reconnecting listener and transport, then recovery, without leaving the session", async () => {
    renderScreen(
      hostSession({
        ...PLAYING,
        connectedListeners: [listener("sam", "Sam's Phone"), listener("jo", "Jo's Pixel")],
      }),
    );
    expect(await screen.findByText("Sam's Phone")).toBeVisible();
    const dashboard = screen.getByRole("region", { name: "Live playback and queue controls" });

    publish(
      hostSession({
        ...PLAYING,
        revision: "21",
        transportState: "retrying",
        connectedListeners: [
          listener("sam", "Sam's Phone", { transportState: "retrying", syncConfidence: null }),
          listener("jo", "Jo's Pixel"),
        ],
      }),
    );
    await waitFor(() =>
      expect(
        within(listenerCard("Sam's Phone")).getByText("Approved for this session · Reconnecting"),
      ).toBeVisible(),
    );
    const status = screen.getByRole("region", { name: "Host status" });
    expect(within(status).getByText("Reconnecting")).toBeVisible();
    expect(within(status).getByText("ATTENTION")).toBeVisible();
    expect(
      within(listenerCard("Jo's Pixel")).getByText("Approved for this session · Connected"),
    ).toBeVisible();

    publish(
      hostSession({
        ...PLAYING,
        revision: "22",
        connectedListeners: [listener("sam", "Sam's Phone"), listener("jo", "Jo's Pixel")],
      }),
    );
    await waitFor(() =>
      expect(
        within(listenerCard("Sam's Phone")).getByText("Approved for this session · Connected"),
      ).toBeVisible(),
    );
    expect(within(status).queryByText("ATTENTION")).toBeNull();
    expect(within(listenerCard("Sam's Phone")).getByText("Connected — sync good")).toBeVisible();
    // The same dashboard stayed mounted throughout: no navigation on reconnect.
    expect(screen.getByRole("region", { name: "Live playback and queue controls" })).toBe(
      dashboard,
    );
  });

  it("drops a listener that disconnects for good without a false success message", async () => {
    renderScreen(hostSession({ ...PLAYING, connectedListeners: [listener("sam", "Sam's Phone")] }));
    expect(await screen.findByText("Sam's Phone")).toBeVisible();
    publish(hostSession({ ...PLAYING, revision: "21", connectedListeners: [] }));
    await waitFor(() => expect(screen.queryByText("Sam's Phone")).toBeNull());
    expect(screen.getByText("0 listeners")).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(/no listener/i);
    expect(announcementRegion()).toBeEmptyDOMElement();
  });
});
