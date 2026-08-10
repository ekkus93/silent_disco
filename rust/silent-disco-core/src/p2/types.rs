#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentSessionRole {
    Host,
    Listener,
}

impl RecentSessionRole {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Listener => "listener",
        }
    }

    fn parse(value: &str) -> Result<Self, P2Error> {
        match value {
            "host" => Ok(Self::Host),
            "listener" => Ok(Self::Listener),
            _ => Err(P2Error::CorruptStore(
                "unknown recent-session role".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentSessionOutcome {
    Completed,
    Cancelled,
    Failed,
}

impl RecentSessionOutcome {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, P2Error> {
        match value {
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            _ => Err(P2Error::CorruptStore(
                "unknown recent-session outcome".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSessionRecord {
    pub session_id: String,
    pub role: RecentSessionRole,
    pub session_name: String,
    pub host_name: String,
    pub host_fingerprint: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub outcome: RecentSessionOutcome,
}

impl RecentSessionRecord {
    fn validate(&self) -> Result<(), P2Error> {
        SessionId::new(self.session_id.clone())
            .map_err(|_| P2Error::InvalidArgument("invalid session identifier".to_owned()))?;
        validate_name(&self.session_name, "session name")?;
        validate_name(&self.host_name, "host name")?;
        validate_fingerprint(self.host_fingerprint.as_deref())?;
        if self.started_at_ms == 0 || self.ended_at_ms < self.started_at_ms {
            return Err(P2Error::InvalidArgument(
                "recent-session timestamps are invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHostRecord {
    pub fingerprint: String,
    pub display_name: String,
    pub public_key_der: Vec<u8>,
    pub first_verified_ms: u64,
    pub last_verified_ms: u64,
}

impl TrustedHostRecord {
    fn validate(&self) -> Result<(), P2Error> {
        validate_fingerprint(Some(&self.fingerprint))?;
        validate_name(&self.display_name, "trusted-host display name")?;
        if self.public_key_der.is_empty() || self.public_key_der.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(P2Error::InvalidArgument(
                "trusted-host public key has an unsupported size".to_owned(),
            ));
        }
        if self.first_verified_ms == 0 || self.last_verified_ms < self.first_verified_ms {
            return Err(P2Error::InvalidArgument(
                "trusted-host timestamps are invalid".to_owned(),
            ));
        }
        let actual = public_key_fingerprint(&self.public_key_der);
        if actual != self.fingerprint {
            return Err(P2Error::InvalidArgument(
                "trusted-host fingerprint does not match its public key".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrInvitationInput {
    pub session_id: String,
    pub session_name: String,
    pub host_name: String,
    pub host_public_key_der: Vec<u8>,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
    /// The host's own manual-endpoint connection payload (see
    /// [`crate::transport::ManualHostEndpoint`]), embedded verbatim as an
    /// opaque, canonically-signed string -- present only for hosts with a
    /// real network endpoint to hand a listener (desktop today; Android's
    /// own peer-to-peer QR flow leaves this `None` and keeps relying on
    /// BLE/Wi-Fi Direct discovery after verification). Carrying the exact
    /// same JSON shape the manual paste flow already accepts means the
    /// listener side can validate and connect with it via the identical,
    /// already-tested `ManualHostEndpoint::parse` path, not a second
    /// parallel implementation.
    pub connection_payload_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedQrInvitation {
    pub session_id: String,
    pub session_name: String,
    pub host_name: String,
    pub host_public_key_der: Vec<u8>,
    pub host_fingerprint: String,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
    /// See [`QrInvitationInput::connection_payload_json`]. Already validated
    /// as well-formed by [`validate_unsigned`] (structurally parsed via
    /// [`ManualHostEndpoint::parse`]), but a caller still has to re-parse it
    /// to get typed fields back out -- this module only ever carries it as
    /// an opaque, size-bounded string.
    pub connection_payload_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsignedQrDocument {
    v: u8,
    alg: String,
    sid: String,
    sn: String,
    hn: String,
    pk: String,
    am: String,
    code: Option<String>,
    iat: u64,
    exp: u64,
    nonce: String,
    #[serde(default)]
    conn: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedQrDocument {
    v: u8,
    alg: String,
    sid: String,
    sn: String,
    hn: String,
    pk: String,
    am: String,
    code: Option<String>,
    iat: u64,
    exp: u64,
    nonce: String,
    #[serde(default)]
    conn: Option<String>,
    sig: String,
}

impl From<SignedQrDocument> for UnsignedQrDocument {
    fn from(value: SignedQrDocument) -> Self {
        Self {
            v: value.v,
            alg: value.alg,
            sid: value.sid,
            sn: value.sn,
            hn: value.hn,
            pk: value.pk,
            am: value.am,
            code: value.code,
            iat: value.iat,
            exp: value.exp,
            nonce: value.nonce,
            conn: value.conn,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2Error {
    InvalidArgument(String),
    Storage(String),
    CorruptStore(String),
    InvalidQr(String),
    ExpiredQr,
    ReplayedQr,
    InvalidSignature,
}

impl fmt::Display for P2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => write!(formatter, "invalid P2 argument: {message}"),
            Self::Storage(message) => write!(formatter, "P2 storage failure: {message}"),
            Self::CorruptStore(message) => write!(formatter, "P2 storage is corrupt: {message}"),
            Self::InvalidQr(message) => {
                write!(formatter, "invalid Silent Disco QR invitation: {message}")
            }
            Self::ExpiredQr => formatter.write_str("Silent Disco QR invitation has expired"),
            Self::ReplayedQr => {
                formatter.write_str("Silent Disco QR invitation was already used on this phone")
            }
            Self::InvalidSignature => {
                formatter.write_str("Silent Disco QR invitation signature is invalid")
            }
        }
    }
}

impl Error for P2Error {}

pub struct P2Store {
    connection: Connection,
}
