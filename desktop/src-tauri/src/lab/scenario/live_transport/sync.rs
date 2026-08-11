use silent_disco_core::domain::{DeviceId, MonotonicMillis, SessionId};
use silent_disco_core::protocol::{
    ControlMessage, SyncRequest, SyncResponse, SynchronizationReport,
};
use silent_disco_core::runtime::SynchronizationSummary;
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};
use silent_disco_core::transport::ListenerTransportNode;

pub(super) struct LiveSyncState {
    session_id: SessionId,
    estimator: ClockSyncEstimator,
    next_correlation: u64,
}

impl LiveSyncState {
    pub(super) fn new(session_id: SessionId) -> Result<Self, String> {
        let estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            session_id,
            estimator,
            next_correlation: 1,
        })
    }

    pub(super) fn send_probe(
        &mut self,
        transport: &dyn ListenerTransportNode,
        send_time: MonotonicMillis,
    ) -> Result<(), String> {
        let correlation = self.next_correlation;
        self.next_correlation = self.next_correlation.saturating_add(1);
        self.estimator
            .begin_probe(
                SyncCorrelationId::new(correlation),
                LocalMonotonicMillis::from(send_time),
            )
            .map_err(|error| error.to_string())?;
        transport
            .send_sync_request(&SyncRequest {
                session_id: self.session_id.clone(),
                correlation_id: correlation,
                t1_listener_send_elapsed_ms: send_time,
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(super) fn observe_response(
        &mut self,
        listener_id: DeviceId,
        response: SyncResponse,
        received_at: MonotonicMillis,
    ) -> Result<(SynchronizationSummary, ControlMessage), String> {
        let SyncResponse {
            session_id,
            correlation_id,
            t1_listener_send_elapsed_ms,
            t2_host_receive_elapsed_ms,
            t3_host_send_elapsed_ms,
        } = response;
        if session_id != self.session_id {
            return Err("sync response session does not match the live Lab listener session".to_owned());
        }
        let observation = self
            .estimator
            .observe_response(
                SyncCorrelationId::new(correlation_id),
                LocalMonotonicMillis::from(t1_listener_send_elapsed_ms),
                HostMonotonicMillis::from(t2_host_receive_elapsed_ms),
                HostMonotonicMillis::from(t3_host_send_elapsed_ms),
                LocalMonotonicMillis::from(received_at),
            )
            .map_err(|error| error.to_string())?;
        let snapshot = observation.snapshot;
        let summary = SynchronizationSummary::new(
            snapshot.confidence,
            snapshot.offset_ms,
            snapshot.round_trip_time_ms,
            snapshot.skew_ppm,
        )
        .map_err(|error| error.to_string())?;
        let report = ControlMessage::SynchronizationReport(SynchronizationReport {
            session_id: self.session_id.clone(),
            listener_id,
            confidence: snapshot.confidence,
            offset_ms: snapshot.offset_ms,
            round_trip_ms: snapshot.round_trip_time_ms,
            drift_ppm: snapshot.skew_ppm,
        });
        Ok((summary, report))
    }
}
