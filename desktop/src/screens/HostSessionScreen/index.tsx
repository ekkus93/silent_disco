import { ConnectionDetailsPanel } from "./ConnectionDetailsPanel";
import { DeliveryAndTransportAlerts } from "./DeliveryAndTransportAlerts";
import { ListenerList } from "./ListenerList";
import { PendingJoinRequests } from "./PendingJoinRequests";
import { PlaybackControls } from "./PlaybackControls";
import { QrInvitationPanel } from "./QrInvitationPanel";
import { SessionHeaderAndStatus } from "./SessionHeaderAndStatus";
import { ErrorAlert } from "./shared";
import { useHostSessionViewModel } from "./useHostSessionViewModel";

export function HostSessionScreen() {
  const vm = useHostSessionViewModel();
  const { snapshot } = vm;

  if (!snapshot) {
    return (
      <main className="mx-auto max-w-6xl p-6 text-slate-100">
        <h1 className="text-2xl font-semibold">Host session</h1>
        <p className="mt-3">Loading authoritative host state…</p>
        {vm.refreshFailure ? <ErrorAlert error={vm.refreshFailure} /> : null}
      </main>
    );
  }

  return (
    <main className="mx-auto max-w-6xl p-6 text-slate-100">
      <SessionHeaderAndStatus
        sessionName={snapshot.sessionName}
        revision={snapshot.revision}
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

      <ConnectionDetailsPanel connection={snapshot.connection} onCopy={vm.copyValue} />

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
        selectedListenerId={vm.selectedListenerId}
        onSelectListener={vm.setSelectedListenerId}
        lastDelivery={snapshot.lastDelivery}
        removalOperations={vm.removalOperations}
        removalFailures={vm.removalFailures}
        onRequestRemoval={vm.requestRemoval}
      />

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

      <DeliveryAndTransportAlerts
        lastDelivery={snapshot.lastDelivery}
        broadcast={snapshot.broadcast}
        transportError={snapshot.transportError}
        lastError={snapshot.lastError}
        refreshFailure={vm.refreshFailure}
      />
    </main>
  );
}
