use rusqlite::Connection;

use super::VerifiedListenerRecord;
use crate::domain::{DeviceId, TrustState};
use crate::storage::database::{DatabaseConfig, DatabaseConnection};
use crate::storage::{
    LATEST_SCHEMA_VERSION, StorageErrorKind, TrustedDevice, test_support::TestDatabasePath,
};

const HOST: [u8; 32] = [0xa1; 32];
const OTHER_HOST: [u8; 32] = [0xb2; 32];
const LISTENER: [u8; 32] = [0x0c; 32];

fn open(label: &str) -> (TestDatabasePath, DatabaseConnection) {
    let path = TestDatabasePath::new(label);
    let config = DatabaseConfig::new(path.path()).expect("valid config");
    let connection = DatabaseConnection::open(&config).expect("database opens");
    (path, connection)
}

fn record(name: &str, enrolled_at_ms: u64) -> VerifiedListenerRecord {
    VerifiedListenerRecord {
        host_key_fingerprint: HOST,
        listener_key_fingerprint: LISTENER,
        display_name: name.to_owned(),
        enrolled_at_ms,
        revoked_at_ms: None,
    }
}

#[test]
fn an_enrolled_key_is_trusted_for_its_host_only_and_survives_reopen() {
    let (path, mut database) = open("verified-enroll");
    database
        .enroll_verified_listener(&record("Alex's phone", 10))
        .expect("enrolls");

    let listed = database.list_verified_listeners(&HOST).expect("lists");
    assert_eq!(listed, vec![record("Alex's phone", 10)]);
    assert!(listed[0].is_trusted());
    // A rotated host key is a different scope: it inherits no trust.
    assert!(
        database
            .list_verified_listeners(&OTHER_HOST)
            .expect("lists")
            .is_empty()
    );

    let _ = database.checkpoint_and_close();
    let config = DatabaseConfig::new(path.path()).expect("valid config");
    let reopened = DatabaseConnection::open(&config).expect("reopens");
    assert_eq!(
        reopened.list_verified_listeners(&HOST).expect("lists"),
        vec![record("Alex's phone", 10)]
    );
    let _ = reopened.checkpoint_and_close();
}

#[test]
fn revocation_keeps_the_key_known_as_revoked_until_it_is_enrolled_again() {
    let (_path, mut database) = open("verified-revoke");
    database
        .enroll_verified_listener(&record("Alex", 10))
        .expect("enrolls");

    assert!(
        database
            .revoke_verified_listener(&HOST, &LISTENER, 20)
            .expect("revokes")
    );
    let listed = database.list_verified_listeners(&HOST).expect("lists");
    assert_eq!(listed.len(), 1, "a revoked key stays recorded");
    assert_eq!(listed[0].revoked_at_ms, Some(20));
    assert!(!listed[0].is_trusted());

    // Revoking twice, or revoking a key that was never enrolled, changes nothing.
    assert!(
        !database
            .revoke_verified_listener(&HOST, &LISTENER, 30)
            .expect("second revoke")
    );
    assert!(
        !database
            .revoke_verified_listener(&HOST, &[0xdd; 32], 30)
            .expect("unknown key")
    );
    assert!(
        !database
            .revoke_verified_listener(&OTHER_HOST, &LISTENER, 30)
            .expect("other host")
    );

    // Choosing "Always allow" again trusts it again.
    database
        .enroll_verified_listener(&record("Alex", 40))
        .expect("re-enrolls");
    assert_eq!(
        database.list_verified_listeners(&HOST).expect("lists"),
        vec![record("Alex", 40)]
    );
}

#[test]
fn a_revocation_is_never_dated_before_its_enrollment() {
    let (_path, mut database) = open("verified-revoke-order");
    database
        .enroll_verified_listener(&record("Alex", 50))
        .expect("enrolls");
    assert!(
        database
            .revoke_verified_listener(&HOST, &LISTENER, 5)
            .expect("revokes")
    );
    assert_eq!(
        database.list_verified_listeners(&HOST).expect("lists")[0].revoked_at_ms,
        Some(50)
    );
}

#[test]
fn invalid_enrollments_are_refused_visibly() {
    let (_path, mut database) = open("verified-invalid");
    let blank = database
        .enroll_verified_listener(&record("   ", 1))
        .expect_err("a blank name is refused");
    assert_eq!(blank.kind, StorageErrorKind::InvalidConfiguration);
    let mut revoked = record("Alex", 1);
    revoked.revoked_at_ms = Some(2);
    let already_revoked = database
        .enroll_verified_listener(&revoked)
        .expect_err("an enrollment cannot start revoked");
    assert_eq!(already_revoked.kind, StorageErrorKind::InvalidConfiguration);
    assert!(
        database
            .list_verified_listeners(&HOST)
            .expect("lists")
            .is_empty()
    );
}

/// The legacy claimed-ID table is never read as verified trust: a trusted
/// legacy row, even one carrying a public key, does not become a record.
#[test]
fn a_legacy_trusted_device_is_never_promoted_to_verified_trust() {
    let (_path, mut database) = open("verified-legacy");
    database
        .upsert_trusted_device(&TrustedDevice {
            device_id: DeviceId::new("claimed-listener").expect("device id"),
            display_name: "Claimed".to_owned(),
            public_key: Some(vec![4; 65]),
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: 1,
            last_seen_ms: 1,
            updated_at_ms: 1,
        })
        .expect("legacy row");
    assert!(
        database
            .list_verified_listeners(&HOST)
            .expect("lists")
            .is_empty()
    );
}

#[test]
fn a_schema_4_profile_upgrades_with_an_empty_verified_listener_table() {
    let path = TestDatabasePath::new("verified-upgrade");
    let config = DatabaseConfig::new(path.path()).expect("valid config");
    let connection = DatabaseConnection::open(&config).expect("database opens");
    let _ = connection.checkpoint_and_close();
    {
        let raw = Connection::open(path.path()).expect("raw open");
        raw.execute_batch("DROP TABLE verified_listeners; PRAGMA user_version = 4;")
            .expect("rewind to schema 4");
        raw.execute("DELETE FROM schema_migrations WHERE version = 5", [])
            .expect("forget migration 5");
    }
    let upgraded = DatabaseConnection::open(&config).expect("schema 4 profile upgrades");
    assert_eq!(upgraded.metadata().schema_version, LATEST_SCHEMA_VERSION);
    assert!(
        upgraded
            .list_verified_listeners(&HOST)
            .expect("lists")
            .is_empty()
    );
    let _ = upgraded.checkpoint_and_close();
}

/// The table itself refuses a fingerprint that is not 64 lowercase hex digits,
/// so a bug elsewhere cannot store an unmatchable key.
#[test]
fn the_table_refuses_a_malformed_fingerprint() {
    let (path, database) = open("verified-check");
    let _ = database.checkpoint_and_close();
    let raw = Connection::open(path.path()).expect("raw open");
    let inserted = raw.execute(
        "INSERT INTO verified_listeners VALUES (?1, ?2, 'Alex', 1, NULL)",
        [&"A1".repeat(32), &"0c".repeat(32)],
    );
    assert!(inserted.is_err(), "uppercase hex was stored");
}
