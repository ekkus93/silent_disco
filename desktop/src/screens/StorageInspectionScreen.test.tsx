import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getStorageInspection } from "../core/client";
import { StorageInspectionScreen } from "./StorageInspectionScreen";

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return { ...actual, getStorageInspection: vi.fn() };
});

const inspection = {
  sqliteVersion: "3.50.0",
  foreignKeysEnabled: true,
  journalMode: "wal",
  busyTimeoutMs: 2000,
  synchronousPolicy: "full",
  schemaVersion: 2,
  appliedMigrations: [],
  integrityCheck: "ok",
  settings: null,
  trustedDevices: [
    {
      deviceId: "listener-1",
      displayName: "Phone",
      trustState: "trusted",
      firstSeenMs: "1",
      lastSeenMs: "2",
      updatedAtMs: "3",
      hasPublicKey: true,
      hasPrivateKeyReference: true,
    },
  ],
  recentSessions: [
    {
      sessionId: "session-1",
      role: "host",
      sessionName: "Friday set",
      startedAtMs: "10",
      endedAtMs: "20",
      listenerCount: 3,
      outcome: "completed",
      failureCode: null,
      failureMessage: null,
    },
  ],
  p2StoreApplicable: false,
};

describe("StorageInspectionScreen", () => {
  beforeEach(() => {
    vi.mocked(getStorageInspection).mockReset();
  });

  it("renders real typed storage data and the deliberate P2 not-applicable state", async () => {
    vi.mocked(getStorageInspection).mockResolvedValue(inspection);
    render(<StorageInspectionScreen />);

    expect(await screen.findByText("Friday set")).toBeInTheDocument();
    expect(screen.getByText(/Phone/)).toBeInTheDocument();
    expect(screen.getByText("not applicable")).toBeInTheDocument();
    expect(screen.getByText("wal")).toBeInTheDocument();
  });

  it("surfaces a structured backend failure instead of substituting empty success", async () => {
    vi.mocked(getStorageInspection).mockRejectedValue({
      code: "core.storage_query_failed",
      subsystem: "storage",
      severity: "error",
      retryable: false,
      message: "stored session row is corrupt",
    });
    render(<StorageInspectionScreen />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "core.storage_query_failed: stored session row is corrupt",
    );
    expect(screen.queryByText("No session history.")).not.toBeInTheDocument();
  });

  it("refreshes explicitly", async () => {
    vi.mocked(getStorageInspection).mockResolvedValue(inspection);
    const user = userEvent.setup();
    render(<StorageInspectionScreen />);
    await screen.findByText("Friday set");

    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(getStorageInspection).toHaveBeenCalledTimes(2));
  });
});
