fn is_faultable_datagram(event: &TransportEvent) -> bool {
    matches!(
        event,
        TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization | TransportChannel::Audio,
            ..
        }
    )
}

fn event_channel_and_time(event: &TransportEvent) -> (Option<TransportChannel>, u64) {
    match event {
        TransportEvent::FrameReceived {
            channel,
            received_at,
            ..
        } => (Some(*channel), received_at.get()),
        TransportEvent::PeerAccepted { received_at, .. }
        | TransportEvent::PeerAuthorized { received_at, .. }
        | TransportEvent::PeerDisconnected { received_at, .. }
        | TransportEvent::Rejected { received_at, .. } => (None, received_at.get()),
    }
}

fn should_drop_event(
    event: &TransportEvent,
    loss_permille: u16,
    synchronization_prng: &mut DeterministicPrng,
    audio_prng: &mut DeterministicPrng,
) -> bool {
    let (channel, _) = event_channel_and_time(event);
    match channel {
        Some(channel) => {
            should_drop_channel(channel, loss_permille, synchronization_prng, audio_prng)
        }
        None => false,
    }
}

fn should_drop_channel(
    channel: TransportChannel,
    loss_permille: u16,
    synchronization_prng: &mut DeterministicPrng,
    audio_prng: &mut DeterministicPrng,
) -> bool {
    if loss_permille == 0 {
        return false;
    }
    let prng = match channel {
        TransportChannel::Synchronization => synchronization_prng,
        TransportChannel::Audio => audio_prng,
        TransportChannel::Control | TransportChannel::Runtime => return false,
    };
    prng.next_permille() < loss_permille
}

fn compute_deadline(
    config: LabLatencyConfig,
    prng: &mut DeterministicPrng,
    arrived_at_ms: u64,
) -> u64 {
    let jitter_offset = if config.jitter_ms == 0 {
        0
    } else {
        let span = config.jitter_ms.saturating_mul(2).saturating_add(1);
        let raw = prng.next_below(usize::try_from(span).unwrap_or(usize::MAX));
        i64::try_from(raw).unwrap_or(0) - i64::try_from(config.jitter_ms).unwrap_or(0)
    };
    let base = i64::try_from(arrived_at_ms).unwrap_or(i64::MAX)
        + i64::try_from(config.fixed_latency_ms).unwrap_or(i64::MAX)
        + jitter_offset;
    u64::try_from(base.max(0)).unwrap_or(u64::MAX)
}

fn take_due(queue: &mut BinaryHeap<Reverse<HeldEvent>>, now_ms: u64) -> Option<HeldEvent> {
    let is_due = matches!(queue.peek(), Some(Reverse(held)) if held.deadline_ms <= now_ms);
    if !is_due {
        return None;
    }
    queue.pop().map(|Reverse(held)| held)
}
