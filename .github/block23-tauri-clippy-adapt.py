#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one replacement target, found {count}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_once(
    "desktop/src-tauri/src/host_session_dto.rs",
    "let last_contact_ms = value.last_contact.map(|time| time.get());",
    "let last_contact_ms = value.last_contact.map(MonotonicMillis::get);",
    "host-session redundant closure",
)

replace_once(
    "desktop/src-tauri/src/platform/host_transport.rs",
    """                run_transport_worker(
                    node,
                    advertisement,
                    sink,
                    effect_receiver,
                    &worker_stop,
                    &worker_status,
                )""",
    """                run_transport_worker(
                    node,
                    &advertisement,
                    &sink,
                    &effect_receiver,
                    &worker_stop,
                    &worker_status,
                )""",
    "host-transport worker call ownership",
)
replace_once(
    "desktop/src-tauri/src/platform/host_transport.rs",
    """fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: SessionAdvertisement,
    sink: Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: Receiver<TransportEffect>,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {""",
    """fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: &Receiver<TransportEffect>,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {""",
    "host-transport worker ownership signature",
)
replace_once(
    "desktop/src-tauri/src/platform/host_transport.rs",
    """            process_effects(&*node, &*sink, &effect_receiver, status)""",
    """            process_effects(&*node, &**sink, effect_receiver, status)""",
    "host-transport effect references",
)
replace_once(
    "desktop/src-tauri/src/platform/host_transport.rs",
    """                &advertisement,
                &*sink,""",
    """                advertisement,
                &**sink,""",
    "host-transport event references",
)
replace_once(
    "desktop/src-tauri/src/platform/host_transport.rs",
    """    let drain_error = fail_queued_effects(&effect_receiver, &*sink, status).err();""",
    """    let drain_error = fail_queued_effects(effect_receiver, &**sink, status).err();""",
    "host-transport drain references",
)

replace_once(
    "desktop/src-tauri/src/platform/storage_effect_runner.rs",
    ".spawn(move || run_worker(inbox.receiver, sink, database, accepting))",
    ".spawn(move || run_worker(&inbox.receiver, &sink, &database, &accepting))",
    "storage worker call ownership",
)
replace_once(
    "desktop/src-tauri/src/platform/storage_effect_runner.rs",
    """fn run_worker(
    receiver: Receiver<StorageEffect>,
    sink: Arc<dyn DesktopStorageEventSink>,
    database: DatabaseClient,
    accepting: Arc<AtomicBool>,
) -> Result<(), CoreError> {""",
    """fn run_worker(
    receiver: &Receiver<StorageEffect>,
    sink: &Arc<dyn DesktopStorageEventSink>,
    database: &DatabaseClient,
    accepting: &AtomicBool,
) -> Result<(), CoreError> {""",
    "storage worker ownership signature",
)
replace_once(
    "desktop/src-tauri/src/platform/storage_effect_runner.rs",
    "let event = execute_effect(&database, effect);",
    "let event = execute_effect(database, effect);",
    "storage database reference",
)
replace_once(
    "desktop/src-tauri/src/platform/storage_effect_runner.rs",
    """fn persist_trusted_device(
    database: &DatabaseClient,
    device_id: silent_disco_core::domain::DeviceId,
    display_name: String,
) -> Result<StorageCompletion, StorageError> {
    let now_ms = unix_time_ms();
    let existing = database.get_trusted_device(&device_id)?;
    let device = existing.map_or_else(
        || TrustedDevice {
            device_id: device_id.clone(),
            display_name: display_name.clone(),
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            updated_at_ms: now_ms,
        },
        |mut current| {
            current.display_name = display_name.clone();
            current.trust_state = TrustState::Trusted;
            current.last_seen_ms = now_ms.max(current.last_seen_ms);
            current.updated_at_ms = now_ms.max(current.last_seen_ms);
            current
        },
    );
    database.upsert_trusted_device(&device)?;
    Ok(StorageCompletion::TrustedDeviceUpdated { device_id })
}""",
    """fn persist_trusted_device(
    database: &DatabaseClient,
    device_id: silent_disco_core::domain::DeviceId,
    display_name: String,
) -> Result<StorageCompletion, StorageError> {
    let now_ms = unix_time_ms();
    let existing = database.get_trusted_device(&device_id)?;
    let device = match existing {
        Some(mut current) => {
            current.display_name = display_name;
            current.trust_state = TrustState::Trusted;
            current.last_seen_ms = now_ms.max(current.last_seen_ms);
            current.updated_at_ms = now_ms.max(current.last_seen_ms);
            current
        }
        None => TrustedDevice {
            device_id: device_id.clone(),
            display_name,
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            updated_at_ms: now_ms,
        },
    };
    database.upsert_trusted_device(&device)?;
    Ok(StorageCompletion::TrustedDeviceUpdated { device_id })
}""",
    "trusted-device display-name ownership",
)

print("adapted generated Block 23 Tauri sources for warning-free ownership and projection code")
