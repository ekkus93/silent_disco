//! List and revoke of remembered listeners against a real database worker.

use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker, VerifiedListenerRecord};
use silent_disco_core::transport::secure_identity::key_bound_device_id;
use silent_disco_core::transport::secure_trust::HostVerifiedTrust;

use crate::verified_listeners::{list_verified_listeners, revoke_verified_listener};

fn spki(seed: u8) -> Vec<u8> {
    use p256::{ecdsa::SigningKey, pkcs8::EncodePublicKey};
    SigningKey::from_slice(&[seed; 32])
        .expect("fixture key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

fn record(host: &[u8], listener: u8, name: &str, at: u64) -> VerifiedListenerRecord {
    VerifiedListenerRecord {
        host_key_fingerprint: *HostVerifiedTrust::new(host)
            .expect("host")
            .host_fingerprint(),
        listener_key_fingerprint: [listener; 32],
        display_name: name.to_owned(),
        enrolled_at_ms: at,
        revoked_at_ms: None,
    }
}

struct Database {
    worker: Option<DatabaseWorker>,
    root: std::path::PathBuf,
}

impl Database {
    fn start(tag: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-{tag}-{unique}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let worker =
            DatabaseWorker::start(DatabaseConfig::new(root.join("trust.sqlite3")).expect("config"))
                .expect("database worker");
        Self {
            worker: Some(worker),
            root,
        }
    }

    fn client(&self) -> silent_disco_core::storage::DatabaseClient {
        self.worker.as_ref().expect("running").client()
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop_and_join().expect("database closes");
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn lists_only_this_hosts_trusted_listeners_and_revoke_removes_one() {
    let database = Database::start("verified-list");
    let client = database.client();
    let host = spki(3);
    let other_host = spki(4);
    client
        .enroll_verified_listener(&record(&host, 0xb2, "Riley", 20))
        .expect("enrol");
    client
        .enroll_verified_listener(&record(&host, 0xa1, "Alex", 10))
        .expect("enrol");
    client
        .enroll_verified_listener(&record(&other_host, 0xc3, "Other", 5))
        .expect("enrol");

    let listed = list_verified_listeners(&client, &host).expect("lists");
    let alex = key_bound_device_id(&[0xa1; 32]).as_str().to_owned();
    let riley = key_bound_device_id(&[0xb2; 32]).as_str().to_owned();
    assert_eq!(
        listed
            .iter()
            .map(|l| (
                l.device_id.as_str(),
                l.display_name.as_str(),
                l.enrolled_at_ms
            ))
            .collect::<Vec<_>>(),
        vec![(alex.as_str(), "Alex", 10), (riley.as_str(), "Riley", 20)],
        "only this host's records, oldest approval first"
    );

    assert!(revoke_verified_listener(&client, &host, &alex, 30).expect("revokes"));
    let listed = list_verified_listeners(&client, &host).expect("lists");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].device_id, riley);
    let stored = client
        .list_verified_listeners(
            *HostVerifiedTrust::new(&host)
                .expect("host")
                .host_fingerprint(),
        )
        .expect("raw list");
    assert!(
        stored
            .iter()
            .any(|r| r.listener_key_fingerprint == [0xa1; 32] && r.revoked_at_ms == Some(30)),
        "the revoked row is kept, marked revoked"
    );

    assert!(
        !revoke_verified_listener(&client, &host, &alex, 40).expect("second revoke"),
        "revoking again reports nothing was trusted"
    );
    assert!(
        !revoke_verified_listener(&client, &other_host, &riley, 40).expect("other host"),
        "a host cannot revoke another host's record"
    );
    assert_eq!(
        list_verified_listeners(&client, &host)
            .expect("lists")
            .len(),
        1
    );
}

#[test]
fn revoke_refuses_a_claimed_id_and_list_refuses_an_invalid_host_key() {
    let database = Database::start("verified-refuse");
    let client = database.client();
    let host = spki(3);
    let refused = revoke_verified_listener(&client, &host, "claimed-listener", 1)
        .expect_err("a claimed ID is not a remembered listener");
    assert!(
        refused.to_string().contains("remembered secure listener"),
        "{refused}"
    );
    assert!(list_verified_listeners(&client, b"not a key").is_err());
}
