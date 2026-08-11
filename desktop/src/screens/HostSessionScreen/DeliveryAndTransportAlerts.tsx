import type {
  BroadcastDeliveryDto,
  DeliveryReportDto,
  DesktopErrorDto,
} from "../../core/generated/desktop-bindings";
import { ErrorAlert } from "./shared";

interface DeliveryAndTransportAlertsProps {
  lastDelivery: DeliveryReportDto | null;
  broadcast: BroadcastDeliveryDto | null;
  transportError: string | null;
  lastError: DesktopErrorDto | null;
  refreshFailure: DesktopErrorDto | null;
}

export function DeliveryAndTransportAlerts({
  lastDelivery,
  broadcast,
  transportError,
  lastError,
  refreshFailure,
}: DeliveryAndTransportAlertsProps) {
  return (
    <>
      {lastDelivery ? (
        <p
          role={lastDelivery.successfulPeers === 0 ? "alert" : undefined}
          className={`mt-6 rounded-xl border p-4 text-sm ${
            lastDelivery.successfulPeers === 0 && lastDelivery.intendedPeers > 0
              ? "border-rose-500/60 bg-rose-950/30 text-rose-100"
              : lastDelivery.failedPeers > 0
                ? "border-amber-500/60 bg-amber-950/30 text-amber-100"
                : "border-emerald-500/60 bg-emerald-950/30 text-emerald-100"
          }`}
        >
          {lastDelivery.successfulPeers === 0 && lastDelivery.intendedPeers > 0
            ? `Last delivery reached nobody: 0 of ${lastDelivery.intendedPeers} listeners (${lastDelivery.severity}).`
            : `Last delivery: ${lastDelivery.successfulPeers} of ${lastDelivery.intendedPeers} succeeded; ${lastDelivery.failedPeers} failed (${lastDelivery.severity}).`}
        </p>
      ) : null}
      {broadcast ? (
        <p
          role={Number(broadcast.queueOverflows) > 0 ? "alert" : undefined}
          className={`mt-3 text-sm ${
            Number(broadcast.queueOverflows) > 0 ? "text-amber-200" : "text-slate-400"
          }`}
        >
          Broadcast queue: {broadcast.queueDepth} queued (peak {broadcast.queuePeakDepth})
          {Number(broadcast.queueOverflows) > 0
            ? `; ${broadcast.queueOverflows} frame(s) dropped for a full queue.`
            : "."}
        </p>
      ) : null}
      {transportError ? (
        <p role="alert" className="mt-6 text-sm text-rose-200">
          Transport worker: {transportError}
        </p>
      ) : null}
      {lastError ? <ErrorAlert error={lastError} /> : null}
      {refreshFailure ? <ErrorAlert error={refreshFailure} /> : null}
    </>
  );
}
