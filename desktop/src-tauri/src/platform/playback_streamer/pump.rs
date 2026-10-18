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

fn boundary_schedule_event(
    boundary: &TrackBoundary,
    already_reserved: bool,
) -> Option<AudioEvent> {
    (!already_reserved).then(|| AudioEvent::TrackBoundaryScheduled {
        entry_id: boundary.entry_id.clone(),
        first_sample_index: boundary.first_sample_index.get(),
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
    let authoritative_boundaries = packetizer.boundaries_already_reserved();
    let mut last_reported_position_ms: Option<u64> = None;
    let mut streaming_error: Option<DesktopErrorDto> = None;
    let mut pending_boundaries: VecDeque<TrackBoundary> = VecDeque::new();
    let mut packetizer_disconnected = false;
    let mut last_audible_audio_end_ms = None;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if paused.load(Ordering::Acquire) {
            thread::sleep(PAUSE_POLL_INTERVAL);
            continue;
        }
        let now = match network.transport_now() {
            Ok(now) => now.get(),
            Err(error) => {
                streaming_error = Some(error);
                break;
            }
        };
        if let Err(error) = apply_due_boundaries(
            &mut pending_boundaries,
            host_start_time_ms,
            accumulated_pause_offset_ms.load(Ordering::Acquire),
            now,
            handle,
        ) {
            streaming_error = Some(error);
        }
        if streaming_error.is_some() {
            break;
        }
        if packetizer_disconnected && pending_boundaries.is_empty() {
            break;
        }
        if packetizer_disconnected {
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        let recv_timeout = if pending_boundaries.is_empty() {
            PUMP_RECV_TIMEOUT
        } else {
            Duration::from_millis(10)
        };
        match packetizer.recv_timeout(recv_timeout) {
            Ok(mut frame) => {
                // Stop can arrive while the packetizer receive is blocked.
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)) = &frame {
                    // A producer-reserved boundary is already serialized by
                    // the actor; do not resubmit a stale event after Stop.
                    if let Some(event) =
                        boundary_schedule_event(boundary, authoritative_boundaries)
                    {
                        if let Err(error) = handle.submit_audio_event(event) {
                            streaming_error = Some(DesktopErrorDto::from(error));
                            break;
                        }
                    }
                    pending_boundaries.push_back(boundary.clone());
                }
                // Capture content progress before applying the pause offset.
                // Do not report it until the network accepts the audio frame.
                let content_position_ms = frame_content_position_ms(&frame, host_start_time_ms);
                let mut applied_pause_offset_ms =
                    accumulated_pause_offset_ms.load(Ordering::Acquire);
                apply_pause_offset(&mut frame, applied_pause_offset_ms);
                // A Pause can win the network mutex *after* pacing decides
                // this frame is due. In that case keep it, wait for Resume,
                // and apply the additional pause offset before retrying.
                let accepted = loop {
                    match wait_until_within_send_ahead_horizon(
                        &mut frame,
                        network,
                        stop,
                        paused,
                        accumulated_pause_offset_ms,
                        &mut applied_pause_offset_ms,
                    ) {
                        Ok(true) => {}
                        Ok(false) => break false,
                        Err(error) => {
                            streaming_error = Some(error);
                            break false;
                        }
                    }
                    match broadcast_then_monitor(
                        &frame,
                        |frame| network.broadcast_playback_frame_unless_paused(frame, paused),
                        monitor_tap,
                        stop,
                    ) {
                        Ok(true) => break true,
                        Ok(false) if stop.load(Ordering::Acquire) => break false,
                        Ok(false) => continue,
                        Err(error) => {
                            streaming_error = Some(error);
                            break false;
                        }
                    }
                };
                if !accepted {
                    break;
                }
                let audio_end_ms = match &frame {
                    ProtocolFrame::Audio(audio) => Some(
                        audio.host_presentation_time_ms.get().saturating_add(
                            u64::from(audio.samples_per_packet)
                                .saturating_mul(1_000)
                                / u64::from(audio.sample_rate),
                        ),
                    ),
                    _ => None,
                };
                if let Some(audio_end_ms) = audio_end_ms {
                    last_audible_audio_end_ms = Some(audio_end_ms);
                }
                report_position_if_due(
                    content_position_ms,
                    &mut last_reported_position_ms,
                    handle,
                    &stream_id,
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => packetizer_disconnected = true,
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
    let terminal_failure = streaming_error
        .as_ref()
        .map(|error| error.message.clone())
        .or_else(|| packetizer_result.as_ref().err().map(|error| error.message.clone()));
    // Both a natural queue end and a late failure leave already-sent audio
    // still ahead of its presentation time (up to the send-ahead horizon plus
    // the startup buffer). `Stop` and the end/failure report must wait for the
    // last valid packet to become audible, or listeners cut off the tail of
    // the final song and the host reports the queue finished early.
    let failure_tail_result = if (ended_naturally || terminal_failure.is_some())
        && !stop.load(Ordering::Acquire)
    {
        last_audible_audio_end_ms
            .map(|deadline| wait_until_audio_tail(deadline, network, stop))
            .transpose()
            .map(|_| ())
    } else {
        Ok(())
    };
    let broadcast_result = network.transport_now().and_then(|host_stop_time_ms| {
        network.broadcast_playback_frame(ProtocolFrame::Control(ControlMessage::Stop(Stop {
            session_id,
            stream_id,
            host_stop_time_ms,
        })))
    });
    // A fully packetized source is not proof of successful playback: the
    // transport can fail while already-queued frames are being delivered.
    // Likewise, explicit Stop must preserve the queue even if the worker
    // completed while the sender was draining its bounded output channel.
    let stopped_result = handle
        .submit_audio_event(terminal_audio_event(
            ending_stream_id,
            stop.load(Ordering::Acquire),
            ended_naturally,
            terminal_failure.as_deref(),
        ))
        .map_err(DesktopErrorDto::from);
    if let Some(primary) = streaming_error {
        return Err(primary
            .with_appended_cleanup(monitor_result.err())
            .with_appended_cleanup(packetizer_result.err())
            .with_appended_cleanup(failure_tail_result.err())
            .with_appended_cleanup(broadcast_result.err())
            .with_appended_cleanup(stopped_result.err()));
    }
    monitor_result
        .and(packetizer_result)
        .and(failure_tail_result)
        .and(broadcast_result)
        .and(stopped_result)
}

fn wait_until_audio_tail(
    deadline_ms: u64,
    network: &DesktopHostNetworkControl,
    stop: &AtomicBool,
) -> Result<(), DesktopErrorDto> {
    while !stop.load(Ordering::Acquire) {
        let now = network.transport_now()?.get();
        if now >= deadline_ms {
            break;
        }
        thread::sleep(Duration::from_millis(deadline_ms.saturating_sub(now).min(10)));
    }
    Ok(())
}

/// Selects the actor's terminal event from actual delivery, not merely the
/// packetizer's completion. A transport failure takes precedence over EOF,
/// and an explicit Stop takes precedence over an otherwise clean EOF.
fn terminal_audio_event(
    stream_id: StreamId,
    stop_requested: bool,
    packetizer_completed: bool,
    failure: Option<&str>,
) -> AudioEvent {
    if let Some(message) = failure {
        AudioEvent::Failed(playback_failure(message))
    } else if stop_requested {
        AudioEvent::PlaybackStateChanged(PlaybackState::Stopped)
    } else if packetizer_completed {
        AudioEvent::EndOfStream { stream_id }
    } else {
        AudioEvent::PlaybackStateChanged(PlaybackState::Stopped)
    }
}

fn playback_failure(message: &str) -> CoreError {
    let detail = message
        .chars()
        .filter(|character| !character.is_control())
        .take(384)
        .collect::<String>();
    CoreError::new(
        CoreErrorCode::AudioOutputFailed,
        format!("Playlist playback stopped: {detail}"),
        ErrorSeverity::Error,
        true,
        None,
    )
    .unwrap_or_else(|_| {
        CoreError::new(
            CoreErrorCode::AudioOutputFailed,
            "Playlist playback stopped after an audio failure",
            ErrorSeverity::Error,
            true,
            None,
        )
        .expect("static playback failure is valid")
    })
}

fn apply_due_boundaries(
    pending: &mut VecDeque<TrackBoundary>,
    host_start_time_ms: u64,
    pause_offset_ms: u64,
    now_ms: u64,
    handle: &CoreActorHandle,
) -> Result<(), DesktopErrorDto> {
    while pending.front().is_some_and(|boundary| {
        let boundary_time_ms = host_start_time_ms
            .saturating_add(
                boundary.first_sample_index.get().saturating_mul(1_000)
                    / u64::from(CANONICAL_SAMPLE_RATE_HZ),
            )
            .saturating_add(pause_offset_ms);
        boundary_time_ms <= now_ms
    }) {
        let boundary = pending.pop_front().expect("front was present");
        handle
            .submit_audio_event(AudioEvent::TrackBoundaryApplied {
                entry_id: boundary.entry_id,
                first_sample_index: boundary.first_sample_index.get(),
            })
            .map_err(DesktopErrorDto::from)?;
    }
    Ok(())
}

/// Compute the unshifted content position before pause correction.
/// Non-audio control frames never advance the host's progress indicator.
fn frame_content_position_ms(frame: &ProtocolFrame, host_start_time_ms: u64) -> Option<u64> {
    match frame {
        ProtocolFrame::Audio(datagram) => Some(
            datagram
                .host_presentation_time_ms
                .get()
                .saturating_sub(host_start_time_ms),
        ),
        _ => None,
    }
}

/// Submit a throttled `PositionAdvanced` event only after the network
/// accepts the corresponding audio packet. Failed/aborted sends must not
/// advance the authoritative UI position. A failed actor notification is
/// advisory and is retried on the next due accepted frame.
fn report_position_if_due(
    position_ms: Option<u64>,
    last_reported_position_ms: &mut Option<u64>,
    handle: &CoreActorHandle,
    stream_id: &StreamId,
) {
    let Some(position_ms) = position_ms else {
        return;
    };
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

/// Hand a frame to the network first, and only mirror successfully accepted
/// audio to the best-effort local monitor. The monitor must not play a frame
/// rejected by network delivery, and Stop between pacing and handoff must
/// suppress both outputs. This does not promise remote acknowledgement.
fn broadcast_then_monitor<F>(
    frame: &ProtocolFrame,
    broadcast: F,
    monitor_tap: Option<&SyncSender<AudioDatagram>>,
    stop: &AtomicBool,
) -> Result<bool, DesktopErrorDto>
where
    F: FnOnce(&ProtocolFrame) -> Result<bool, DesktopErrorDto>,
{
    if stop.load(Ordering::Acquire) {
        return Ok(false);
    }
    // The caller retains ownership when Pause defers the network handoff.
    // Never mirror a frame that the transport did not accept.
    if !broadcast(frame)? {
        return Ok(false);
    }
    forward_to_monitor_if_running(frame, monitor_tap, stop);
    Ok(true)
}

/// Forward only after a successful network handoff. A full or disconnected
/// monitor is best-effort and must never block the broadcast path.
/// Both outputs share the pause-adjusted presentation timestamp.
fn forward_to_monitor_if_running(
    frame: &ProtocolFrame,
    monitor_tap: Option<&SyncSender<AudioDatagram>>,
    stop: &AtomicBool,
) {
    if !stop.load(Ordering::Acquire) {
        forward_to_monitor(frame, monitor_tap);
    }
}

fn forward_to_monitor(frame: &ProtocolFrame, monitor_tap: Option<&SyncSender<AudioDatagram>>) {
    let (Some(tap), ProtocolFrame::Audio(datagram)) = (monitor_tap, frame) else {
        return;
    };
    drop(tap.try_send(datagram.clone()));
}

/// Holds a frame until it is within the send-ahead horizon, also holding it
/// across Pause. A frame that was already dequeued before Pause must receive
/// the *additional* pause offset on Resume, not just frames dequeued later.
fn wait_until_within_send_ahead_horizon(
    frame: &mut ProtocolFrame,
    network: &Arc<DesktopHostNetworkControl>,
    stop: &AtomicBool,
    paused: &AtomicBool,
    accumulated_pause_offset_ms: &AtomicU64,
    applied_pause_offset_ms: &mut u64,
) -> Result<bool, DesktopErrorDto> {
    wait_until_within_send_ahead_horizon_with_clock(
        frame,
        || network.transport_now().map(MonotonicMillis::get),
        stop,
        paused,
        accumulated_pause_offset_ms,
        applied_pause_offset_ms,
    )
}

/// The injected transport clock permits deterministic pause/Stop regressions.
/// No lock is held across the pause poll or while the network clock is read.
fn wait_until_within_send_ahead_horizon_with_clock(
    frame: &mut ProtocolFrame,
    mut now_ms: impl FnMut() -> Result<u64, DesktopErrorDto>,
    stop: &AtomicBool,
    paused: &AtomicBool,
    accumulated_pause_offset_ms: &AtomicU64,
    applied_pause_offset_ms: &mut u64,
) -> Result<bool, DesktopErrorDto> {
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        if paused.load(Ordering::Acquire) {
            thread::sleep(PAUSE_POLL_INTERVAL);
            continue;
        }
        // Resume publishes the accumulated offset before clearing Paused.
        // Shift this in-flight frame only by what it has not yet received.
        let latest_offset_ms = accumulated_pause_offset_ms.load(Ordering::Acquire);
        if latest_offset_ms > *applied_pause_offset_ms {
            apply_pause_offset(frame, latest_offset_ms - *applied_pause_offset_ms);
            *applied_pause_offset_ms = latest_offset_ms;
        }
        if paused.load(Ordering::Acquire) {
            continue;
        }
        let ProtocolFrame::Audio(datagram) = frame else {
            return Ok(true);
        };
        let now = now_ms()?;
        // Pause can arrive during the transport-clock lookup.
        if paused.load(Ordering::Acquire) {
            continue;
        }
        let lead_ms = datagram.host_presentation_time_ms.get().saturating_sub(now);
        if lead_ms <= SEND_AHEAD_HORIZON_MS {
            return Ok(true);
        }
        thread::sleep(SEND_AHEAD_POLL_INTERVAL);
    }
}
use std::collections::VecDeque;

use silent_disco_core::audio::CANONICAL_SAMPLE_RATE_HZ;
use silent_disco_core::protocol::TrackBoundary;
