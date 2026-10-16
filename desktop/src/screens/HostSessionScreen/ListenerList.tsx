import type {
  ConnectedListenerDto,
  DeliveryReportDto,
  DesktopErrorDto,
} from "../../core/generated/desktop-bindings";
import { listenerProgress, type PendingOperation } from "./domain";
import { ListenerDetailScreen } from "../ListenerDetailScreen";

function trustLabel(value: string): string {
  return value === "trusted" ? "Trusted device" : "Approved for this session";
}

function connectionLabel(value: string): string {
  switch (value) {
    case "connected":
      return "Connected";
    case "retrying":
      return "Reconnecting";
    case "disconnected":
      return "Disconnected";
    case "failed":
      return "Connection failed";
    default:
      return value.replaceAll("_", " ");
  }
}

interface ListenerListProps {
  listeners: ConnectedListenerDto[];
  playbackState: string;
  selectedListenerId: string | null;
  onSelectListener: (deviceId: string | null) => void;
  lastDelivery: DeliveryReportDto | null;
  removalOperations: Record<string, PendingOperation>;
  removalFailures: Record<string, DesktopErrorDto>;
  onRequestRemoval: (listenerId: string) => void;
}

export function ListenerList({
  listeners,
  playbackState,
  selectedListenerId,
  onSelectListener,
  lastDelivery,
  removalOperations,
  removalFailures,
  onRequestRemoval,
}: ListenerListProps) {
  const selectedListener = listeners.find((listener) => listener.deviceId === selectedListenerId);

  return (
    <section aria-labelledby="listeners-title" className="sd-workflow-card mt-6 p-5">
      <h2 id="listeners-title" className="text-xl font-semibold">
        Connected listeners
      </h2>
      {selectedListener ? (
        <div className="mt-4">
          <ListenerDetailScreen
            listener={selectedListener}
            lastDelivery={lastDelivery}
            pending={Boolean(removalOperations[selectedListener.deviceId])}
            failure={removalFailures[selectedListener.deviceId] ?? null}
            onRemove={() => onRequestRemoval(selectedListener.deviceId)}
            onBack={() => onSelectListener(null)}
          />
        </div>
      ) : listeners.length === 0 ? (
        <p className="mt-3 text-sm text-slate-400">No listeners have joined yet.</p>
      ) : (
        <ul className="mt-4 grid gap-3 md:grid-cols-2">
          {listeners.map((listener) => (
            <li key={listener.deviceId} className="sd-workflow-card border-0 p-4 shadow-none">
              <p className="font-semibold">{listener.displayName}</p>
              <p className="mt-1 text-sm text-slate-200">
                {listenerProgress(listener, playbackState)}
              </p>
              <p className="mt-1 text-xs text-slate-400">
                {trustLabel(listener.trustState)} · {connectionLabel(listener.transportState)}
              </p>
              <button
                type="button"
                onClick={() => onSelectListener(listener.deviceId)}
                className="sd-workflow-control mt-3 rounded-sd-control border border-sd-outline px-3 py-2 text-sm font-semibold text-sd-on-surface hover:border-sd-primary"
              >
                View {listener.displayName}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
