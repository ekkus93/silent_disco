use super::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceId, DeviceIdentity, Disconnect,
    FLAG_PAYLOAD_INTEGRITY, FRAME_HEADER_BYTES, FrameHeader, Heartbeat, Hello, JoinApproval,
    JoinRejection, JoinRequest, MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES, MAX_REASON_BYTES,
    MAX_SESSION_NAME_BYTES, MessageKind, MonotonicMillis, PROTOCOL_MAGIC, PROTOCOL_VERSION,
    PacketSequence, Pause, ProtocolError, ProtocolFrame, Reader, ResyncNotice,
    SUPPORTED_FRAME_FLAGS, SampleIndex, SessionId, Stop, StreamId, StreamStart, SyncRequest,
    SyncResponse, crc32, validate_audio_parameters, validate_text,
};

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::Truncated)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::Truncated)?;
        self.position = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8, ProtocolError> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ProtocolError::InvalidBoolean { value }),
        }
    }

    fn read_string(
        &mut self,
        field: &'static str,
        maximum: usize,
        secret: bool,
    ) -> Result<String, ProtocolError> {
        let length = usize::from(self.read_u16()?);
        if length > maximum {
            return Err(ProtocolError::PayloadTooLarge {
                actual: length,
                maximum,
            });
        }
        let value =
            core::str::from_utf8(self.take(length)?).map_err(|_| ProtocolError::InvalidUtf8)?;
        validate_text(field, value, maximum, secret)?;
        Ok(value.to_owned())
    }

    fn read_optional_string(
        &mut self,
        field: &'static str,
        maximum: usize,
        secret: bool,
    ) -> Result<Option<String>, ProtocolError> {
        match self.read_u8()? {
            0 => Ok(None),
            1 => self.read_string(field, maximum, secret).map(Some),
            value => Err(ProtocolError::InvalidBoolean { value }),
        }
    }

    fn read_session_id(&mut self) -> Result<SessionId, ProtocolError> {
        let value = self.read_string("session_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        SessionId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn read_stream_id(&mut self) -> Result<StreamId, ProtocolError> {
        let value = self.read_string("stream_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        StreamId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn read_device_id(&mut self) -> Result<DeviceId, ProtocolError> {
        let value = self.read_string("device_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        DeviceId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes)
        }
    }
}

fn decode_control(kind: MessageKind, payload: &[u8]) -> Result<ControlMessage, ProtocolError> {
    let mut input = Reader::new(payload);
    let message = match kind {
        MessageKind::Hello => ControlMessage::Hello(Hello {
            session_id: input.read_session_id()?,
            session_name: input.read_string("session_name", MAX_SESSION_NAME_BYTES, false)?,
            host_name: input.read_string("host_name", MAX_DISPLAY_NAME_BYTES, false)?,
            approval_required: input.read_bool()?,
        }),
        MessageKind::JoinRequest => ControlMessage::JoinRequest(JoinRequest {
            session_id: input.read_session_id()?,
            device: DeviceIdentity {
                device_id: input.read_device_id()?,
                display_name: input.read_string("display_name", MAX_DISPLAY_NAME_BYTES, false)?,
            },
            invite_code: input.read_optional_string("invite_code", MAX_INVITE_CODE_BYTES, true)?,
        }),
        MessageKind::JoinApproval => ControlMessage::JoinApproval(JoinApproval {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            trusted_for_future: input.read_bool()?,
        }),
        MessageKind::JoinRejection => ControlMessage::JoinRejection(JoinRejection {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        MessageKind::Heartbeat => ControlMessage::Heartbeat(Heartbeat {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            sent_at_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::StreamStart => ControlMessage::StreamStart(StreamStart {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_start_time_ms: MonotonicMillis::new(input.read_u64()?),
            sample_rate: input.read_u32()?,
            channels: input.read_u16()?,
            samples_per_packet: input.read_u32()?,
        }),
        MessageKind::Pause => ControlMessage::Pause(Pause {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_pause_time_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::Stop => ControlMessage::Stop(Stop {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_stop_time_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::Disconnect => ControlMessage::Disconnect(Disconnect {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        MessageKind::ResyncNotice => ControlMessage::ResyncNotice(ResyncNotice {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        _ => {
            return Err(ProtocolError::UnsupportedMessageKind { kind: kind.code() });
        }
    };
    input.finish()?;
    Ok(message)
}

fn decode_sync_request(payload: &[u8]) -> Result<SyncRequest, ProtocolError> {
    let mut input = Reader::new(payload);
    let value = SyncRequest {
        session_id: input.read_session_id()?,
        correlation_id: input.read_u64()?,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
    };
    input.finish()?;
    Ok(value)
}

fn decode_sync_response(payload: &[u8]) -> Result<SyncResponse, ProtocolError> {
    let mut input = Reader::new(payload);
    let value = SyncResponse {
        session_id: input.read_session_id()?,
        correlation_id: input.read_u64()?,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        t3_host_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
    };
    input.finish()?;
    if value.t3_host_send_elapsed_ms < value.t2_host_receive_elapsed_ms {
        return Err(ProtocolError::InvalidField {
            field: "host_timestamp_order",
        });
    }
    Ok(value)
}

fn decode_audio(payload: &[u8]) -> Result<AudioDatagram, ProtocolError> {
    let mut input = Reader::new(payload);
    let session_id = input.read_session_id()?;
    let stream_id = input.read_stream_id()?;
    let sequence = PacketSequence::new(input.read_u64()?);
    let codec_code = input.read_u8()?;
    let codec = AudioCodec::try_from(codec_code)
        .map_err(|codec| ProtocolError::UnsupportedAudioCodec { codec })?;
    let sample_rate = input.read_u32()?;
    let channels = input.read_u16()?;
    let samples_per_packet = input.read_u32()?;
    let first_sample_index = SampleIndex::new(input.read_u64()?);
    let host_presentation_time_ms = MonotonicMillis::new(input.read_u64()?);
    let declared_payload_length = usize::from(input.read_u16()?);
    let checksum = input.read_u32()?;
    if declared_payload_length > input.remaining() {
        return Err(ProtocolError::LengthMismatch {
            declared: declared_payload_length,
            actual: input.remaining(),
        });
    }
    if declared_payload_length < input.remaining() {
        return Err(ProtocolError::TrailingBytes);
    }
    let payload_bytes = input.take(declared_payload_length)?;
    if crc32(payload_bytes) != checksum {
        return Err(ProtocolError::IntegrityMismatch);
    }
    input.finish()?;
    let value = AudioDatagram {
        session_id,
        stream_id,
        sequence,
        codec,
        sample_rate,
        channels,
        samples_per_packet,
        first_sample_index,
        host_presentation_time_ms,
        payload: payload_bytes.to_vec(),
    };
    validate_audio_parameters(&value)?;
    Ok(value)
}

/// Validates and decodes the fixed-width protocol-v2 header without allocating a payload.
///
/// # Errors
///
/// Returns [`ProtocolError`] for truncated input, invalid magic, unsupported
/// versions/kinds/flags, invalid header length, or an oversized declared payload.
pub fn decode_header(bytes: &[u8]) -> Result<FrameHeader, ProtocolError> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return Err(ProtocolError::Truncated);
    }
    let mut input = Reader::new(&bytes[..FRAME_HEADER_BYTES]);
    if input.take(PROTOCOL_MAGIC.len())? != PROTOCOL_MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    let version = input.read_u16()?;
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion { version });
    }
    let kind_code = input.read_u16()?;
    let kind = MessageKind::try_from(kind_code)
        .map_err(|kind| ProtocolError::UnsupportedMessageKind { kind })?;
    let flags = input.read_u16()?;
    if flags & !SUPPORTED_FRAME_FLAGS != 0 {
        return Err(ProtocolError::UnsupportedFlags { flags });
    }
    let expected_flags = if kind == MessageKind::Audio {
        FLAG_PAYLOAD_INTEGRITY
    } else {
        0
    };
    if flags != expected_flags {
        return Err(ProtocolError::UnsupportedFlags { flags });
    }
    let header_length = input.read_u16()?;
    if usize::from(header_length) != FRAME_HEADER_BYTES {
        return Err(ProtocolError::InvalidHeaderLength {
            length: header_length,
        });
    }
    let payload_length = input.read_u32()?;
    input.finish()?;
    let payload_length_usize =
        usize::try_from(payload_length).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: usize::MAX,
            maximum: kind.maximum_payload_bytes(),
        })?;
    let maximum = kind.maximum_payload_bytes();
    if payload_length_usize > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: payload_length_usize,
            maximum,
        });
    }
    Ok(FrameHeader {
        version,
        kind,
        flags,
        header_length,
        payload_length,
    })
}

/// Decodes exactly one complete canonical protocol-v2 frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when the frame header or payload is malformed,
/// unsupported, oversized, truncated, non-canonical, or fails integrity checks.
pub fn decode_frame(bytes: &[u8]) -> Result<ProtocolFrame, ProtocolError> {
    let header = decode_header(bytes)?;
    let payload_length =
        usize::try_from(header.payload_length).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: usize::MAX,
            maximum: header.kind.maximum_payload_bytes(),
        })?;
    let expected_total =
        FRAME_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or(ProtocolError::PayloadTooLarge {
                actual: usize::MAX,
                maximum: header.kind.maximum_payload_bytes(),
            })?;
    match bytes.len().cmp(&expected_total) {
        core::cmp::Ordering::Less => {
            return Err(ProtocolError::LengthMismatch {
                declared: payload_length,
                actual: bytes.len().saturating_sub(FRAME_HEADER_BYTES),
            });
        }
        core::cmp::Ordering::Greater => return Err(ProtocolError::TrailingBytes),
        core::cmp::Ordering::Equal => {}
    }
    let payload = &bytes[FRAME_HEADER_BYTES..expected_total];
    match header.kind {
        kind if kind.is_control() => decode_control(kind, payload).map(ProtocolFrame::Control),
        MessageKind::SyncRequest => decode_sync_request(payload).map(ProtocolFrame::SyncRequest),
        MessageKind::SyncResponse => decode_sync_response(payload).map(ProtocolFrame::SyncResponse),
        MessageKind::Audio => decode_audio(payload).map(ProtocolFrame::Audio),
        kind => Err(ProtocolError::UnsupportedMessageKind { kind: kind.code() }),
    }
}
