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
    let mut approved_listener: Option<(
        silent_disco_core::domain::DeviceId,
        bool,
        silent_disco_core::domain::SessionId,
    )> = None;
    let delivery = match effect.request {
        TransportEffectRequest::DeliverJoinApproval {
            session_id,
            listener_id,
            trusted_for_future,
            ..
        } => {
            // Legacy v3 TCP is plaintext and has no listener key proof. A
            // claimed device ID must never become durable Trusted identity.
            // Refuse this effect before any JoinApproval or UDP authorization;
            // the Rust actor receives a failed delivery and the listener sees
            // a visible rejection rather than a misleading success.
            if trusted_for_future {
                refuse_legacy_durable_trust(
                    node, sink, status, processor, operation_id, session_id, &listener_id,
                )?;
                return Ok(());
            }
            approved_listener = Some((listener_id.clone(), false, session_id.clone()));
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

    finish_delivery(
        node,
        sink,
        status,
        processor,
        operation_id,
        delivery,
        approved_listener,
    )
}

/// Reports an actor success only after the native host authorized this
/// listener's UDP routes. Non-approval effects retain their original result.
fn finish_delivery(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    status: &SharedStatus,
    processor: &mut HostTransportEventProcessor,
    operation_id: silent_disco_core::domain::OperationId,
    delivery: Result<
        silent_disco_core::transport::TransportDelivery,
        silent_disco_core::transport::TransportError,
    >,
    approved_listener: Option<(
        silent_disco_core::domain::DeviceId,
        bool,
        silent_disco_core::domain::SessionId,
    )>,
) -> Result<(), DesktopNetworkError> {
    let mut report = match delivery {
        Ok(delivery) => delivery.report,
        Err(error) => {
            set_last_error(status, error.to_string())?;
            failed_delivery_report()
        }
    };
    let mut connected = None;
    if report.successful_peers > 0
        && let Some((listener_id, trusted_for_future, session_id)) = approved_listener
    {
        connected = finish_approved_listener(
            node,
            status,
            processor,
            listener_id,
            trusted_for_future,
            session_id,
        )?;
        if connected.is_none() {
            report = failed_delivery_report();
        }
    }
    // Commit the actor's decision only AFTER native route authorization.
    // Keep this event before ListenerConnected, as required by Rust's
    // admission lifecycle. An unsuccessful route never counts as delivery.
    submit_delivery(sink, operation_id, report)?;
    if let Some(listener) = connected {
        sink.submit_transport_event(CoreTransportEvent::ListenerConnected(listener))
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
    }

    Ok(())
}

/// A TCP approval becomes a successful actor decision only when its
/// reported UDP routes have also been authorized on the SAME host generation.
/// A failure leaves the old listener route untouched and yields no new
/// connected-listener projection.
fn finish_approved_listener(
    node: &dyn HostTransportNode,
    status: &SharedStatus,
    processor: &mut HostTransportEventProcessor,
    listener_id: silent_disco_core::domain::DeviceId,
    trusted_for_future: bool,
    session_id: silent_disco_core::domain::SessionId,
) -> Result<Option<silent_disco_core::runtime::ListenerSummary>, DesktopNetworkError> {
    let Some((sync_port, audio_port, display_name, control_address)) =
        processor.take_pending_listener(&listener_id)
    else {
        set_last_error(
            status,
            "JoinApproval reached TCP but no matching pending listener route exists".to_owned(),
        )?;
        return Ok(None);
    };
    if let Err(error) = node.authorize_peer_ports(&listener_id, sync_port, audio_port) {
        set_last_error(status, DesktopNetworkError::transport(&error).to_string())?;
        // TCP JoinApproval was written, but no UDP route is authorized.
        // Notify the listener and fail the actor decision rather than
        // creating a ghost connected listener.
        let disconnect = ControlMessage::Disconnect(Disconnect {
            session_id,
            listener_id: listener_id.clone(),
            reason: "Listener audio route authorization failed".to_owned(),
        });
        if let Err(error) = node.send_pending_control(&listener_id, &disconnect) {
            set_last_error(
                status,
                format!("Admission rollback notification failed: {error}"),
            )?;
        }
        return Ok(None);
    }
    processor.mark_authorized_listener(listener_id.clone(), control_address);
    HostTransportEventProcessor::connected_listener(
        listener_id,
        display_name,
        trusted_for_future,
    )
    .map(Some)
    .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))
}

/// Refuse an unverified durable-trust request before any admission or route
/// authorization. Legacy v3 may grant only one-session approval.
#[allow(clippy::too_many_arguments)]
fn refuse_legacy_durable_trust(
    node: &dyn HostTransportNode,
    sink: &dyn DesktopHostTransportEventSink,
    status: &SharedStatus,
    processor: &mut HostTransportEventProcessor,
    operation_id: silent_disco_core::domain::OperationId,
    session_id: silent_disco_core::domain::SessionId,
    listener_id: &silent_disco_core::domain::DeviceId,
) -> Result<(), DesktopNetworkError> {
    processor.take_pending_ports(listener_id);
    set_last_error(
        status,
        "Legacy v3 cannot remember an unverified listener identity".to_owned(),
    )?;
    let rejection = ControlMessage::JoinRejection(JoinRejection {
        session_id,
        listener_id: listener_id.clone(),
        reason: "Secure listener identity verification is required for durable trust".to_owned(),
    });
    if let Err(error) = node.send_pending_control(listener_id, &rejection) {
        set_last_error(status, error.to_string())?;
    }
    submit_delivery(sink, operation_id, failed_delivery_report())
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
