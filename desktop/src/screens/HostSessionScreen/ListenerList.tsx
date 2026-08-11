import type {
  ConnectedListenerDto,
  DeliveryReportDto,
  DesktopErrorDto,
} from "../../core/generated/desktop-bindings";
import type { PendingOperation } from "./domain";
import { ListenerDetailScreen } from "../ListenerDetailScreen";

interface ListenerListProps {
  listeners: ConnectedListenerDto[];
  selectedListenerId: string | null;
  onSelectListener: (deviceId: string | null) => void;
  lastDelivery: DeliveryReportDto | null;
  removalOperations: Record<string, PendingOperation>;
  removalFailures: Record<string, DesktopErrorDto>;
  onRequestRemoval: (listenerId: string) => void;
}

export function ListenerList({
  listeners,
  selectedListenerId,
  onSelectListener,
  lastDelivery,
  removalOperations,
  removalFailures,
  onRequestRemoval,
}: ListenerListProps) {
  const selectedListener = listeners.find((listener) => listener.deviceId === selectedListenerId);

  return (
    <section
      aria-labelledby="listeners-title"
      className="mt-6 rounded-2xl border border-slate-700 bg-slate-950/70 p-5"
    >
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
        <p className="mt-3 text-sm text-slate-400">
          No listeners have completed delivery-confirmed approval.
        </p>
      ) : (
        <ul className="mt-4 grid gap-3 md:grid-cols-2">
          {listeners.map((listener) => (
            <li key={listener.deviceId} className="rounded-xl bg-slate-900 p-4">
              <p className="font-semibold">{listener.displayName}</p>
              <p className="mt-1 text-xs text-slate-400">
                {listener.transportState} · {listener.trustState}
              </p>
              <p className="mt-1 text-xs text-slate-400">
                Sync: {listener.syncConfidence ?? "not available"}
              </p>
              <button
                type="button"
                onClick={() => onSelectListener(listener.deviceId)}
                className="mt-3 rounded-lg border border-slate-600 px-3 py-2 text-sm font-semibold hover:border-slate-400"
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
