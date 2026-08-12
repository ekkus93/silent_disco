fn process_effects(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    receiver: &Receiver<TransportEffect>,
    status: &SharedStatus,
    processor: &mut HostTransportEventProcessor,
) -> Result<(), DesktopNetworkError> {
    for _ in 0..MAX_EFFECTS_PER_TICK {
        match receiver.try_recv() {
            Ok(effect) => process_effect(node, sink, effect, status, processor)?,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                return Err(DesktopNetworkError::unavailable(
                    "desktop host transport effect queue disconnected",
                ));
            }
        }
    }
    Ok(())
}

fn process_effect(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    effect: TransportEffect,
    status: &SharedStatus,
    processor: &mut HostTransportEventProcessor,
) -> Result<(), DesktopNetworkError> {
    let operation_id = effect.operation_id;
    let mut authorize: Option<(silent_disco_core::domain::DeviceId, u16, u16)> = None;
    let delivery = match effect.request {
        TransportEffectRequest::DeliverJoinApproval {
            session_id,
            listener_id,
            trusted_for_future,
            ..
        } => {
            if let Some((sync_port, audio_port)) = processor.take_pending_ports(&listener_id) {
                authorize = Some((listener_id.clone(), sync_port, audio_port));
            }
            node.send_pending_control(
                &listener_id.clone(),
                &ControlMessage::JoinApproval(JoinApproval {
                    session_id,
                    listener_id,
                    trusted_for_future,
                }),
            )
        }
        TransportEffectRequest::DeliverJoinRejection {
            session_id,
            listener_id,
            reason_code,
            ..
        } => {
            processor.take_pending_ports(&listener_id);
            node.send_pending_control(
                &listener_id.clone(),
                &ControlMessage::JoinRejection(JoinRejection {
                    session_id,
                    listener_id,
                    reason: reason_code,
                }),
            )
        }
        TransportEffectRequest::DisconnectListener {
            session_id,
            listener_id,
            reason_code,
        } => {
            processor.take_pending_ports(&listener_id);
            node.send_pending_control(
                &listener_id.clone(),
                &ControlMessage::Disconnect(Disconnect {
                    session_id,
                    listener_id,
                    reason: reason_code,
                }),
            )
        }
    };

    if let (Ok(delivery), Some((listener_id, sync_port, audio_port))) = (&delivery, authorize)
        && delivery.report.successful_peers > 0
        && let Err(error) = node.authorize_peer_ports(&listener_id, sync_port, audio_port)
    {
        set_last_error(status, DesktopNetworkError::transport(&error).to_string())?;
    }

    let report = match delivery {
        Ok(delivery) => delivery.report,
        Err(error) => {
            set_last_error(status, error.to_string())?;
            failed_delivery_report()
        }
    };
    submit_delivery(sink, operation_id, report)
}

fn submit_delivery(
    sink: &dyn DesktopHostTransportEventSink,
    operation_id: silent_disco_core::domain::OperationId,
    report: DeliveryReport,
) -> Result<(), DesktopNetworkError> {
    sink.submit_transport_event(CoreTransportEvent::DeliveryCompleted {
        operation_id,
        report,
    })
    .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))
}

fn fail_queued_effects(
    receiver: &Receiver<TransportEffect>,
    sink: &dyn DesktopHostTransportEventSink,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    loop {
        match receiver.try_recv() {
            Ok(effect) => {
                set_last_error(
                    status,
                    "transport effect cancelled during host transport shutdown".to_owned(),
                )?;
                submit_delivery(sink, effect.operation_id, failed_delivery_report())?;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

const fn failed_delivery_report() -> DeliveryReport {
    DeliveryReport {
        intended_peers: 1,
        successful_peers: 0,
        failed_peers: 1,
        severity: DeliverySeverity::PartialFailure,
    }
}

fn set_last_error(status: &SharedStatus, message: String) -> Result<(), DesktopNetworkError> {
    *status.last_error.lock().map_err(|_| {
        DesktopNetworkError::invalid_state("desktop host transport status mutex was poisoned")
    })? = Some(message);
    Ok(())
}
