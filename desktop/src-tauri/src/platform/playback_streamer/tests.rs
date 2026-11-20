#[cfg(test)]
mod tests {
    use super::{
        apply_pause_offset, boundary_schedule_event, broadcast_then_monitor,
        forward_to_monitor, forward_to_monitor_if_running, frame_content_position_ms,
        implicit_join_failure, terminal_audio_event,
        wait_until_within_send_ahead_horizon_with_clock,
    };
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{
        AudioCodec, AudioDatagram, ControlMessage, Disconnect, ProtocolFrame, TrackBoundary,
    };
    use silent_disco_core::runtime::AudioEvent;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;


    #[test]
    fn producer_reserved_boundary_does_not_trigger_duplicate_actor_event() {
        let boundary = TrackBoundary {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            stream_id: terminal_stream_id(),
            first_sample_index: SampleIndex::new(480),
            entry_id: "next-entry".to_owned(),
            display_name: "Next track".to_owned(),
        };
        assert!(boundary_schedule_event(&boundary, true).is_none());
        assert!(matches!(
            boundary_schedule_event(&boundary, false),
            Some(AudioEvent::TrackBoundaryScheduled {
                entry_id,
                first_sample_index: 480
            }) if entry_id == "next-entry"
        ));
    }

    fn terminal_stream_id() -> StreamId {
        StreamId::new("stream-pump-terminal-test").expect("stream id")
    }

    #[test]
    fn transport_failure_after_packetizer_completion_is_not_natural_queue_end() {
        let event = terminal_audio_event(
            terminal_stream_id(),
            false,
            true,
            Some("boundary broadcast failed"),
        );
        assert!(
            matches!(event, AudioEvent::Failed(_)),
            "a packetizer that finished ahead of the sender must not hide transport failure"
        );
    }

    #[test]
    fn explicit_stop_after_packetizer_completion_preserves_queue() {
        let event = terminal_audio_event(terminal_stream_id(), true, true, None);
        assert!(
            matches!(event, AudioEvent::PlaybackStateChanged(silent_disco_core::domain::PlaybackState::Stopped)),
            "explicit Stop must not advance the queue as if it ended naturally"
        );
    }

    #[test]
    fn genuine_queue_exhaustion_still_reports_natural_end() {
        let event = terminal_audio_event(terminal_stream_id(), false, true, None);
        assert!(matches!(event, AudioEvent::EndOfStream { .. }));
    }

    #[test]
    fn early_worker_cancellation_does_not_claim_natural_end() {
        let event = terminal_audio_event(terminal_stream_id(), false, false, None);
        assert!(
            matches!(event, AudioEvent::PlaybackStateChanged(silent_disco_core::domain::PlaybackState::Stopped))
        );
    }

    #[test]
    fn a_transport_failure_remains_visible_when_stop_is_also_requested() {
        let event = terminal_audio_event(
            terminal_stream_id(),
            true,
            true,
            Some("network send failed"),
        );
        assert!(matches!(event, AudioEvent::Failed(_)));
    }

    #[test]
    fn implicit_drop_cleanup_classifies_worker_failure_instead_of_discarding_it() {
        let error = crate::dto::DesktopErrorDto::new(
            "desktop.playback.test_failure",
            "audio",
            "error",
            false,
            "injected pump failure",
        );
        let failure = implicit_join_failure(Ok(Err(error)))
            .expect("a pump error must remain observable to the Drop fallback");
        assert!(failure.contains("injected pump failure"));
    }

    #[test]
    fn implicit_drop_cleanup_classifies_worker_panic_instead_of_discarding_it() {
        let worker = std::thread::spawn(|| -> Result<(), crate::dto::DesktopErrorDto> {
            panic!("injected playback pump panic");
        });
        let failure = implicit_join_failure(worker.join())
            .expect("a pump panic must remain observable to the Drop fallback");
        assert!(failure.contains("panicked during implicit shutdown"));
    }

    fn audio_frame(host_presentation_time_ms: u64) -> ProtocolFrame {
        ProtocolFrame::Audio(AudioDatagram {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            stream_id: StreamId::new("stream-pump-test").expect("stream id"),
            sequence: PacketSequence::new(0),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 240,
            first_sample_index: SampleIndex::new(0),
            host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
            payload: vec![0_u8; 240 * 2 * 2],
        })
    }

    #[test]
    fn position_progress_uses_unshifted_audio_time_not_pause_adjusted_time() {
        let mut frame = audio_frame(1_250);
        let content_position_ms = frame_content_position_ms(&frame, 1_000);
        apply_pause_offset(&mut frame, 500);
        assert_eq!(content_position_ms, Some(250));
        assert_eq!(frame_content_position_ms(&frame, 1_000), Some(750));
    }

    #[test]
    fn control_frames_never_advance_content_progress() {
        let frame = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        assert_eq!(frame_content_position_ms(&frame, 1_000), None);
    }

    /// This is the fix's whole mechanism: a resumed stream's packetizer keeps
    /// computing presentation times from its original, now-stale anchor, so
    /// the pump must add back exactly the elapsed pause duration before the
    /// send-ahead pacing check sees the frame -- get the arithmetic wrong and
    /// either pacing stays broken (offset too small) or every frame reads as
    /// further in the future than it really is (offset too large).
    #[test]
    fn adds_the_offset_to_an_audio_frames_presentation_time() {
        let mut frame = audio_frame(1_000);
        apply_pause_offset(&mut frame, 500);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), 1_500);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    /// The overwhelmingly common case -- a stream that has never paused --
    /// must be a true no-op, not just a zero-valued shift, since this runs on
    /// every single frame of every stream.
    #[test]
    fn a_zero_offset_leaves_the_presentation_time_unchanged() {
        let mut frame = audio_frame(1_000);
        apply_pause_offset(&mut frame, 0);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), 1_000);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    #[test]
    fn a_non_audio_frame_is_left_untouched() {
        let mut frame = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        let before = frame.clone();
        apply_pause_offset(&mut frame, 500);
        assert!(frame == before, "a non-audio frame must never be mutated");
    }

    /// Saturates rather than wraps -- a wrapped timestamp would read as an
    /// enormous negative lead in `wait_until_within_send_ahead_horizon`'s
    /// `saturating_sub`, which is exactly the "reads as due immediately"
    /// failure this fix exists to prevent, just triggered a different way.
    #[test]
    fn saturates_instead_of_overflowing() {
        let mut frame = audio_frame(u64::MAX - 10);
        apply_pause_offset(&mut frame, 500);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), u64::MAX);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    /// Block 34.3 "host transmit continues or stops exactly according to
    /// policy": this is the actual mechanism that guarantees it. A
    /// capacity-0 `sync_channel` (a rendezvous channel) makes `try_send`
    /// fail immediately whenever no receiver is concurrently waiting --
    /// exactly the "monitor cannot keep up right now" case -- and
    /// `forward_to_monitor` must swallow that and return normally rather
    /// than blocking or propagating an error, since [`run_pump`]'s call
    /// site always reaches the unconditional broadcast immediately after
    /// this call regardless of what happened here.
    #[test]
    fn forwarding_to_a_tap_that_cannot_accept_right_now_never_blocks_or_panics() {
        let (tap, _receiver) = std::sync::mpsc::sync_channel(0);
        let frame = audio_frame(1_000);
        forward_to_monitor(&frame, Some(&tap));
        // Reaching here at all is the proof: a blocking or panicking
        // implementation would never return control to this line.
    }

    #[test]
    fn stopped_pump_does_not_forward_audio_to_monitor_after_pacing() {
        let (tap, receiver) = std::sync::mpsc::sync_channel(2);
        let stop = AtomicBool::new(false);
        let frame = audio_frame(50_000);

        // Model a Stop request while send-ahead pacing was still waiting.
        stop.store(true, Ordering::Release);
        forward_to_monitor_if_running(&frame, Some(&tap), &stop);
        assert!(receiver.try_recv().is_err(), "Stop must suppress the monitor tap");

        stop.store(false, Ordering::Release);
        forward_to_monitor_if_running(&frame, Some(&tap), &stop);
        assert!(receiver.try_recv().is_ok(), "active playback still feeds the monitor");
    }

    #[test]
    fn monitor_only_receives_audio_after_successful_network_handoff() {
        let (tap, receiver) = std::sync::mpsc::sync_channel(2);
        let stop = AtomicBool::new(false);
        let failure = crate::dto::DesktopErrorDto::new(
            "desktop.playback.test_broadcast_failure",
            "audio",
            "error",
            true,
            "injected network failure",
        );
        let failed = broadcast_then_monitor(
            &audio_frame(1_000),
            |_frame| Err(failure),
            Some(&tap),
            &stop,
        );
        assert!(failed.is_err(), "the network error must propagate");
        assert!(
            receiver.try_recv().is_err(),
            "the monitor must not play an unaccepted network packet"
        );

        let accepted = broadcast_then_monitor(
            &audio_frame(2_000),
            |frame| {
                assert!(matches!(frame, ProtocolFrame::Audio(_)));
                Ok(true)
            },
            Some(&tap),
            &stop,
        );
        assert!(matches!(accepted, Ok(true)));
        let played = receiver.try_recv().expect("accepted frame reaches monitor");
        assert_eq!(played.host_presentation_time_ms.get(), 2_000);
    }

    #[test]
    fn stop_before_network_handoff_suppresses_broadcast_and_monitor() {
        let (tap, receiver) = std::sync::mpsc::sync_channel(1);
        let stop = AtomicBool::new(true);
        let broadcast_called = AtomicBool::new(false);
        let outcome = broadcast_then_monitor(
            &audio_frame(1_000),
            |_frame| {
                broadcast_called.store(true, Ordering::Release);
                Ok(true)
            },
            Some(&tap),
            &stop,
        );
        assert!(matches!(outcome, Ok(false)));
        assert!(!broadcast_called.load(Ordering::Acquire));
        assert!(receiver.try_recv().is_err());
    }


    #[test]
    fn pause_defers_network_frame_without_discarding_or_monitoring_it() {
        let stop = AtomicBool::new(false);
        let (tap, receiver) = std::sync::mpsc::sync_channel(1);
        let frame = audio_frame(1_000);
        let deferred = broadcast_then_monitor(&frame, |_frame| Ok(false), Some(&tap), &stop);
        assert!(matches!(deferred, Ok(false)));
        assert!(receiver.try_recv().is_err());
        let delivered = broadcast_then_monitor(&frame, |_frame| Ok(true), Some(&tap), &stop);
        assert!(matches!(delivered, Ok(true)));
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn pause_during_send_ahead_reanchors_the_held_frame_once() {
        let paused = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        let offset = AtomicU64::new(0);
        let clock = AtomicU64::new(1_000);
        let looked_up = AtomicBool::new(false);
        let mut frame = audio_frame(3_000);
        let mut applied = 0;

        std::thread::scope(|scope| {
            let controller = scope.spawn(|| {
                while !looked_up.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                paused.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(50));
                offset.store(500, Ordering::Release);
                clock.store(2_600, Ordering::Release);
                paused.store(false, Ordering::Release);
            });
            let ready = wait_until_within_send_ahead_horizon_with_clock(
                &mut frame,
                || {
                    looked_up.store(true, Ordering::Release);
                    Ok(clock.load(Ordering::Acquire))
                },
                &stop,
                &paused,
                &offset,
                &mut applied,
                &mut super::OpeningPace::default(),
            )
            .expect("clock lookup");
            assert!(ready);
            controller.join().expect("pause controller");
        });
        let ProtocolFrame::Audio(audio) = &frame else {
            panic!("expected audio frame");
        };
        assert_eq!(audio.host_presentation_time_ms.get(), 3_500);
        assert_eq!(applied, 500);
    }

    #[test]
    fn stop_interrupts_a_held_paused_frame_without_reading_clock() {
        let paused = AtomicBool::new(true);
        let stop = AtomicBool::new(false);
        let offset = AtomicU64::new(0);
        let clock_called = AtomicBool::new(false);
        let mut frame = audio_frame(1_000);
        let mut applied = 0;
        std::thread::scope(|scope| {
            let cancel = scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                stop.store(true, Ordering::Release);
            });
            let ready = wait_until_within_send_ahead_horizon_with_clock(
                &mut frame,
                || {
                    clock_called.store(true, Ordering::Release);
                    Ok(1_000)
                },
                &stop,
                &paused,
                &offset,
                &mut applied,
                &mut super::OpeningPace::default(),
            )
            .expect("clock lookup");
            assert!(!ready, "Stop must cancel even while Paused");
            cancel.join().expect("Stop controller");
        });
        assert!(!clock_called.load(Ordering::Acquire));
        assert_eq!(applied, 0);
    }

    /// A stream's opening leaves at twice real time, not all at once: with
    /// an 800 ms start lead the 40 packets already inside the 1 s horizon
    /// used to leave back to back, and Wi-Fi lost a burst of them on the A54.
    #[test]
    fn the_opening_is_paced_at_twice_real_time() {
        let mut pace = super::OpeningPace::default();
        let start = 10_000;
        let first_presented = start + 800;
        assert_eq!(pace.earliest_send_ms(start, first_presented), start, "the first packet goes at once");
        let sends: Vec<u64> = (0..40)
            .map(|k| pace.earliest_send_ms(start, first_presented + 5 * k))
            .collect();
        assert_eq!(sends[1], start + 3, "5 ms of audio every 2.5 ms, rounded up");
        assert_eq!(sends[39], start + 98);
        assert!(sends.windows(2).all(|pair| pair[1] >= pair[0]), "never earlier than the one before");
    }

    #[test]
    fn pacing_never_holds_back_a_stream_that_has_caught_up_with_its_horizon() {
        let mut pace = super::OpeningPace::default();
        let start = 10_000;
        pace.earliest_send_ms(start, start + 800);
        // Ten seconds in, the horizon sends a packet presented 1 s ahead.
        let now = start + 10_000;
        assert!(pace.earliest_send_ms(now, now + 1_000) <= now);
        // After a 30 s pause, presentation shifted by the pause and real
        // time passed with it.
        let after_pause = now + 30_000;
        assert!(pace.earliest_send_ms(after_pause, after_pause + 1_000) <= after_pause);
    }

    #[test]
    fn the_send_ahead_wait_releases_the_second_packet_only_at_its_paced_time() {
        let paused = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        let offset = AtomicU64::new(0);
        let clock = AtomicU64::new(10_000);
        let mut pace = super::OpeningPace::default();
        let mut applied = 0;
        let mut first = audio_frame(10_800);
        assert!(
            wait_until_within_send_ahead_horizon_with_clock(
                &mut first,
                || Ok(clock.load(Ordering::Acquire)),
                &stop,
                &paused,
                &offset,
                &mut applied,
                &mut pace,
            )
            .expect("first")
        );
        // The second packet is 100 ms later in the stream: it may leave 50
        // ms after the first. Each clock read advances 1 ms.
        let mut second = audio_frame(10_900);
        let mut reads = Vec::new();
        assert!(
            wait_until_within_send_ahead_horizon_with_clock(
                &mut second,
                || {
                    let now = clock.fetch_add(1, Ordering::AcqRel);
                    reads.push(now);
                    Ok(now)
                },
                &stop,
                &paused,
                &offset,
                &mut applied,
                &mut pace,
            )
            .expect("second")
        );
        assert_eq!(*reads.last().expect("read"), 10_050, "released at its paced time, not before");
    }

    fn sequenced_audio(sequence: u64, presentation_ms: u64) -> ProtocolFrame {
        let ProtocolFrame::Audio(mut datagram) = audio_frame(presentation_ms) else {
            unreachable!("audio_frame builds audio");
        };
        datagram.sequence = PacketSequence::new(sequence);
        ProtocolFrame::Audio(datagram)
    }

    fn sequence_of(frame: &ProtocolFrame) -> u64 {
        match frame {
            ProtocolFrame::Audio(datagram) => datagram.sequence.get(),
            _ => panic!("expected audio"),
        }
    }

    /// The first 200 ms are sent twice, each copy 20 packets after its
    /// original; nothing later is copied (stream start-up loss, 2026-11-20).
    #[test]
    fn each_packet_of_the_opening_is_sent_once_more_twenty_packets_later() {
        let mut resend = super::OpeningResend::default();
        let mut copies = Vec::new();
        for sequence in 0..200 {
            let frame = sequenced_audio(sequence, 10_800 + 5 * sequence);
            if let Some(copy) = resend.after_sent(&frame, 0) {
                copies.push((sequence, sequence_of(&copy)));
            }
        }
        assert_eq!(copies.len(), 40, "200 ms of 5 ms packets");
        assert_eq!(copies.first(), Some(&(20, 0)), "the first copy follows packet 20");
        assert!(copies.iter().all(|(sent_after, copied)| sent_after - copied == 20));
        assert_eq!(copies.last(), Some(&(59, 39)), "the last packet of the opening");
    }

    #[test]
    fn a_pause_during_the_opening_discards_copies_not_yet_sent() {
        let mut resend = super::OpeningResend::default();
        for sequence in 0..10 {
            assert!(resend.after_sent(&sequenced_audio(sequence, 10_800 + 5 * sequence), 0).is_none());
        }
        // Resumed with a 2 s pause offset: copies sent under the old offset
        // carry pre-pause presentation times and must not go out.
        let copies: Vec<u64> = (10..80)
            .filter_map(|sequence| resend.after_sent(&sequenced_audio(sequence, 12_800 + 5 * sequence), 2_000))
            .map(|copy| sequence_of(&copy))
            .collect();
        assert!(copies.iter().all(|&sequence| sequence >= 10), "no pre-pause copy: {copies:?}");
    }

    #[test]
    fn control_frames_are_never_copied() {
        let mut resend = super::OpeningResend::default();
        let stop = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        assert!(resend.after_sent(&stop, 0).is_none());
    }

    /// The overwhelmingly common case -- no monitor active at all -- must
    /// also be a true no-op.
    #[test]
    fn forwarding_with_no_monitor_tap_is_a_no_op() {
        let frame = audio_frame(1_000);
        forward_to_monitor(&frame, None);
    }

    /// A non-audio frame must never be forwarded to the monitor, which only
    /// ever understands `AudioDatagram`s.
    #[test]
    fn a_non_audio_frame_is_never_forwarded_to_the_monitor() {
        let (tap, receiver) = std::sync::mpsc::sync_channel(1);
        let frame = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        forward_to_monitor(&frame, Some(&tap));
        assert!(
            receiver.try_recv().is_err(),
            "a control frame must never reach the monitor tap"
        );
    }
}
