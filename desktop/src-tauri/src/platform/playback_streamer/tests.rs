#[cfg(test)]
mod tests {
    use super::{apply_pause_offset, forward_to_monitor, implicit_join_failure};
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{
        AudioCodec, AudioDatagram, ControlMessage, Disconnect, ProtocolFrame,
    };


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
