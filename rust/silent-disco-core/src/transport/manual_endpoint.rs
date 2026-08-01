use std::net::IpAddr;

use serde::Deserialize;

use crate::domain::SessionId;
use crate::runtime::{NetworkEndpoint, current_protocol_version};

use super::types::{TransportChannel, TransportError, TransportErrorKind};

/// Bounded, validated representation of a desktop host's manual connection
/// payload. Field names and shape mirror exactly what the desktop host's
/// "Connection payload" copy control produces, so a value copied from the
/// desktop host UI parses without transformation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualHostEndpoint {
    pub endpoint: NetworkEndpoint,
    pub session_id: SessionId,
    pub protocol_version: u16,
    pub invite_code_required: bool,
    pub expires_at_ms: Option<u64>,
}

/// Maximum accepted size of a serialized manual connection payload.
pub const MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES: usize = 2_048;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualHostEndpointPayload {
    host_address: String,
    control_port: u16,
    sync_port: u16,
    audio_port: u16,
    session_id: String,
    protocol_version: u16,
    invite_code_required: bool,
    expires_at_ms: Option<String>,
}

impl ManualHostEndpoint {
    /// Parses and validates a copy-pasted desktop connection payload.
    ///
    /// `now_wall_clock_ms` is the caller-supplied current wall-clock time
    /// used only to reject an already-expired payload. It is never used for
    /// synchronization or playback scheduling, which remain monotonic-only.
    ///
    /// # Errors
    ///
    /// Returns a [`TransportErrorKind::Protocol`] error when the advertised
    /// protocol version does not match [`current_protocol_version`], and a
    /// [`TransportErrorKind::InvalidConfiguration`] error for every other
    /// malformed, oversized, expired, or unsupported field.
    pub fn parse(raw: &str, now_wall_clock_ms: u64) -> Result<Self, TransportError> {
        if raw.len() > MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES {
            return Err(invalid(
                "connection payload exceeds the maximum supported size",
            ));
        }
        let payload: ManualHostEndpointPayload = serde_json::from_str(raw)
            .map_err(|error| invalid(&format!("connection payload is not valid JSON: {error}")))?;
        Self::from_payload(payload, now_wall_clock_ms)
    }

    fn from_payload(
        payload: ManualHostEndpointPayload,
        now_wall_clock_ms: u64,
    ) -> Result<Self, TransportError> {
        let address: IpAddr = payload
            .host_address
            .parse()
            .map_err(|_| invalid("host address is blank or malformed"))?;
        if address.is_unspecified() || address.is_multicast() {
            return Err(invalid(
                "host address must be a specific unicast destination",
            ));
        }
        if let IpAddr::V4(v4) = address
            && v4.is_broadcast()
        {
            return Err(invalid("host address must not be a broadcast address"));
        }
        let endpoint = NetworkEndpoint::new(
            address,
            payload.control_port,
            payload.sync_port,
            payload.audio_port,
        )
        .map_err(|_| invalid("connection ports must all be nonzero"))?;
        let session_id = SessionId::new(payload.session_id)
            .map_err(|error| invalid(&format!("session ID is invalid: {error}")))?;
        if payload.protocol_version != current_protocol_version() {
            return Err(TransportError::new(
                TransportErrorKind::Protocol,
                TransportChannel::Control,
                format!(
                    "unsupported protocol version {}; this build supports version {}",
                    payload.protocol_version,
                    current_protocol_version()
                ),
            ));
        }
        let expires_at_ms = payload
            .expires_at_ms
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| invalid("expiration timestamp is malformed"))
            })
            .transpose()?;
        if let Some(expires_at_ms) = expires_at_ms
            && now_wall_clock_ms >= expires_at_ms
        {
            return Err(invalid("connection payload has expired"));
        }
        Ok(Self {
            endpoint,
            session_id,
            protocol_version: payload.protocol_version,
            invite_code_required: payload.invite_code_required,
            expires_at_ms,
        })
    }
}

fn invalid(message: &str) -> TransportError {
    TransportError::new(
        TransportErrorKind::InvalidConfiguration,
        TransportChannel::Runtime,
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES, ManualHostEndpoint};
    use crate::runtime::current_protocol_version;
    use crate::transport::TransportErrorKind;

    fn valid_payload() -> String {
        format!(
            r#"{{"hostAddress":"192.168.1.50","controlPort":41000,"syncPort":41001,"audioPort":41002,"sessionId":"session-1","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        )
    }

    #[test]
    fn parses_a_valid_desktop_payload() {
        let endpoint = ManualHostEndpoint::parse(&valid_payload(), 0).expect("valid payload");
        assert_eq!(endpoint.endpoint.control_port, 41_000);
        assert_eq!(endpoint.session_id.as_str(), "session-1");
        assert!(!endpoint.invite_code_required);
        assert_eq!(endpoint.expires_at_ms, None);
    }

    #[test]
    fn parses_loopback_for_local_testing() {
        let payload = format!(
            r#"{{"hostAddress":"127.0.0.1","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":true,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert!(ManualHostEndpoint::parse(&payload, 0).is_ok());
    }

    #[test]
    fn rejects_malformed_json() {
        let error = ManualHostEndpoint::parse("not json", 0).unwrap_err();
        assert_eq!(error.kind, TransportErrorKind::InvalidConfiguration);
    }

    #[test]
    fn rejects_unknown_fields() {
        let payload = format!(
            r#"{{"hostAddress":"127.0.0.1","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":true,"expiresAtMs":null,"extra":"x"}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_oversized_payload() {
        let filler = "a".repeat(MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES);
        let payload = format!(r#"{{"hostAddress":"{filler}"}}"#);
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_blank_address() {
        let payload = format!(
            r#"{{"hostAddress":"","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_multicast_address() {
        let payload = format!(
            r#"{{"hostAddress":"239.1.1.1","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_unspecified_address() {
        let payload = format!(
            r#"{{"hostAddress":"0.0.0.0","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_zero_port() {
        let payload = format!(
            r#"{{"hostAddress":"192.168.1.50","controlPort":0,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_oversized_session_id() {
        let filler = "s".repeat(200);
        let payload = format!(
            r#"{{"hostAddress":"192.168.1.50","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"{filler}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 0).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn rejects_wrong_protocol_version_distinctly() {
        let payload = r#"{"hostAddress":"192.168.1.50","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":65535,"inviteCodeRequired":false,"expiresAtMs":null}"#;
        assert_eq!(
            ManualHostEndpoint::parse(payload, 0).unwrap_err().kind,
            TransportErrorKind::Protocol
        );
    }

    #[test]
    fn rejects_expired_payload() {
        let payload = format!(
            r#"{{"hostAddress":"192.168.1.50","controlPort":1,"syncPort":2,"audioPort":3,"sessionId":"s","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":"1000"}}"#,
            current_protocol_version()
        );
        assert_eq!(
            ManualHostEndpoint::parse(&payload, 1_000).unwrap_err().kind,
            TransportErrorKind::InvalidConfiguration
        );
        assert!(ManualHostEndpoint::parse(&payload, 999).is_ok());
    }
}
