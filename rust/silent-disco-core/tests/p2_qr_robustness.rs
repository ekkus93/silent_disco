use std::path::Path;

use silent_disco_core::p2::{P2Store, current_unix_millis};

fn test_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "silent-disco-p2-qr-robustness-{}-{}.sqlite3",
        std::process::id(),
        current_unix_millis()
    ))
}

fn remove_database(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn malformed_qr_inputs_never_panic_or_validate() {
    let path = test_path();
    let mut store = P2Store::open(&path).expect("open P2 store");
    let now = 2_000_000_000_u64;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;

    for length in 0..2_048_usize {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push((state & 0x7f) as u8);
        }
        let payload = String::from_utf8(bytes).expect("ASCII generator");
        assert!(
            store.validate_and_consume_qr(&payload, now).is_err(),
            "random malformed payload unexpectedly validated at length {length}",
        );
    }

    drop(store);
    remove_database(&path);
}
