#[allow(clippy::too_many_arguments)]
fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: &Receiver<TransportEffect>,
    broadcast_receiver: &Receiver<ProtocolFrame>,
    stop: &AtomicBool,
    status: &SharedStatus,
    clock: &Arc<dyn TransportClock>,
) -> Result<(), DesktopNetworkError> {
    let mut processor = HostTransportEventProcessor::new(Arc::clone(clock));
    let mut primary_error = None;
    while !stop.load(Ordering::Acquire) {
        if let Err(error) =
            process_effects(&*node, &**sink, effect_receiver, status, &mut processor)
        {
            primary_error = Some(error);
            break;
        }
        if let Err(error) = process_broadcast_frames(&*node, broadcast_receiver, status) {
            primary_error = Some(error);
            break;
        }
        let poll_interval = if status.broadcast.queue_depth.load(Ordering::Relaxed) > 0 {
            BACKLOG_POLL_INTERVAL
        } else {
            EVENT_POLL_INTERVAL
        };
        match node.recv_event(poll_interval) {
            Ok(event) => match processor.process(event, &*node, advertisement, &**sink) {
                Ok(Some(message)) => set_last_error(status, message)?,
                Ok(None) => {}
                Err(error) => {
                    set_last_error(status, error.to_string())?;
                    primary_error = Some(error);
                    break;
                }
            },
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => {
                let error = DesktopNetworkError::transport(&error);
                set_last_error(status, error.to_string())?;
                primary_error = Some(error);
                break;
            }
        }
    }

    let drain_error = fail_queued_effects(effect_receiver, &**sink, status).err();
    let shutdown_error = node
        .shutdown()
        .map_err(|error| DesktopNetworkError::transport(&error))
        .err();
    status.running.store(false, Ordering::Release);
    primary_error
        .or(drain_error)
        .or(shutdown_error)
        .map_or(Ok(()), Err)
}

/// Drains up to [`MAX_BROADCAST_FRAMES_PER_TICK`] frames queued by a
/// playback pump thread (stream-start control, audio datagrams) and
/// broadcasts each on the channel its `ProtocolFrame` variant belongs to.
/// A per-frame delivery failure is recorded as the last error but does not
/// stop the worker -- one dropped audio packet is not fatal to the stream,
/// matching the Android host's per-packet broadcast-audio handling.
fn process_broadcast_frames(
    node: &dyn HostTransportNode,
    receiver: &Receiver<ProtocolFrame>,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {
    for _ in 0..MAX_BROADCAST_FRAMES_PER_TICK {
        let frame = match receiver.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                return Err(DesktopNetworkError::unavailable(
                    "desktop host transport broadcast queue disconnected",
                ));
            }
        };
        status.broadcast.record_dequeued();
        let delivery = match &frame {
            ProtocolFrame::Control(message) => node.broadcast_control(message),
            ProtocolFrame::Audio(_) => node.broadcast_audio(&frame),
            ProtocolFrame::SyncResponse(_) => node.broadcast_sync(&frame),
            ProtocolFrame::SyncRequest(_) => continue, // the host never sends this frame kind
        };
        match delivery {
            Ok(delivery) => status.broadcast.record_delivery(&delivery),
            Err(error) => {
                status.broadcast.record_failure();
                set_last_error(status, DesktopNetworkError::transport(&error).to_string())?;
            }
        }
    }
    Ok(())
}

