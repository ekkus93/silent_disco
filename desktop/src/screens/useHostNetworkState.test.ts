import { describe, expect, it } from "vitest";

import type { NetworkInterfaceSnapshotDto } from "../core/generated/desktop-bindings";
import { networkReadiness } from "./useHostNetworkState";

const candidate = {
  interfaceName: "enp1s0",
  interfaceIndex: 2,
  address: "192.168.1.20",
  prefixLength: 24,
  classification: "private_lan" as const,
  isDefaultRoute: true,
  isActive: true,
  isPhysical: true,
  selectable: true,
  rejectionReason: null,
};

function snapshot(
  overrides: Partial<NetworkInterfaceSnapshotDto> = {},
): NetworkInterfaceSnapshotDto {
  return {
    preference: { mode: "automatic", interfaceName: null, address: null },
    candidates: [candidate],
    automaticSelection: candidate,
    resolvedSelection: candidate,
    requiresExplicitSelection: false,
    selectionError: null,
    activeBinding: null,
    activeBindingValid: false,
    interfaceChange: null,
    ...overrides,
  };
}

// Host setup gates session creation on this rule while Settings owns the
// control that changes it. Asserting the rule directly keeps the two from
// drifting apart, which a callback on one of the two screens could not.
describe("networkReadiness", () => {
  it("is ready for a single resolvable automatic private-LAN address", () => {
    expect(networkReadiness(snapshot(), true)).toBe(true);
  });

  it("is not ready when the platform has not confirmed local-network capability", () => {
    expect(networkReadiness(snapshot(), false)).toBe(false);
  });

  it("is not ready before a snapshot has been loaded", () => {
    expect(networkReadiness(null, true)).toBe(false);
  });

  it("is not ready when automatic selection is ambiguous", () => {
    expect(
      networkReadiness(
        snapshot({
          automaticSelection: null,
          resolvedSelection: null,
          requiresExplicitSelection: true,
          selectionError: "multiple private-LAN addresses require an explicit selection",
        }),
        true,
      ),
    ).toBe(false);
  });

  it("becomes ready once an explicit selection resolves", () => {
    expect(
      networkReadiness(
        snapshot({
          preference: {
            mode: "explicit",
            interfaceName: "wlp2s0",
            address: "192.168.2.30",
          },
          requiresExplicitSelection: false,
          selectionError: null,
        }),
        true,
      ),
    ).toBe(true);
  });

  it("is not ready when an active binding is no longer valid", () => {
    expect(
      networkReadiness(
        snapshot({
          activeBinding: {
            interfaceName: "enp1s0",
            address: "192.168.1.20",
            controlPort: 47000,
            syncPort: 47001,
            audioPort: 47002,
            mdns: { active: true, failureReason: null },
          },
          activeBindingValid: false,
        }),
        true,
      ),
    ).toBe(false);
  });
});
