import "@testing-library/jest-dom/vitest";

import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DesktopDiagnosticsDto } from "../core/generated/desktop-bindings";
import { DiagnosticsScreen } from "./DiagnosticsScreen";

const { getHostDiagnostics, exportHostDiagnostics } = vi.hoisted(() => ({
  getHostDiagnostics: vi.fn(),
  exportHostDiagnostics: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return {
    ...actual,
    getHostDiagnostics,
    exportHostDiagnostics,
  };
});

function diagnosticsFixture(): DesktopDiagnosticsDto {
  return {
    versions: {
      coreVersion: { major: 0, minor: 1, patch: 0 },
      appVersion: "0.1.0",
      exportSchemaVersion: 1,
    },
    profile: { profileId: "block45", platform: "linux" },
    storage: {
      available: true,
      schemaVersion: 1,
      journalMode: "wal",
      foreignKeysEnabled: true,
      integrityCheck: "ok",
      appliedMigrationCount: 1,
      failureReason: null,
    },
    identity: {
      deviceIdentityPresent: true,
      signingIdentityPresent: true,
      signingKeyFingerprint: "block45-fingerprint",
    },
    endpoint: null,
    transport: { state: "running", lastDelivery: null, broadcast: null },
    listeners: [],
    listenersTruncated: false,
    synchronization: null,
    decodeQueue: null,
    packetizeQueue: null,
    monitor: {
      enabled: false,
      active: false,
      failureReason: null,
      callbackCount: null,
      framesWritten: null,
      framesSilenceFilled: null,
    },
    notificationBridge: { deliveryFailure: null },
    lastError: null,
    shuttingDown: false,
    generatedAtMs: "0",
  };
}

describe("DiagnosticsScreen Block 45 cadence", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    getHostDiagnostics.mockReset();
    exportHostDiagnostics.mockReset();
    getHostDiagnostics.mockResolvedValue(diagnosticsFixture());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("polls diagnostics at the measured two-second UI update cadence", async () => {
    const view = render(<DiagnosticsScreen />);
    await act(async () => Promise.resolve());
    expect(getHostDiagnostics).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(6_000);
    });
    expect(getHostDiagnostics).toHaveBeenCalledTimes(4);

    view.unmount();
  });
});
