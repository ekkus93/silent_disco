//! Rust-owned persistence and validation for optional P2 convenience features.
//!
//! This module deliberately keeps recent-session history, trusted-host keys, and
//! consumed invitation nonces out of Kotlin-owned state. The store uses a
//! dedicated app-private `SQLite` file so it never opens or competes for the
//! existing domain database connection.

use std::{
    error::Error,
    fmt,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{
    ecdsa::{Signature, VerifyingKey, signature::Verifier},
    pkcs8::DecodePublicKey,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::SessionId;
use crate::transport::{MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES, ManualHostEndpoint};

pub const P2_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_RECENT_MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
pub const MAX_RECENT_QUERY_LIMIT: u32 = 20;
pub const MAX_QR_LIFETIME_MS: u64 = 10 * 60 * 1_000;
pub const QR_CLOCK_SKEW_MS: u64 = 60 * 1_000;
const MAX_NAME_BYTES: usize = 256;
const MAX_PUBLIC_KEY_BYTES: usize = 1_024;
const MAX_QR_PAYLOAD_BYTES: usize = 16_384;

include!("types.rs");
include!("store_recent.rs");
include!("store_trust_qr.rs");
include!("qr.rs");
include!("validation.rs");
include!("storage.rs");
include!("tests.rs");
