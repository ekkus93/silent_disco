import { useEffect } from "react";

import { Button } from "../components/Button";
import { useHostNetworkState } from "./useHostNetworkState";

/// Read-only network readiness for host setup.
///
/// The control that *changes* network policy lives on Settings, because it is
/// machine configuration that outlives a session. Host setup still has to know
/// whether the policy resolves, because session creation is gated on it, so it
/// shows the outcome and hands off to Settings rather than duplicating the
/// control.
export function HostNetworkReadinessSummary({
  available,
  onReadinessChange,
  onConfigure,
}: {
  available: boolean;
  onReadinessChange: (ready: boolean) => void;
  onConfigure: () => void;
}) {
  const { snapshot, loading, error, ready } = useHostNetworkState(available);

  // Reported from an effect, never during render: this sets state on the
  // parent, and doing that mid-render is what React forbids.
  useEffect(() => {
    onReadinessChange(ready);
  }, [onReadinessChange, ready]);

  const resolved = snapshot?.resolvedSelection ?? null;

  return (
    <section
      aria-labelledby="host-network-readiness-title"
      className="rounded-2xl border border-violet-200/15 bg-black/25 p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="host-network-readiness-title" className="font-semibold text-cyan-200">
            Network interface
          </h3>
          <p className="mt-1 text-sm text-violet-100/65">
            Configured in Settings. Only an intentional private-LAN IPv4 address can be advertised.
          </p>
        </div>
        <Button onClick={onConfigure} className="px-3">
          Configure in Settings
        </Button>
      </div>

      {!available ? (
        <p className="mt-3 text-sm text-amber-200">
          Local-network capability has not been confirmed by the platform runner.
        </p>
      ) : loading && snapshot === null ? (
        <p role="status" className="mt-3 text-sm text-violet-100/70">
          Inspecting active network interfaces…
        </p>
      ) : resolved !== null ? (
        <p className="mt-3 text-sm text-emerald-200">
          Ready to bind {resolved.interfaceName} at {resolved.address}.
        </p>
      ) : snapshot !== null ? (
        <p role="alert" className="mt-3 text-sm text-amber-200">
          {snapshot.selectionError ??
            "No private-LAN address is selected. Choose one in Settings before starting a session."}
        </p>
      ) : null}

      {error !== null ? (
        <p role="alert" className="mt-3 text-sm text-red-200">
          {error}
        </p>
      ) : null}
    </section>
  );
}
