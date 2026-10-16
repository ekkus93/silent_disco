import { ConnectionDetailsPanel } from "./ConnectionDetailsPanel";
import { DeliveryAndTransportAlerts } from "./DeliveryAndTransportAlerts";
import { ListenerList } from "./ListenerList";
import { PendingJoinRequests } from "./PendingJoinRequests";
import { PlaybackControls } from "./PlaybackControls";
import { HostQueueControls } from "./HostQueueControls";
import { QrInvitationPanel } from "./QrInvitationPanel";
import { SessionHeaderAndStatus } from "./SessionHeaderAndStatus";
import { ErrorAlert } from "./shared";
import { useHostSessionViewModel } from "./useHostSessionViewModel";

export function HostSessionScreen() {
  const vm = useHostSessionViewModel();
  const { snapshot } = vm;

  if (!snapshot) {
    return (
      <section className="mx-auto max-w-6xl p-6 text-slate-100">
        <h2 className="text-2xl font-semibold">Host session</h2>
        <p className="mt-3">Loading authoritative host state…</p>
        {vm.refreshFailure ? <ErrorAlert error={vm.refreshFailure} /> : null}
      </section>
    );
  }

  const pendingRequestIds = new Set(
    snapshot.pendingJoinRequests.map((request) => request.requestId),
  );
  const connectedListenerIds = new Set(
    snapshot.connectedListeners.map((listener) => listener.deviceId),
  );
  const orphanDecisionFailures = Object.entries(vm.decisionFailures).filter(
    ([requestId]) => !pendingRequestIds.has(requestId),
  );
  const orphanRemovalFailures = Object.entries(vm.removalFailures).filter(
    ([listenerId]) => !connectedListenerIds.has(listenerId),
  );

  return (
    <section className="mx-auto max-w-6xl p-6 text-slate-100 [@media(max-height:700px)]:p-3">
      <SessionHeaderAndStatus
        sessionName={snapshot.sessionName}
        onRefresh={vm.refresh}
        onEndSession={vm.endSession}
        endPending={vm.endPending}
        announcement={vm.announcement}
        copyStatus={vm.copyStatus}
        hostLifecycle={snapshot.hostLifecycle}
        transportState={snapshot.transportState}
        playbackState={snapshot.playbackState}
        transportWorkerRunning={snapshot.transportWorkerRunning}
      />

      <section
        aria-label="Live playback and queue controls"
        className="host-live-dashboard sticky top-0 z-20 -mx-2 mt-3 max-h-[calc(100vh-1rem)] overflow-y-auto bg-[#171229]/95 px-2 pb-2 backdrop-blur [@media(max-height:700px)]:-mx-1 [@media(max-height:700px)]:mt-2 [@media(max-height:700px)]:px-1 [@media(max-height:700px)]:pb-1"
      >
        <PlaybackControls
          audioSource={snapshot.audioSource}
          playbackPositionMs={snapshot.playbackPositionMs}
          streamEndedNaturally={snapshot.streamEndedNaturally}
          playbackState={snapshot.playbackState}
          playbackControlsEnabled={snapshot.playbackControlsEnabled}
          playbackPending={vm.playbackPending}
          onControlPlayback={vm.controlPlayback}
          monitor={snapshot.monitor}
          monitorPending={vm.monitorPending}
          onToggleMonitor={vm.toggleMonitor}
        />

        <HostQueueControls
          queue={snapshot.hostQueue}
          revision={snapshot.revision}
          playbackState={snapshot.playbackState}
          onRefresh={vm.refresh}
        />

        <div className="flex items-center justify-between gap-3 rounded-xl border border-slate-700 bg-slate-950/70 px-4 py-3 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-2">
          <span className="text-sm font-medium">
            {snapshot.connectedListeners.length} listeners
          </span>
          <button
            type="button"
            onClick={vm.refreshInvitation}
            disabled={vm.invitationPending}
            className="rounded-lg border border-cyan-500/60 px-4 py-2 text-sm font-semibold text-cyan-100 hover:bg-cyan-950/40 disabled:opacity-50 [@media(max-height:700px)]:px-3 [@media(max-height:700px)]:py-1"
          >
            {vm.invitationPending
              ? "Creating invite…"
              : vm.invitation
                ? "Refresh invite"
                : "Create invite"}
          </button>
        </div>
      </section>

      <ConnectionDetailsPanel connection={snapshot.connection} onCopy={vm.copyValue} />

      {orphanDecisionFailures.map(([requestId, error]) => (
        <ErrorAlert key={`decision-${requestId}`} error={error} />
      ))}
      {orphanRemovalFailures.map(([listenerId, error]) => (
        <ErrorAlert key={`removal-${listenerId}`} error={error} />
      ))}

      {snapshot.connection ? (
        <QrInvitationPanel
          invitation={vm.invitation}
          invitationQrDataUrl={vm.invitationQrDataUrl}
          invitationPending={vm.invitationPending}
          invitationError={vm.invitationError}
          onRefreshInvitation={vm.refreshInvitation}
          onCopy={vm.copyValue}
        />
      ) : null}

      <PendingJoinRequests
        requests={snapshot.pendingJoinRequests}
        decisionOperations={vm.decisionOperations}
        decisionFailures={vm.decisionFailures}
        remember={vm.remember}
        setRemember={vm.setRemember}
        onDecide={vm.decide}
      />

      <ListenerList
        listeners={snapshot.connectedListeners}
        playbackState={snapshot.playbackState}
        selectedListenerId={vm.selectedListenerId}
        onSelectListener={vm.setSelectedListenerId}
        lastDelivery={snapshot.lastDelivery}
        removalOperations={vm.removalOperations}
        removalFailures={vm.removalFailures}
        onRequestRemoval={vm.requestRemoval}
      />

      <DeliveryAndTransportAlerts
        lastDelivery={snapshot.lastDelivery}
        broadcast={snapshot.broadcast}
        playbackState={snapshot.playbackState}
        connectedListenerCount={snapshot.connectedListeners.length}
        transportError={snapshot.transportError}
        lastError={snapshot.lastError}
        refreshFailure={vm.refreshFailure}
      />
    </section>
  );
}
