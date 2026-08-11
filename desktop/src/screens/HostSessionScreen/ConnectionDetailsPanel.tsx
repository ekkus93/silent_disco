import type { HostConnectionDto } from "../../core/generated/desktop-bindings";
import { CopyButton, Detail } from "./shared";

interface ConnectionDetailsPanelProps {
  connection: HostConnectionDto | null;
  onCopy: (label: string, value: string) => void;
}

export function ConnectionDetailsPanel({ connection, onCopy }: ConnectionDetailsPanelProps) {
  if (!connection) {
    return (
      <p className="mt-6 rounded-xl border border-amber-500/50 bg-amber-950/30 p-4 text-amber-100">
        The core has not published an active manual endpoint.
      </p>
    );
  }

  const connectionPayload = JSON.stringify({
    hostAddress: connection.hostAddress,
    controlPort: connection.controlPort,
    syncPort: connection.syncPort,
    audioPort: connection.audioPort,
    sessionId: connection.sessionId,
    protocolVersion: connection.protocolVersion,
    inviteCodeRequired: connection.inviteCodeRequired,
    expiresAtMs: connection.expiresAtMs,
  });

  return (
    <section
      aria-labelledby="connection-title"
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
      <h2 id="connection-title" className="text-xl font-semibold">
        Manual connection details
      </h2>
      <p className="mt-2 text-sm text-slate-400">Listeners can enter these details without mDNS.</p>
      <dl className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <Detail label="Host address" value={connection.hostAddress} />
        <Detail label="Control port" value={String(connection.controlPort)} />
        <Detail label="Sync port" value={String(connection.syncPort)} />
        <Detail label="Audio port" value={String(connection.audioPort)} />
        <Detail label="Session ID" value={connection.sessionId} />
        <Detail label="Protocol version" value={String(connection.protocolVersion)} />
        <Detail
          label="Invite code"
          value={connection.inviteCodeRequired ? "required" : "not required"}
        />
        <Detail label="Expiration" value={connection.expiresAtMs ?? "No expiration policy"} />
      </dl>
      <div className="mt-4 flex flex-wrap gap-2">
        <CopyButton
          label="Copy host address"
          onClick={() => onCopy("Host address", connection.hostAddress)}
        />
        <CopyButton
          label="Copy session ID"
          onClick={() => onCopy("Session ID", connection.sessionId)}
        />
        <CopyButton
          label="Copy connection payload"
          onClick={() => onCopy("Connection payload", connectionPayload)}
        />
      </div>
    </section>
  );
}
