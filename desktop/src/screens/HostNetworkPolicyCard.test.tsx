import "@testing-library/jest-dom/vitest";

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { NetworkInterfaceSnapshotDto } from "../core/generated/desktop-bindings";
import { HostNetworkPolicyCard } from "./HostNetworkPolicyCard";

const { getHostNetworkState, setHostNetworkPreference, setAllowSharedAddressSpace } = vi.hoisted(
  () => ({
    getHostNetworkState: vi.fn(),
    setHostNetworkPreference: vi.fn(),
    setAllowSharedAddressSpace: vi.fn(),
  }),
);

vi.mock("../core/client", async () => {
  const actual = await vi.importActual<typeof import("../core/client")>("../core/client");
  return {
    ...actual,
    getHostNetworkState,
    setHostNetworkPreference,
    setAllowSharedAddressSpace,
  };
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
    allowSharedAddressSpace: false,
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
    render(<HostNetworkPolicyCard available disabled={false} />);

    expect(await screen.findByText(/Ready to bind enp1s0 at 192\.168\.1\.20/)).toBeInTheDocument();
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
    render(<HostNetworkPolicyCard available disabled={false} />);

    const select = await screen.findByLabelText("Host network interface");
    expect(select).toHaveValue("automatic");
    expect(screen.getByRole("alert")).toHaveTextContent("explicit selection");

    fireEvent.change(select, { target: { value: "wlp2s0\u0000192.168.2.30" } });
    await waitFor(() =>
      expect(setHostNetworkPreference).toHaveBeenCalledWith({
        mode: "explicit",
        interfaceName: "wlp2s0",
        address: "192.168.2.30",
      }),
    );
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

    render(<HostNetworkPolicyCard available disabled={false} />);
    const select = await screen.findByLabelText("Host network interface");
    const rejected = Array.from((select as HTMLSelectElement).options).filter(
      (option) => option.value !== "automatic",
    );
    expect(rejected).toHaveLength(2);
    expect(rejected.every((option) => option.disabled)).toBe(true);
  });

  it("shows mDNS as advertising once a session is actually bound", async () => {
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        activeBinding: {
          interfaceName: "enp1s0",
          address: "192.168.1.20",
          controlPort: 41_100,
          syncPort: 41_101,
          audioPort: 41_102,
          mdns: { active: true, failureReason: null },
        },
        activeBindingValid: true,
      }),
    );

    render(<HostNetworkPolicyCard available disabled={false} />);
    expect(
      await screen.findByText("Auto-discovery (mDNS): advertising this session."),
    ).toBeInTheDocument();
  });

  it("surfaces an mDNS publication failure without hiding the manual connection details", async () => {
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        activeBinding: {
          interfaceName: "enp1s0",
          address: "192.168.1.20",
          controlPort: 41_100,
          syncPort: 41_101,
          audioPort: 41_102,
          mdns: { active: false, failureReason: "mDNS daemon unavailable: multicast socket busy" },
        },
        activeBindingValid: true,
      }),
    );

    render(<HostNetworkPolicyCard available disabled={false} />);
    const failure = await screen.findByText(/Auto-discovery \(mDNS\) unavailable/);
    expect(failure).toHaveAttribute("role", "alert");
    expect(failure).toHaveTextContent("multicast socket busy");
    expect(failure).toHaveTextContent("manual connection details below still work");
    // The real binding summary (unrelated to mDNS) must still render too --
    // a publication failure must never hide it.
    expect(screen.getByText(/Bound enp1s0 at 192\.168\.1\.20/)).toBeInTheDocument();
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

    render(<HostNetworkPolicyCard available disabled={false} />);
    expect(
      await screen.findByText("the requested address is no longer active"),
    ).toBeInTheDocument();
    expect(screen.getByText("the active interface snapshot changed")).toBeInTheDocument();
  });
  // The CGNAT opt-in is persisted by the backend, so the card must send the
  // command and adopt the returned authoritative snapshot rather than keeping
  // its own copy of the flag.
  it("sends the shared-address-space opt-in and adopts the returned snapshot", async () => {
    const cgnat = {
      interfaceName: "wlo1",
      interfaceIndex: 3,
      address: "100.64.64.132",
      prefixLength: 22,
      classification: "shared_address_space" as const,
      isDefaultRoute: true,
      isActive: true,
      isPhysical: true,
      selectable: false,
      rejectionReason:
        "shared address space (carrier-grade NAT) is not advertised unless it is enabled in Settings",
    };
    getHostNetworkState.mockResolvedValue(
      networkSnapshot({
        candidates: [cgnat],
        automaticSelection: null,
        resolvedSelection: null,
        selectionError: "no active private-LAN IPv4 address is available for the desktop host",
      }),
    );
    setAllowSharedAddressSpace.mockResolvedValue(
      networkSnapshot({
        candidates: [{ ...cgnat, selectable: true, rejectionReason: null }],
        automaticSelection: { ...cgnat, selectable: true, rejectionReason: null },
        resolvedSelection: { ...cgnat, selectable: true, rejectionReason: null },
        allowSharedAddressSpace: true,
      }),
    );

    render(<HostNetworkPolicyCard available disabled={false} />);

    const toggle = await screen.findByLabelText(/Allow shared address space/);
    expect(toggle).not.toBeChecked();
    // The refusal must name the setting that changes it.
    expect(screen.getByText(/enabled in Settings/)).toBeInTheDocument();

    fireEvent.click(toggle);

    await waitFor(() => expect(setAllowSharedAddressSpace).toHaveBeenCalledWith(true));
    await waitFor(() => expect(screen.getByLabelText(/Allow shared address space/)).toBeChecked());
    expect(await screen.findByText(/Ready to bind wlo1 at 100\.64\.64\.132/)).toBeInTheDocument();
  });
});
