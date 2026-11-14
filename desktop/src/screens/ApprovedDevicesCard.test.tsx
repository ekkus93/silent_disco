import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApprovedDevicesCard } from "./ApprovedDevicesCard";

const { listRememberedListeners, revokeRememberedListener } = vi.hoisted(() => ({
  listRememberedListeners: vi.fn(),
  revokeRememberedListener: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return { ...actual, listRememberedListeners, revokeRememberedListener };
});

const kitchen = { deviceId: "v4-aa", displayName: "Kitchen phone", approvedAtMs: "0" };
const riley = { deviceId: "v4-bb", displayName: "Riley's phone", approvedAtMs: "1000" };

describe("ApprovedDevicesCard", () => {
  beforeEach(() => {
    listRememberedListeners.mockReset();
    revokeRememberedListener.mockReset();
  });

  it("lists the host's remembered listeners", async () => {
    listRememberedListeners.mockResolvedValue([kitchen, riley]);
    render(<ApprovedDevicesCard />);
    expect(await screen.findByText("Kitchen phone")).toBeInTheDocument();
    expect(screen.getByText("Riley's phone")).toBeInTheDocument();
    expect(screen.getAllByText(/Always allowed since/)).toHaveLength(2);
  });

  it("shows an empty state", async () => {
    listRememberedListeners.mockResolvedValue([]);
    render(<ApprovedDevicesCard />);
    expect(await screen.findByText(/No approved devices/)).toBeInTheDocument();
  });

  it("revokes only after confirmation and refreshes from the host", async () => {
    listRememberedListeners.mockResolvedValueOnce([kitchen, riley]).mockResolvedValueOnce([riley]);
    revokeRememberedListener.mockResolvedValue(true);
    render(<ApprovedDevicesCard />);
    await screen.findByText("Kitchen phone");

    const [first] = screen.getAllByRole("button", { name: "Remove approval" });
    if (first === undefined) throw new Error("expected a remove button");
    fireEvent.click(first);
    expect(revokeRememberedListener).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Remove Kitchen phone" }));

    await waitFor(() => expect(revokeRememberedListener).toHaveBeenCalledWith("v4-aa"));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Kitchen phone will need your approval next time it joins.",
    );
    await waitFor(() => expect(screen.queryByText("Kitchen phone")).not.toBeInTheDocument());
    expect(listRememberedListeners).toHaveBeenCalledTimes(2);
  });

  it("keeps the device when the host declines to keep it removed", async () => {
    listRememberedListeners.mockResolvedValue([kitchen]);
    render(<ApprovedDevicesCard />);
    await screen.findByText("Kitchen phone");
    fireEvent.click(screen.getByRole("button", { name: "Remove approval" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep approved" }));
    expect(revokeRememberedListener).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Remove approval" })).toBeInTheDocument();
  });

  it("shows a failure from the host", async () => {
    listRememberedListeners.mockRejectedValue({
      code: "desktop.trust.storage_failed",
      category: "storage",
      severity: "error",
      retryable: false,
      message: "could not read the trusted listeners",
    });
    render(<ApprovedDevicesCard />);
    expect(await screen.findByText("Approved devices unavailable")).toBeInTheDocument();
  });
});
