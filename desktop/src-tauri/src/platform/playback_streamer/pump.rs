#[allow(clippy::too_many_arguments)]
fn spawn_pump(
    packetizer: StreamingPacketizeHandle,
    network: Arc<DesktopHostNetworkControl>,
    handle: CoreActorHandle,
    session_id: SessionId,
    stream_id: StreamId,
    host_start_time_ms: u64,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    accumulated_pause_offset_ms: Arc<AtomicU64>,
    monitor_tap: Option<SyncSender<AudioDatagram>>,
) -> Result<JoinHandle<Result<(), DesktopErrorDto>>, DesktopErrorDto> {
    thread::Builder::new()
        .name("silent-disco-desktop-playback-pump".to_owned())
        .spawn(move || {
            run_pump(
                packetizer,
                &network,
                &handle,
                session_id,
                stream_id,
                host_start_time_ms,
                &stop,
                &paused,
                &accumulated_pause_offset_ms,
                monitor_tap.as_ref(),
            )
        })
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.playback.pump_start_failed",
                "audio",
                "error",
                true,
                &format!("failed to start desktop playback pump: {error}"),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn run_pump(
    packetizer: StreamingPacketizeHandle,
    network: &Arc<DesktopHostNetworkControl>,
    handle: &CoreActorHandle,
    session_id: SessionId,
    stream_id: StreamId,
    host_start_time_ms: u64,
    stop: &AtomicBool,
    paused: &AtomicBool,
    accumulated_pause_offset_ms: &AtomicU64,
    monitor_tap: Option<&SyncSender<AudioDatagram>>,
) -> Result<(), DesktopErrorDto> {
    let mut last_reported_position_ms: Option<u64> = None;
    let mut streaming_error: Option<DesktopErrorDto> = None;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if paused.load(Ordering::Acquire) {
            thread::sleep(PAUSE_POLL_INTERVAL);
            continue;
        }
        match packetizer.recv_timeout(PUMP_RECV_TIMEOUT) {
            Ok(mut frame) => {
                // Position must reflect actual song content progress, which
                // is exactly what the packetizer's own (unshifted) sequence
                // math already gives -- compute it before the pause offset
                // below inflates the frame's presentation time by however
                // long the stream has spent paused so far.
                report_position_if_due(
                    &frame,
                    host_start_time_ms,
                    &mut last_reported_position_ms,
                    handle,
                    &stream_id,
                );
                apply_pause_offset(
                    &mut frame,
                    accumulated_pause_offset_ms.load(Ordering::Acquire),
                );
                forward_to_monitor(&frame, monitor_tap);
                match wait_until_within_send_ahead_horizon(&frame, network, stop) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        streaming_error = Some(error);
                        break;
                    }
                }
                if let Err(error) = network.broadcast_playback_frame(frame) {
                    streaming_error = Some(error);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // Tearing the monitor down is attempted unconditionally, exactly like
    // every other shutdown step below -- and, like them, is never allowed
    // to prevent listeners being told the stream ended or the actor
    // leaving `Playing`. Ordered before the packetizer cancellation only so
    // the monitor's own thread/device stop first, not because anything
    // downstream depends on that order.
    let monitor_result = network.monitor.on_stream_stopped().map_err(|message| {
        DesktopErrorDto::new(
            "desktop.playback.monitor_shutdown_failed",
            "audio",
            "error",
            false,
            &message,
        )
    });
    // Every shutdown step is attempted even when an earlier one fails -- a
    // packetizer that will not cancel must not prevent the listeners being
    // told the stream ended, nor the actor leaving `Playing` -- and the first
    // failure is what gets reported.
    let packetizer_summary = packetizer.cancel_and_join();
    // The packetizer only ever reaches `Ok` by emitting its final packet and
    // exiting on its own -- every other exit, cancellation included, is an
    // `Err`. So `Ok` here means the source genuinely finished, distinct from
    // being told to stop, and that distinction is worth keeping visible to a
    // playback UI rather than folding both into the same generic `Stopped`.
    let ended_naturally = packetizer_summary.is_ok();
    let packetizer_result = match packetizer_summary {
        Ok(_) => Ok(()),
        // Cancellation is precisely what stopping asks the worker to do, so it
        // is this path's normal outcome rather than a failure. Every other kind
        // -- a decode failure, a packetize failure, a panicking worker -- is
        // real and must not be reported as a clean stop.
        Err(error) if error.kind == PacketizerWorkerErrorKind::Cancelled => Ok(()),
        Err(error) => Err(DesktopErrorDto::new(
            "desktop.playback.packetizer_shutdown_failed",
            "audio",
            "error",
            false,
            &error.message,
        )),
    };
    let ending_stream_id = stream_id.clone();
    let broadcast_result = network.transport_now().and_then(|host_stop_time_ms| {
        network.broadcast_playback_frame(ProtocolFrame::Control(ControlMessage::Stop(Stop {
            session_id,
            stream_id,
            host_stop_time_ms,
        })))
    });
    let stopped_result = if ended_naturally {
        handle.submit_audio_event(AudioEvent::EndOfStream {
            stream_id: ending_stream_id,
        })
    } else {
        handle.submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
    }
    .map_err(DesktopErrorDto::from);
    if let Some(primary) = streaming_error {
        return Err(primary
            .with_appended_cleanup(monitor_result.err())
            .with_appended_cleanup(packetizer_result.err())
            .with_appended_cleanup(broadcast_result.err())
            .with_appended_cleanup(stopped_result.err()));
    }
    monitor_result
        .and(packetizer_result)
        .and(broadcast_result)
        .and(stopped_result)
}

/// Submits a throttled `PositionAdvanced` event for one emitted audio frame.
///
/// Position is computed from the frame's own presentation time against the
/// stream's start -- the authoritative timeline this stream is scheduled
/// against -- rather than from wall-clock elapsed time, which would drift
/// under pause or send-ahead bursting. A failed submission is advisory-grade,
/// the same severity as the audio broadcast just below this call in the
/// caller, and is handled the same way: retried on the next due frame rather
/// than treated as a reason to stop the stream.
fn report_position_if_due(
    frame: &ProtocolFrame,
    host_start_time_ms: u64,
    last_reported_position_ms: &mut Option<u64>,
    handle: &CoreActorHandle,
    stream_id: &StreamId,
) {
    let ProtocolFrame::Audio(datagram) = frame else {
        return;
    };
    let position_ms = datagram
        .host_presentation_time_ms
        .get()
        .saturating_sub(host_start_time_ms);
    let due = match *last_reported_position_ms {
        Some(previous) => position_ms >= previous.saturating_add(POSITION_REPORT_INTERVAL_MS),
        None => true,
    };
    if !due {
        return;
    }
    if handle
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: stream_id.clone(),
            position_ms,
        })
        .is_ok()
    {
        *last_reported_position_ms = Some(position_ms);
    }
}

/// Adds the stream's accumulated pause offset to one outgoing audio frame's
/// presentation time. The packetizer computes `host_presentation_time_ms`
/// from a fixed anchor set once at stream start, so it has no way to know
/// real time kept moving while the pump stopped draining it for a pause --
/// every frame it produces from then on reads as further and further behind
/// schedule. Left uncorrected, [`wait_until_within_send_ahead_horizon`]
/// reads that lag as "already late", disabling the send-ahead throttle and
/// bursting the whole backlog at once, which is what overwhelmed the
/// broadcast queue on a real device after a pause/resume. Non-audio frames
/// are untouched; a zero offset (the common case, stream never paused) is a
/// no-op.
fn apply_pause_offset(frame: &mut ProtocolFrame, offset_ms: u64) {
    if offset_ms == 0 {
        return;
    }
    if let ProtocolFrame::Audio(datagram) = frame {
        datagram.host_presentation_time_ms = MonotonicMillis::new(
            datagram
                .host_presentation_time_ms
                .get()
                .saturating_add(offset_ms),
        );
    }
}

/// Forwards one outgoing audio frame's datagram to the local monitor pump,
/// if a monitor is currently active for this stream (Block 34).
///
/// Best-effort and non-blocking (`try_send`): a monitor that cannot keep up
/// simply misses frames rather than ever slowing or blocking the network
/// broadcast path below this call, which must never be affected by monitor
/// health (34.2 policy). Runs after [`apply_pause_offset`] so the monitor's
/// own scheduler sees the same pause-corrected timeline the network
/// broadcast does, and before the send-ahead wait, since the monitor has no
/// use for that network-specific pacing at all.
fn forward_to_monitor(frame: &ProtocolFrame, monitor_tap: Option<&SyncSender<AudioDatagram>>) {
    let (Some(tap), ProtocolFrame::Audio(datagram)) = (monitor_tap, frame) else {
        return;
    };
    drop(tap.try_send(datagram.clone()));
}

/// Blocks, in short stop-responsive increments, until `frame`'s presentation
/// time is no more than [`SEND_AHEAD_HORIZON_MS`] ahead of the transport's
/// current time. Non-audio frames (there are none on this path today, but
/// [`StreamingPacketizeHandle::recv_timeout`] returns `ProtocolFrame`) pass
/// through immediately. Returns `false` if `stop` fired while waiting, so
/// the caller can exit without sending a stale frame.
fn wait_until_within_send_ahead_horizon(
    frame: &ProtocolFrame,
    network: &Arc<DesktopHostNetworkControl>,
    stop: &AtomicBool,
) -> Result<bool, DesktopErrorDto> {
    let ProtocolFrame::Audio(datagram) = frame else {
        return Ok(true);
    };
    let presentation_time_ms = datagram.host_presentation_time_ms.get();
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        let now = network.transport_now()?;
        let lead_ms = presentation_time_ms.saturating_sub(now.get());
        if lead_ms <= SEND_AHEAD_HORIZON_MS {
            return Ok(true);
        }
        thread::sleep(SEND_AHEAD_POLL_INTERVAL);
    }
}
