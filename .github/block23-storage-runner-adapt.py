#!/usr/bin/env python3
from pathlib import Path

path = Path('desktop/src-tauri/src/platform/storage_effect_runner.rs')
text = path.read_text(encoding='utf-8')

replacements = [
    (
        'use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};',
        'use std::sync::mpsc::{\n'
        '    Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel,\n'
        '};',
    ),
    (
        'use std::time::{SystemTime, UNIX_EPOCH};',
        'use std::time::{Duration, SystemTime, UNIX_EPOCH};',
    ),
    (
        'const STORAGE_EFFECT_QUEUE_CAPACITY: usize = 16;',
        'const STORAGE_EFFECT_QUEUE_CAPACITY: usize = 16;\n'
        'const STORAGE_EFFECT_POLL_INTERVAL: Duration = Duration::from_millis(50);',
    ),
    (
        '''        let join = thread::Builder::new()
            .name("silent-disco-desktop-storage-effects".to_owned())
            .spawn(move || run_worker(inbox.receiver, sink, database))''',
        '''        let accepting = Arc::clone(&dispatcher.accepting);
        let join = thread::Builder::new()
            .name("silent-disco-desktop-storage-effects".to_owned())
            .spawn(move || run_worker(inbox.receiver, sink, database, accepting))''',
    ),
    (
        '''fn run_worker(
    receiver: Receiver<StorageEffect>,
    sink: Arc<dyn DesktopStorageEventSink>,
    database: DatabaseClient,
) -> Result<(), CoreError> {
    while let Ok(effect) = receiver.recv() {
        let event = execute_effect(&database, effect);
        sink.submit_storage_event(event)?;
    }
    Ok(())
}''',
        '''fn run_worker(
    receiver: Receiver<StorageEffect>,
    sink: Arc<dyn DesktopStorageEventSink>,
    database: DatabaseClient,
    accepting: Arc<AtomicBool>,
) -> Result<(), CoreError> {
    loop {
        match receiver.recv_timeout(STORAGE_EFFECT_POLL_INTERVAL) {
            Ok(effect) => {
                let event = execute_effect(&database, effect);
                sink.submit_storage_event(event)?;
            }
            Err(RecvTimeoutError::Timeout) if accepting.load(Ordering::Acquire) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}''',
    ),
]

for old, new in replacements:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(
            f'storage_effect_runner.rs: expected one ownership anchor, found {count}: {old[:80]!r}'
        )
    text = text.replace(old, new, 1)

path.write_text(text, encoding='utf-8')
print(
    'adapted generated storage-effect runner: shutdown now uses a shared stop flag '
    'and bounded receiver polling, so external dispatcher clones cannot retain the worker'
)

admission_path = Path('desktop/src-tauri/src/platform/host_transport_admission_tests.rs')
admission = admission_path.read_text(encoding='utf-8')
old_delivery_read = '''    let delivery = receiver.recv_timeout(TEST_TIMEOUT).expect("delivery");
    assert!(matches!(
        delivery,
        CoreTransportEvent::DeliveryCompleted { report, .. }
            if report.intended_peers == 1
                && report.successful_peers == 1
                && report.failed_peers == 0
    ));'''
new_delivery_read = '''    let delivery = wait_for_delivery(&receiver);
    assert!(matches!(
        delivery,
        CoreTransportEvent::DeliveryCompleted { report, .. }
            if report.intended_peers == 1
                && report.successful_peers == 1
                && report.failed_peers == 0
    ));'''
count = admission.count(old_delivery_read)
if count != 1:
    raise RuntimeError(
        'host_transport_admission_tests.rs: expected one successful delivery read anchor, '
        f'found {count}'
    )
admission = admission.replace(old_delivery_read, new_delivery_read, 1)
helper_anchor = '''fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&ControlMessage) -> bool,
) {'''
helper = '''fn wait_for_delivery(
    receiver: &mpsc::Receiver<CoreTransportEvent>,
) -> CoreTransportEvent {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = receiver
            .recv_timeout(remaining)
            .expect("delivery event before timeout");
        if matches!(event, CoreTransportEvent::DeliveryCompleted { .. }) {
            return event;
        }
        assert!(Instant::now() < deadline, "timed out waiting for delivery event");
    }
}

fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&ControlMessage) -> bool,
) {'''
count = admission.count(helper_anchor)
if count != 1:
    raise RuntimeError(
        'host_transport_admission_tests.rs: expected one wait_for_control anchor, '
        f'found {count}'
    )
admission = admission.replace(helper_anchor, helper, 1)
admission_path.write_text(admission, encoding='utf-8')
print(
    'adapted generated approval-delivery test: ignore earlier join events and wait for '
    'the authoritative DeliveryCompleted event'
)

# Keep the generated Rust/TypeScript contract and Block 23 fixtures aligned.
bindings_path = Path('desktop/src-tauri/src/bindings.rs')
bindings = bindings_path.read_text(encoding='utf-8')
bindings_import_old = (
    '    ConnectedListenerDto, HostConnectionDto, HostSessionSnapshotDto, PendingJoinRequestDto,\n'
)
bindings_import_new = (
    '    ConnectedListenerDto, DeliveryReportDto, HostConnectionDto, HostSessionSnapshotDto,\n'
    '    PendingJoinRequestDto,\n'
)
if bindings.count(bindings_import_old) != 1:
    raise RuntimeError('bindings.rs: expected one host-session DTO import anchor')
bindings = bindings.replace(bindings_import_old, bindings_import_new, 1)
bindings_declaration_old = (
    '        declaration::<ConnectedListenerDto>(&config),\n'
    '        declaration::<HostSessionSnapshotDto>(&config),\n'
)
bindings_declaration_new = (
    '        declaration::<ConnectedListenerDto>(&config),\n'
    '        declaration::<DeliveryReportDto>(&config),\n'
    '        declaration::<HostSessionSnapshotDto>(&config),\n'
)
if bindings.count(bindings_declaration_old) != 1:
    raise RuntimeError('bindings.rs: expected one delivery declaration anchor')
bindings = bindings.replace(bindings_declaration_old, bindings_declaration_new, 1)
bindings_test_old = (
    '        assert!(first.contains("export type CommandReceiptDto"));\n'
)
bindings_test_new = (
    '        assert!(first.contains("export type CommandReceiptDto"));\n'
    '        assert!(first.contains("export type DeliveryReportDto"));\n'
)
if bindings.count(bindings_test_old) != 1:
    raise RuntimeError('bindings.rs: expected one binding assertion anchor')
bindings = bindings.replace(bindings_test_old, bindings_test_new, 1)
bindings_path.write_text(bindings, encoding='utf-8')

app_test_path = Path('desktop/src/App.test.tsx')
app_test = app_test_path.read_text(encoding='utf-8')
app_fixture_old = '''  pendingJoinRequests: [],
  connectedListeners: [],
  playbackControlsEnabled: false,'''
app_fixture_new = '''  pendingJoinRequests: [],
  connectedListeners: [],
  lastDelivery: null,
  recoverableAction: null,
  playbackControlsEnabled: false,'''
if app_test.count(app_fixture_old) != 1:
    raise RuntimeError('App.test.tsx: expected one host-session fixture anchor')
app_test = app_test.replace(app_fixture_old, app_fixture_new, 1)
app_test_path.write_text(app_test, encoding='utf-8')

screen_path = Path('desktop/src/screens/HostSessionScreen.tsx')
screen = screen_path.read_text(encoding='utf-8')
synthetic_error_old = '''      message: "The control message was not delivered to the listener.",
      operationId: null,
      context: [],
'''
synthetic_error_new = '''      message: "The control message was not delivered to the listener.",
'''
if screen.count(synthetic_error_old) != 1:
    raise RuntimeError('HostSessionScreen.tsx: expected one stale synthetic-error shape')
screen = screen.replace(synthetic_error_old, synthetic_error_new, 1)
screen_path.write_text(screen, encoding='utf-8')

screen_test_path = Path('desktop/src/screens/HostSessionScreen.test.tsx')
screen_test = screen_test_path.read_text(encoding='utf-8')
invoke_error_old = '''  message: "join request is stale or no longer pending",
  operationId: "command-9",
  context: [],
'''
invoke_error_new = '''  message: "join request is stale or no longer pending",
'''
if screen_test.count(invoke_error_old) != 1:
    raise RuntimeError('HostSessionScreen.test.tsx: expected one stale invoke-error shape')
screen_test = screen_test.replace(invoke_error_old, invoke_error_new, 1)
delivery_error_old = '''            message: "transport control delivery had no intended recipients",
            operationId: "command-1",
            context: [],
'''
delivery_error_new = '''            message: "transport control delivery had no intended recipients",
'''
if screen_test.count(delivery_error_old) != 1:
    raise RuntimeError('HostSessionScreen.test.tsx: expected one stale delivery-error shape')
screen_test = screen_test.replace(delivery_error_old, delivery_error_new, 1)
listener_spread_old = '              ...fixture().connectedListeners[0],\n'
listener_spread_new = '              ...fixture().connectedListeners[0]!,\n'
if screen_test.count(listener_spread_old) != 1:
    raise RuntimeError('HostSessionScreen.test.tsx: expected one indexed listener spread')
screen_test = screen_test.replace(listener_spread_old, listener_spread_new, 1)
screen_test_path.write_text(screen_test, encoding='utf-8')

host_dto_path = Path('desktop/src-tauri/src/host_session_dto.rs')
host_dto = host_dto_path.read_text(encoding='utf-8')
from_parts_old = '    pub fn from_parts(\n'
from_parts_new = '    pub(crate) fn from_parts(\n'
if host_dto.count(from_parts_old) != 1:
    raise RuntimeError('host_session_dto.rs: expected one projection visibility anchor')
host_dto = host_dto.replace(from_parts_old, from_parts_new, 1)
host_dto_path.write_text(host_dto, encoding='utf-8')

transport_path = Path('desktop/src-tauri/src/platform/host_transport.rs')
transport = transport_path.read_text(encoding='utf-8')
for method in ('start', 'status', 'shutdown'):
    old = f'    pub(crate) fn {method}'
    new = f'    pub(super) fn {method}'
    if transport.count(old) != 1:
        raise RuntimeError(
            f'host_transport.rs: expected one {method} visibility anchor'
        )
    transport = transport.replace(old, new, 1)
transport_path.write_text(transport, encoding='utf-8')

print(
    'adapted generated Block 23 frontend contract: exported DeliveryReportDto, '
    'aligned DesktopErrorDto shapes, completed authoritative fixtures, retained '
    'strict listener typing, and narrowed internal Rust interfaces'
)

# Align App integration expectations with the loaded authoritative host-session UI
# and keep the listener transition fixture type-safe without non-null assertions.
app_test_path = Path('desktop/src/App.test.tsx')
app_test = app_test_path.read_text(encoding='utf-8')
loaded_heading_old = '    expect(await screen.findByRole("heading", { name: "Host session" })).toBeVisible();'
loaded_heading_new = '    expect(await screen.findByRole("heading", { name: "Oakland Night" })).toBeVisible();'
if app_test.count(loaded_heading_old) != 1:
    raise RuntimeError(
        'App.test.tsx: expected one stale loading-heading assertion'
    )
app_test = app_test.replace(loaded_heading_old, loaded_heading_new, 1)
app_test_path.write_text(app_test, encoding='utf-8')

screen_test_path = Path('desktop/src/screens/HostSessionScreen.test.tsx')
screen_test = screen_test_path.read_text(encoding='utf-8')
listener_fixture_old = '''            {
              ...fixture().connectedListeners[0]!,
              deviceId: "listener-1",
              displayName: "Listener One",
              trustState: "session_only",
            },'''
listener_fixture_new = '''            {
              deviceId: "listener-1",
              displayName: "Listener One",
              trustState: "session_only",
              transportState: "connected",
              lastContactMs: "500",
              lastContactAgeMs: "250",
              syncConfidence: "good",
              estimatedOffsetMs: "-2.5",
              roundTripTimeMs: "18",
              driftPpm: "1.2",
              lastControlDeliveryState: "ok",
              retryAvailable: false,
              resyncAvailable: false,
              canRemove: true,
              lastError: null,
            },'''
if screen_test.count(listener_fixture_old) != 1:
    raise RuntimeError(
        'HostSessionScreen.test.tsx: expected one non-null listener fixture spread'
    )
screen_test = screen_test.replace(listener_fixture_old, listener_fixture_new, 1)
screen_test_path.write_text(screen_test, encoding='utf-8')

print(
    'adapted generated Block 23 frontend tests: assert the loaded authoritative '
    'session-name heading and use an explicit typed connected-listener fixture'
)

# Keep the generated Tauri tree warning-free under `cargo clippy -D warnings`.
def replace_clippy_once(path_name: str, old: str, new: str, label: str) -> None:
    target = Path(path_name)
    source = target.read_text(encoding='utf-8')
    count = source.count(old)
    if count != 1:
        raise RuntimeError(f'{label}: expected one replacement target, found {count}')
    target.write_text(source.replace(old, new, 1), encoding='utf-8')

replace_clippy_once(
    'desktop/src-tauri/src/host_session_dto.rs',
    'let last_contact_ms = value.last_contact.map(|time| time.get());',
    'let last_contact_ms = value.last_contact.map(silent_disco_core::domain::MonotonicMillis::get);',
    'host-session redundant closure',
)

replace_clippy_once(
    'desktop/src-tauri/src/platform/host_transport.rs',
    '''                run_transport_worker(
                    node,
                    advertisement,
                    sink,
                    effect_receiver,
                    &worker_stop,
                    &worker_status,
                )''',
    '''                run_transport_worker(
                    node,
                    &advertisement,
                    &sink,
                    &effect_receiver,
                    &worker_stop,
                    &worker_status,
                )''',
    'host-transport worker call ownership',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/host_transport.rs',
    '''fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: SessionAdvertisement,
    sink: Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: Receiver<TransportEffect>,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {''',
    '''fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: &Receiver<TransportEffect>,
    stop: &AtomicBool,
    status: &SharedStatus,
) -> Result<(), DesktopNetworkError> {''',
    'host-transport worker ownership signature',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/host_transport.rs',
    '            process_effects(&*node, &*sink, &effect_receiver, status)',
    '            process_effects(&*node, &**sink, effect_receiver, status)',
    'host-transport effect references',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/host_transport.rs',
    '''                &advertisement,
                &*sink,''',
    '''                advertisement,
                &**sink,''',
    'host-transport event references',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/host_transport.rs',
    '    let drain_error = fail_queued_effects(&effect_receiver, &*sink, status).err();',
    '    let drain_error = fail_queued_effects(effect_receiver, &**sink, status).err();',
    'host-transport drain references',
)

replace_clippy_once(
    'desktop/src-tauri/src/platform/storage_effect_runner.rs',
    '.spawn(move || run_worker(inbox.receiver, sink, database, accepting))',
    '.spawn(move || run_worker(&inbox.receiver, &sink, &database, &accepting))',
    'storage worker call ownership',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/storage_effect_runner.rs',
    '''fn run_worker(
    receiver: Receiver<StorageEffect>,
    sink: Arc<dyn DesktopStorageEventSink>,
    database: DatabaseClient,
    accepting: Arc<AtomicBool>,
) -> Result<(), CoreError> {''',
    '''fn run_worker(
    receiver: &Receiver<StorageEffect>,
    sink: &Arc<dyn DesktopStorageEventSink>,
    database: &DatabaseClient,
    accepting: &AtomicBool,
) -> Result<(), CoreError> {''',
    'storage worker ownership signature',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/storage_effect_runner.rs',
    'let event = execute_effect(&database, effect);',
    'let event = execute_effect(database, effect);',
    'storage database reference',
)
replace_clippy_once(
    'desktop/src-tauri/src/platform/storage_effect_runner.rs',
    '''fn persist_trusted_device(
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
}''',
    '''fn persist_trusted_device(
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
}''',
    'trusted-device display-name ownership',
)

print('adapted generated Block 23 Tauri sources for warning-free ownership and projection code')
