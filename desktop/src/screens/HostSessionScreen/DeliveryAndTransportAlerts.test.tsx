import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { BroadcastDeliveryDto } from "../../core/generated/desktop-bindings";
import { DeliveryAndTransportAlerts } from "./DeliveryAndTransportAlerts";

function broadcast(overrides: Partial<BroadcastDeliveryDto> = {}): BroadcastDeliveryDto {
  return {
    framesAttempted: "0",
    framesFailed: "0",
    framesFullyDelivered: "0",
    framesPartiallyDelivered: "0",
    framesUndelivered: "0",
    framesWithoutRecipients: "0",
    recipientsIntended: "0",
    recipientsDelivered: "0",
    queueDepth: "0",
    queuePeakDepth: "0",
    queueOverflows: "0",
    ...overrides,
  };
}

const approvalDelivered = {
  intendedPeers: 1,
  successfulPeers: 1,
  failedPeers: 0,
  severity: "ok",
};

// Block 52.5, observed 2026-09-16: "Last delivery: 1 of 1 succeeded (ok)" --
// the approval -- stayed green for three minutes while every audio frame went
// to nobody, because the listener had been dropped.
describe("DeliveryAndTransportAlerts", () => {
  it("raises an alert when audio is playing with no listener connected", () => {
    render(
      <DeliveryAndTransportAlerts
        lastDelivery={approvalDelivered}
        broadcast={broadcast({
          framesAttempted: "7122",
          framesWithoutRecipients: "7122",
        })}
        playbackState="playing"
        connectedListenerCount={0}
        transportError={null}
        lastError={null}
        refreshFailure={null}
      />,
    );

    const alerts = screen.getAllByRole("alert");
    expect(alerts.some((alert) => /Streaming to nobody/.test(alert.textContent ?? ""))).toBe(true);
    expect(screen.getByText(/7122 sent with no listener connected/)).toBeInTheDocument();
    // The approval line no longer reads as a statement about the audio.
    expect(
      screen.getByText(/Last control message \(approval, rejection, or removal\)/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/^Last delivery:/)).not.toBeInTheDocument();
  });

  it("does not alarm while stopped with no listeners", () => {
    render(
      <DeliveryAndTransportAlerts
        lastDelivery={null}
        broadcast={broadcast()}
        playbackState="stopped"
        connectedListenerCount={0}
        transportError={null}
        lastError={null}
        refreshFailure={null}
      />,
    );
    expect(screen.queryByText(/Streaming to nobody/)).not.toBeInTheDocument();
  });

  it("reports clean audio delivery without an alert", () => {
    render(
      <DeliveryAndTransportAlerts
        lastDelivery={approvalDelivered}
        broadcast={broadcast({ framesAttempted: "3120", framesFullyDelivered: "3120" })}
        playbackState="playing"
        connectedListenerCount={1}
        transportError={null}
        lastError={null}
        refreshFailure={null}
      />,
    );
    expect(screen.queryByText(/Streaming to nobody/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/Packets sent this session: 3120 of 3120 reached every listener/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  // Block 52.5, observed 2026-09-16: with one listener, 11 packets that
  // missed it were shown as "11 partially", as if someone had heard them.
  it("reports packets that reached no listener apart from partial delivery", () => {
    render(
      <DeliveryAndTransportAlerts
        lastDelivery={approvalDelivered}
        broadcast={broadcast({
          framesAttempted: "24827",
          framesFullyDelivered: "9302",
          framesUndelivered: "11",
          framesWithoutRecipients: "15514",
        })}
        playbackState="playing"
        connectedListenerCount={1}
        transportError={null}
        lastError={null}
        refreshFailure={null}
      />,
    );
    expect(screen.getByText(/0 reached only some/)).toBeInTheDocument();
    expect(screen.getByText(/11 reached no listener/)).toBeInTheDocument();
  });
});
