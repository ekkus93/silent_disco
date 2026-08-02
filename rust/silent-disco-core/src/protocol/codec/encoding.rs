use super::{
    AudioDatagram, ControlMessage, DeviceId, Encoder, FLAG_PAYLOAD_INTEGRITY, FRAME_HEADER_BYTES,
    FrameHeader, MAX_AUDIO_DATAGRAM_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES,
    MAX_REASON_BYTES, MAX_SESSION_NAME_BYTES, PROTOCOL_MAGIC, ProtocolError, ProtocolFrame,
    SessionId, StreamId, SyncRequest, SyncResponse, crc32, validate_audio_parameters,
    validate_text,
};

impl Encoder {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn put_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_bool(&mut self, value: bool) {
        self.put_u8(u8::from(value));
    }

    fn put_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn put_string(
        &mut self,
        field: &'static str,
        value: &str,
        maximum: usize,
        secret: bool,
    ) -> Result<(), ProtocolError> {
        validate_text(field, value, maximum, secret)?;
        let length =
            u16::try_from(value.len()).map_err(|_| ProtocolError::InvalidField { field })?;
        self.put_u16(length);
        self.put_bytes(value.as_bytes());
        Ok(())
    }

    fn put_optional_string(
        &mut self,
        field: &'static str,
        value: Option<&str>,
        maximum: usize,
        secret: bool,
    ) -> Result<(), ProtocolError> {
        if let Some(value) = value {
            self.put_u8(1);
            self.put_string(field, value, maximum, secret)
        } else {
            self.put_u8(0);
            Ok(())
        }
    }

    fn put_session_id(&mut self, value: &SessionId) -> Result<(), ProtocolError> {
        self.put_string(
            "session_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn put_stream_id(&mut self, value: &StreamId) -> Result<(), ProtocolError> {
        self.put_string(
            "stream_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn put_device_id(&mut self, value: &DeviceId) -> Result<(), ProtocolError> {
        self.put_string(
            "device_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

fn encode_control(message: &ControlMessage) -> Result<Vec<u8>, ProtocolError> {
    let mut output = Encoder::with_capacity(256);
    match message {
        ControlMessage::Hello(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_string(
                "session_name",
                &value.session_name,
                MAX_SESSION_NAME_BYTES,
                false,
            )?;
            output.put_string("host_name", &value.host_name, MAX_DISPLAY_NAME_BYTES, false)?;
            output.put_bool(value.approval_required);
        }
        ControlMessage::JoinRequest(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.device.device_id)?;
            output.put_string(
                "display_name",
                &value.device.display_name,
                MAX_DISPLAY_NAME_BYTES,
                false,
            )?;
            output.put_optional_string(
                "invite_code",
                value.invite_code.as_deref(),
                MAX_INVITE_CODE_BYTES,
                true,
            )?;
            output.put_u16(value.sync_port);
            output.put_u16(value.audio_port);
        }
        ControlMessage::JoinApproval(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_bool(value.trusted_for_future);
        }
        ControlMessage::JoinRejection(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
        ControlMessage::Heartbeat(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_u64(value.sent_at_elapsed_ms.get());
        }
        ControlMessage::StreamStart(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_start_time_ms.get());
            output.put_u32(value.sample_rate);
            output.put_u16(value.channels);
            output.put_u32(value.samples_per_packet);
        }
        ControlMessage::Pause(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_pause_time_ms.get());
        }
        ControlMessage::Stop(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_stop_time_ms.get());
        }
        ControlMessage::Disconnect(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
        ControlMessage::ResyncNotice(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
    }
    Ok(output.finish())
}

fn encode_sync_request(value: &SyncRequest) -> Result<Vec<u8>, ProtocolError> {
    let mut output = Encoder::with_capacity(160);
    output.put_session_id(&value.session_id)?;
    output.put_u64(value.correlation_id);
    output.put_u64(value.t1_listener_send_elapsed_ms.get());
    Ok(output.finish())
}

fn encode_sync_response(value: &SyncResponse) -> Result<Vec<u8>, ProtocolError> {
    if value.t3_host_send_elapsed_ms < value.t2_host_receive_elapsed_ms {
        return Err(ProtocolError::InvalidField {
            field: "host_timestamp_order",
        });
    }
    let mut output = Encoder::with_capacity(176);
    output.put_session_id(&value.session_id)?;
    output.put_u64(value.correlation_id);
    output.put_u64(value.t1_listener_send_elapsed_ms.get());
    output.put_u64(value.t2_host_receive_elapsed_ms.get());
    output.put_u64(value.t3_host_send_elapsed_ms.get());
    Ok(output.finish())
}

fn encode_audio(value: &AudioDatagram) -> Result<Vec<u8>, ProtocolError> {
    validate_audio_parameters(value)?;
    let mut output = Encoder::with_capacity(320 + value.payload.len());
    output.put_session_id(&value.session_id)?;
    output.put_stream_id(&value.stream_id)?;
    output.put_u64(value.sequence.get());
    output.put_u8(value.codec.code());
    output.put_u32(value.sample_rate);
    output.put_u16(value.channels);
    output.put_u32(value.samples_per_packet);
    output.put_u64(value.first_sample_index.get());
    output.put_u64(value.host_presentation_time_ms.get());
    let payload_length =
        u16::try_from(value.payload.len()).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: value.payload.len(),
            maximum: u16::MAX.into(),
        })?;
    output.put_u16(payload_length);
    output.put_u32(crc32(&value.payload));
    output.put_bytes(&value.payload);
    let encoded = output.finish();
    let maximum = MAX_AUDIO_DATAGRAM_BYTES - FRAME_HEADER_BYTES;
    if encoded.len() > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: encoded.len(),
            maximum,
        });
    }
    Ok(encoded)
}

fn encode_payload(frame: &ProtocolFrame) -> Result<(Vec<u8>, u16), ProtocolError> {
    match frame {
        ProtocolFrame::Control(message) => encode_control(message).map(|payload| (payload, 0)),
        ProtocolFrame::SyncRequest(value) => encode_sync_request(value).map(|payload| (payload, 0)),
        ProtocolFrame::SyncResponse(value) => {
            encode_sync_response(value).map(|payload| (payload, 0))
        }
        ProtocolFrame::Audio(value) => {
            encode_audio(value).map(|payload| (payload, FLAG_PAYLOAD_INTEGRITY))
        }
    }
}

/// Encodes one canonical protocol-v2 frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when a field violates its bound, audio metadata
/// is inconsistent, or the encoded payload exceeds the message-kind limit.
pub fn encode_frame(frame: &ProtocolFrame) -> Result<Vec<u8>, ProtocolError> {
    let kind = frame.kind();
    let (payload, flags) = encode_payload(frame)?;
    let maximum = kind.maximum_payload_bytes();
    if payload.len() > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: payload.len(),
            maximum,
        });
    }
    let payload_length =
        u32::try_from(payload.len()).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: payload.len(),
            maximum,
        })?;
    let header = FrameHeader::new(kind, flags, payload_length);
    let mut output = Encoder::with_capacity(FRAME_HEADER_BYTES + payload.len());
    output.put_bytes(&PROTOCOL_MAGIC);
    output.put_u16(header.version);
    output.put_u16(header.kind.code());
    output.put_u16(header.flags);
    output.put_u16(header.header_length);
    output.put_u32(header.payload_length);
    output.put_bytes(&payload);
    Ok(output.finish())
}
