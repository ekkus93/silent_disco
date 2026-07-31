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
