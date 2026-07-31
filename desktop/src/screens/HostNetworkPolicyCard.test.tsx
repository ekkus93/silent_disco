import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NetworkInterfaceSnapshotDto } from "../core/generated/desktop-bindings";
import { HostNetworkPolicyCard } from "./HostNetworkPolicyCard";

const { getHostNetworkState, setHostNetworkPreference } = vi.hoisted(() => ({
  getHostNetworkState: vi.fn(),
  setHostNetworkPreference: vi.fn(),
}));

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return { ...actual, getHostNetworkState, setHostNetworkPreference };
});

function candidate(interfaceName: string, address: string, isDefaultRoute: boolean) {
  return {
    interfaceName,
    interfaceIndex: interfaceName === "enp1s0" ? 2 : 3,
    address,
    prefixLength: 24,
    classification: "private_lan" as const,
    isDefaultRoute,
    isActive: true,
    isPhysical: true,
    selectable: true,
    rejectionReason: null,
  };
}

function networkSnapshot(overrides: Partial<NetworkInterfaceSnapshotDto> = {}) {
  const selected = candidate("enp1s0", "192.168.1.20", true);
  return {
    preference: { mode: "automatic", interfaceName: null, address: null },
    candidates: [selected],
    automaticSelection: selected,
    resolvedSelection: selected,
    requiresExplicitSelection: false,
    selectionError: null,
    activeBinding: null,
    activeBindingValid: false,
    interfaceChange: null,
    ...overrides,
  } satisfies NetworkInterfaceSnapshotDto;
}

beforeEach(() => {
  vi.clearAllMocks();
  getHostNetworkState.mockResolvedValue(networkSnapshot());
  setHostNetworkPreference.mockImplementation(async (request) =>
    networkSnapshot({
      preference: request,
      resolvedSelection:
        request.mode === "explicit"
          ? candidate(request.interfaceName ?? "", request.address ?? "", false)
          : candidate("enp1s0", "192.168.1.20", true),
      requiresExplicitSelection: false,
    }),
  );
});

describe("HostNetworkPolicyCard", () => {
  it("reports a single automatic private-LAN address as ready", async () => {
    const readiness = vi.fn();
    render(
      <HostNetworkPolicyCard available disabled={false} onReadinessChange={readiness} />,
    );

    expect(await screen.findByText(/Ready to bind enp1s0 at 192\.168\.1\.20/)).toBeInTheDocument();
    await waitFor(() => expect(readiness).toHaveBeenLastCalledWith(true));
  });

  it("requires an explicit choice when automatic selection is ambiguous", async () => {
    const first = candidate("enp1s0", "192.168.1.20", false);
    const second = candidate("wlp2s0", "192.168.2.30", false);
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        candidates: [first, second],
        automaticSelection: null,
        resolvedSelection: null,
        requiresExplicitSelection: true,
        selectionError: "multiple private-LAN addresses require an explicit selection",
      }),
    );
    const readiness = vi.fn();
    render(
      <HostNetworkPolicyCard available disabled={false} onReadinessChange={readiness} />,
    );

    const select = await screen.findByLabelText("Host network interface");
    expect(select).toHaveValue("automatic");
    expect(screen.getByRole("alert")).toHaveTextContent("explicit selection");
    await waitFor(() => expect(readiness).toHaveBeenLastCalledWith(false));

    fireEvent.change(select, { target: { value: "wlp2s0\u0000192.168.2.30" } });
    await waitFor(() =>
      expect(setHostNetworkPreference).toHaveBeenCalledWith({
        mode: "explicit",
        interfaceName: "wlp2s0",
        address: "192.168.2.30",
      }),
    );
    await waitFor(() => expect(readiness).toHaveBeenLastCalledWith(true));
  });

  it("shows rejected VPN and container addresses without allowing selection", async () => {
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        candidates: [
          {
            ...candidate("tun0", "10.8.0.2", false),
            classification: "vpn",
            isPhysical: false,
            selectable: false,
            rejectionReason: "VPN interfaces are not advertised automatically",
          },
          {
            ...candidate("docker0", "172.17.0.1", false),
            classification: "container",
            isPhysical: false,
            selectable: false,
            rejectionReason: "container interfaces are not host LAN endpoints",
          },
        ],
        automaticSelection: null,
        resolvedSelection: null,
        requiresExplicitSelection: false,
        selectionError: "no selectable private-LAN IPv4 address is active",
      }),
    );

    render(<HostNetworkPolicyCard available disabled={false} onReadinessChange={vi.fn()} />);
    const select = await screen.findByLabelText("Host network interface");
    const rejected = Array.from((select as HTMLSelectElement).options).filter(
      (option) => option.value !== "automatic",
    );
    expect(rejected).toHaveLength(2);
    expect(rejected.every((option) => option.disabled)).toBe(true);
  });

  it("keeps a vanished explicit address visible as an error after refresh", async () => {
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        preference: {
          mode: "explicit",
          interfaceName: "enp1s0",
          address: "192.168.1.20",
        },
        resolvedSelection: null,
        selectionError: "the requested address is no longer active",
        interfaceChange: "the active interface snapshot changed",
      }),
    );

    render(<HostNetworkPolicyCard available disabled={false} onReadinessChange={vi.fn()} />);
    expect(await screen.findByText("the requested address is no longer active")).toBeInTheDocument();
    expect(screen.getByText("the active interface snapshot changed")).toBeInTheDocument();
  });
});
