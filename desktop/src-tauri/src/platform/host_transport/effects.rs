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
    let mut approved_listener: Option<(silent_disco_core::domain::DeviceId, bool)> = None;
    let delivery = match effect.request {
        TransportEffectRequest::DeliverJoinApproval {
            session_id,
            listener_id,
            trusted_for_future,
            ..
        } => {
            approved_listener = Some((listener_id.clone(), trusted_for_future));
            eprintln!(
                "join_approval_send_started operation_id={operation_id} send_started_unix_ms={}",
                unix_time_ms()
            );
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

    let report = match delivery {
        Ok(delivery) => delivery.report,
        Err(error) => {
            set_last_error(status, error.to_string())?;
            failed_delivery_report()
        }
    };
    submit_delivery(sink, operation_id, report)?;

    if report.successful_peers > 0
        && let Some((listener_id, trusted_for_future)) = approved_listener
        && let Some((sync_port, audio_port, display_name, control_address)) =
            processor.take_pending_listener(&listener_id)
    {
        if let Err(error) = node.authorize_peer_ports(&listener_id, sync_port, audio_port) {
            set_last_error(status, DesktopNetworkError::transport(&error).to_string())?;
            return Ok(());
        }
        processor.mark_authorized_listener(listener_id.clone(), control_address);
        let listener = HostTransportEventProcessor::connected_listener(
            listener_id,
            display_name,
            trusted_for_future,
        )
        .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        sink.submit_transport_event(CoreTransportEvent::ListenerConnected(listener))
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
    }

    Ok(())
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
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
