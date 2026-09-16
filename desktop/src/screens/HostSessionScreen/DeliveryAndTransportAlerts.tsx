import type {
  BroadcastDeliveryDto,
  DeliveryReportDto,
  DesktopErrorDto,
} from "../../core/generated/desktop-bindings";
import { ErrorAlert } from "./shared";

interface DeliveryAndTransportAlertsProps {
  lastDelivery: DeliveryReportDto | null;
  broadcast: BroadcastDeliveryDto | null;
  playbackState: string;
  connectedListenerCount: number;
  transportError: string | null;
  lastError: DesktopErrorDto | null;
  refreshFailure: DesktopErrorDto | null;
}

/** Playback states in which the host is sending audio frames. */
const SENDING_AUDIO_STATES = new Set(["buffering", "playing"]);

export function DeliveryAndTransportAlerts({
  lastDelivery,
  broadcast,
  playbackState,
  connectedListenerCount,
  transportError,
  lastError,
  refreshFailure,
}: DeliveryAndTransportAlertsProps) {
  // Block 52.5: the only delivery line used to describe the last *control*
  // message, so it stayed green ("1 of 1 succeeded") for minutes while the
  // audio reached nobody. Audio delivery is reported separately, and sending
  // audio with no listener connected is an alert in its own right.
  const streamingToNobody = SENDING_AUDIO_STATES.has(playbackState) && connectedListenerCount === 0;
  const framesAttempted = Number(broadcast?.framesAttempted ?? 0);
  const framesWithoutRecipients = Number(broadcast?.framesWithoutRecipients ?? 0);
  const framesImperfect =
    Number(broadcast?.framesFailed ?? 0) +
    Number(broadcast?.framesPartiallyDelivered ?? 0) +
    framesWithoutRecipients;
  return (
    <>
      {streamingToNobody ? (
        <p
          role="alert"
          className="mt-6 rounded-xl border border-rose-500/60 bg-rose-950/30 p-4 text-sm text-rose-100"
        >
          Streaming to nobody: audio is playing but no listener is connected, so it reaches no one.
        </p>
      ) : null}
      {broadcast && framesAttempted > 0 ? (
        <p
          className={`mt-6 rounded-xl border p-4 text-sm ${
            framesImperfect > 0
              ? "border-amber-500/60 bg-amber-950/30 text-amber-100"
              : "border-slate-700 bg-slate-900 text-slate-200"
          }`}
        >
          Audio this stream: {broadcast.framesFullyDelivered} of {broadcast.framesAttempted} frames
          reached every listener; {broadcast.framesPartiallyDelivered} partially,{" "}
          {broadcast.framesWithoutRecipients} to nobody, {broadcast.framesFailed} failed.
        </p>
      ) : null}
      {lastDelivery ? (
        <p
          role={lastDelivery.successfulPeers === 0 ? "alert" : undefined}
          className={`mt-3 rounded-xl border p-4 text-sm ${
            lastDelivery.successfulPeers === 0 && lastDelivery.intendedPeers > 0
              ? "border-rose-500/60 bg-rose-950/30 text-rose-100"
              : lastDelivery.failedPeers > 0
                ? "border-amber-500/60 bg-amber-950/30 text-amber-100"
                : "border-emerald-500/60 bg-emerald-950/30 text-emerald-100"
          }`}
        >
          {lastDelivery.successfulPeers === 0 && lastDelivery.intendedPeers > 0
            ? `Last control message (approval, rejection, or removal) reached nobody: 0 of ${lastDelivery.intendedPeers} listeners (${lastDelivery.severity}).`
            : `Last control message (approval, rejection, or removal): ${lastDelivery.successfulPeers} of ${lastDelivery.intendedPeers} delivered; ${lastDelivery.failedPeers} failed (${lastDelivery.severity}).`}
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
