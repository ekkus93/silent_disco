import { useCallback, useEffect, useState } from "react";

import { Button } from "../components/Button";
import { ErrorAlert } from "../components/ErrorAlert";
import { Panel } from "../components/Panel";
import {
  listRememberedListeners,
  revokeRememberedListener,
  toDesktopBridgeError,
} from "../core/client";
import type { DesktopErrorDto, RememberedListenerDto } from "../core/generated/desktop-bindings";

function approvedLabel(approvedAtMs: string): string {
  const milliseconds = Number(approvedAtMs);
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) return "Always allowed";
  return `Always allowed since ${new Date(milliseconds).toLocaleString()}`;
}

/// The listeners this host remembered with "Always allow". They join
/// Approved-devices sessions without asking. The list comes from the host's
/// own verified records; removing one takes effect from its next join.
export function ApprovedDevicesCard() {
  const [listeners, setListeners] = useState<RememberedListenerDto[] | null>(null);
  const [error, setError] = useState<DesktopErrorDto | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setListeners(await listRememberedListeners());
      setError(null);
    } catch (failure: unknown) {
      setError(toDesktopBridgeError(failure, "list approved devices"));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const remove = async (listener: RememberedListenerDto) => {
    setConfirming(null);
    setRemoving(listener.deviceId);
    setMessage(null);
    try {
      const revoked = await revokeRememberedListener(listener.deviceId);
      setMessage(
        revoked
          ? `${listener.displayName} will need your approval next time it joins.`
          : "That device was already removed. The list has been refreshed.",
      );
      await load();
    } catch (failure: unknown) {
      setError(toDesktopBridgeError(failure, "remove approved device"));
    } finally {
      setRemoving(null);
    }
  };

  return (
    <Panel title="Approved devices">
      {error ? <ErrorAlert error={error} title="Approved devices unavailable" /> : null}
      {message ? (
        <p role="status" className="mb-3 text-sm text-slate-200">
          {message}
        </p>
      ) : null}
      {listeners === null && !error ? (
        <p className="text-sm text-slate-300">Loading approved devices…</p>
      ) : null}
      {listeners !== null && listeners.length === 0 ? (
        <p className="text-sm text-slate-300">
          No approved devices. Listeners you approve with "Always allow" appear here.
        </p>
      ) : null}
      {listeners !== null && listeners.length > 0 ? (
        <>
          <p className="mb-3 text-sm text-slate-300">
            These devices can join your sessions without asking. Removing one does not disconnect a
            listener that is already connected.
          </p>
          <ul className="flex flex-col gap-2">
            {listeners.map((listener) => (
              <li
                key={listener.deviceId}
                className="rounded-lg bg-slate-900 p-3 text-sm text-slate-100"
              >
                <p className="font-semibold">{listener.displayName}</p>
                <p className="mt-1 text-xs text-slate-300">
                  {approvedLabel(listener.approvedAtMs)}
                </p>
                <div className="mt-2 flex gap-2">
                  {confirming === listener.deviceId ? (
                    <>
                      <Button variant="danger" onClick={() => void remove(listener)}>
                        Remove {listener.displayName}
                      </Button>
                      <Button onClick={() => setConfirming(null)}>Keep approved</Button>
                    </>
                  ) : (
                    <Button
                      variant="danger"
                      disabled={removing !== null}
                      onClick={() => setConfirming(listener.deviceId)}
                    >
                      {removing === listener.deviceId ? "Removing approval…" : "Remove approval"}
                    </Button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </Panel>
  );
}
