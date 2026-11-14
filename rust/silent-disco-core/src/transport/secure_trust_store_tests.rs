//! Trust records through a real database worker.

use crate::domain::{DeviceId, TrustState};
use crate::storage::{DatabaseConfig, DatabaseWorker};
use crate::transport::secure_identity::key_bound_device_id;
use crate::transport::secure_trust_store::{
    TrustStoreError, enroll_listener, join_trust_state, list_remembered_listeners, load_host_trust,
    revoke_listener,
};

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

fn worker() -> (tempfile::TempDir, DatabaseWorker) {
    let directory = tempfile::tempdir().expect("temp dir");
    let worker = DatabaseWorker::start(
        DatabaseConfig::new(directory.path().join("trust.sqlite3")).expect("config"),
    )
    .expect("database worker");
    (directory, worker)
}

#[test]
fn enrol_trusts_revoke_untrusts_and_other_hosts_see_nothing() {
    let (_directory, worker) = worker();
    let client = worker.client();
    let host = spki(3);
    let other_host = spki(4);
    let alex = key_bound_device_id(&[0xa1; 32]);
    let riley = key_bound_device_id(&[0xb2; 32]);

    enroll_listener(&client, &host, &riley, "Riley", 20).expect("enrol");
    enroll_listener(&client, &host, &alex, "Alex", 10).expect("enrol");
    let trust = load_host_trust(&client, &host).expect("load");
    assert_eq!(join_trust_state(Some(&trust), &alex), TrustState::Trusted);
    let listed = list_remembered_listeners(&client, &host).expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|l| (l.device_id.clone(), l.enrolled_at_ms))
            .collect::<Vec<_>>(),
        vec![(alex.clone(), 10), (riley.clone(), 20)],
        "oldest approval first"
    );

    assert!(revoke_listener(&client, &host, &alex, 30).expect("revoke"));
    assert!(!revoke_listener(&client, &host, &alex, 31).expect("again"));
    let trust = load_host_trust(&client, &host).expect("reload");
    assert_eq!(
        join_trust_state(Some(&trust), &alex),
        TrustState::SessionOnly
    );
    assert_eq!(join_trust_state(Some(&trust), &riley), TrustState::Trusted);
    assert_eq!(
        list_remembered_listeners(&client, &host)
            .expect("list")
            .len(),
        1
    );

    let other = load_host_trust(&client, &other_host).expect("other host");
    assert_eq!(
        join_trust_state(Some(&other), &riley),
        TrustState::SessionOnly
    );
    assert!(!revoke_listener(&client, &other_host, &riley, 40).expect("other revoke"));

    enroll_listener(&client, &host, &alex, "Alex", 50).expect("re-enrol");
    let trust = load_host_trust(&client, &host).expect("reload");
    assert_eq!(join_trust_state(Some(&trust), &alex), TrustState::Trusted);
    drop(client);
    worker.stop_and_join().expect("close");
}

#[test]
fn claimed_ids_are_never_remembered_or_trusted() {
    let (_directory, worker) = worker();
    let client = worker.client();
    let host = spki(3);
    let claimed = DeviceId::new("claimed-listener").expect("id");
    assert_eq!(
        enroll_listener(&client, &host, &claimed, "Claimed", 1),
        Err(TrustStoreError::NotKeyBound)
    );
    assert_eq!(
        revoke_listener(&client, &host, &claimed, 1),
        Err(TrustStoreError::NotKeyBound)
    );
    assert!(
        list_remembered_listeners(&client, &host)
            .expect("list")
            .is_empty()
    );
    let trust = load_host_trust(&client, &host).expect("load");
    assert_eq!(
        join_trust_state(Some(&trust), &claimed),
        TrustState::SessionOnly
    );
    assert_eq!(
        join_trust_state(None, &key_bound_device_id(&[1; 32])),
        TrustState::SessionOnly,
        "no registry loaded trusts nobody"
    );
    assert!(matches!(
        load_host_trust(&client, b"not a key"),
        Err(TrustStoreError::Trust(_))
    ));
    drop(client);
    worker.stop_and_join().expect("close");
}
