#!/usr/bin/env python3
from __future__ import annotations

import ast
from pathlib import Path

PAYLOAD = Path('.github/apply-block23.py')
TARGET = 'desktop/src/core/client.ts'


def remove_replace_calls(source: str) -> tuple[str, int]:
    tree = ast.parse(source)
    ranges: list[tuple[int, int]] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Expr) or not isinstance(node.value, ast.Call):
            continue
        call = node.value
        if not isinstance(call.func, ast.Name) or call.func.id != 'replace_once' or not call.args:
            continue
        first = call.args[0]
        if isinstance(first, ast.Constant) and first.value == TARGET:
            if node.end_lineno is None:
                raise SystemExit('client replacement call has no end line')
            ranges.append((node.lineno, node.end_lineno))
    lines = source.splitlines(keepends=True)
    for start, end in sorted(ranges, reverse=True):
        del lines[start - 1 : end]
    return ''.join(lines), len(ranges)


def emit(old: str, new: str) -> str:
    return f'replace_once({TARGET!r}, {old!r}, {new!r})\n'


source = PAYLOAD.read_text(encoding='utf-8')
source, removed = remove_replace_calls(source)
if removed != 3:
    raise SystemExit(f'expected three stale client replacements, found {removed}')

patches = [
    emit(
        '  AttachNotificationResponse,\n  BridgeLifecycleDto,',
        '  ApproveJoinRequest,\n  AttachNotificationResponse,\n  BridgeLifecycleDto,',
    ),
    emit(
        '  HostSessionSnapshotDto,\n  OpenProfileRequest,',
        '  HostSessionSnapshotDto,\n  JoinRequestCommandRequest,\n  ListenerCommandRequest,\n  OpenProfileRequest,',
    ),
    emit(
        'export async function endHostSession(expectedRevision: string): Promise<CommandReceiptDto> {',
        '''export async function approveJoinRequest(
  request: ApproveJoinRequest,
): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("approve_join_request", { request });
}

export async function rejectJoinRequest(
  request: JoinRequestCommandRequest,
): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("reject_join_request", { request });
}

export async function removeListener(
  request: ListenerCommandRequest,
): Promise<CommandReceiptDto> {
  return invokeDesktop<CommandReceiptDto>("remove_listener", { request });
}

export async function endHostSession(expectedRevision: string): Promise<CommandReceiptDto> {''',
    ),
]
source += '\n# Current frontend client compatibility patches.\n' + ''.join(patches)
source += '''
# Correct only the first host-session DTO import. The same text also occurs in
# a generated test fixture, so replace_once's global uniqueness guard is not
# appropriate for this specific compatibility correction.
dto_path = Path('desktop/src-tauri/src/host_session_dto.rs')
dto_old = 'use crate::platform::network::ActiveHostSessionSnapshot;'
dto_new = 'use crate::platform::host_transport::ActiveHostSessionSnapshot;'
dto_text = dto_path.read_text(encoding='utf-8')
if dto_old not in dto_text:
    raise RuntimeError('host_session_dto.rs: stale snapshot import not found')
dto_path.write_text(dto_text.replace(dto_old, dto_new, 1), encoding='utf-8')
'''

observer_old = """    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        storage_dispatcher.clone(),
        Arc::clone(&network),
    );"""
observer_new = """    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        Arc::clone(&network),
        storage_dispatcher.clone(),
    );"""
source += (
    '\n# Match DesktopCoreObserver::new\'s current constructor order.\n'
    + f"replace_once('desktop/src-tauri/src/app_state.rs', {observer_old!r}, {observer_new!r})\n"
)

source += r'''
# Remove the obsolete playback-effect match arm. Playback is not represented by
# TransportEffectRequest in the current core; Block 23 owns only approval,
# rejection, and listener-disconnect delivery effects.
transport_path = Path('desktop/src-tauri/src/platform/host_transport.rs')
transport_lines = transport_path.read_text(encoding='utf-8').splitlines(keepends=True)
playback_variants = (
    'TransportEffectRequest::StartHostPlayback',
    'TransportEffectRequest::PauseHostPlayback',
    'TransportEffectRequest::ResumeHostPlayback',
    'TransportEffectRequest::StopHostPlayback',
)
start_matches = [
    index
    for index, line in enumerate(transport_lines)
    if playback_variants[0] in line
]
if len(start_matches) != 1:
    raise RuntimeError(
        f'host_transport.rs: expected one obsolete playback arm, found {len(start_matches)}'
    )
playback_start = start_matches[0]
playback_arrow = next(
    (
        index
        for index in range(playback_start, len(transport_lines))
        if '=> {' in transport_lines[index]
    ),
    None,
)
if playback_arrow is None:
    raise RuntimeError('host_transport.rs: obsolete playback arm has no body')
playback_header = ''.join(transport_lines[playback_start : playback_arrow + 1])
for variant in playback_variants:
    if variant not in playback_header:
        raise RuntimeError(f'host_transport.rs: obsolete playback arm is missing {variant}')

brace_depth = 0
body_started = False
playback_end = None
for index in range(playback_arrow, len(transport_lines)):
    segment = transport_lines[index]
    if index == playback_arrow:
        segment = segment.split('=>', 1)[1]
    for character in segment:
        if character == '{':
            brace_depth += 1
            body_started = True
        elif character == '}':
            brace_depth -= 1
            if brace_depth < 0:
                raise RuntimeError('host_transport.rs: malformed obsolete playback arm')
    if body_started and brace_depth == 0:
        playback_end = index
        break
if playback_end is None:
    raise RuntimeError('host_transport.rs: obsolete playback arm body is unterminated')

del transport_lines[playback_start : playback_end + 1]
transport_text = ''.join(transport_lines)
if any(variant in transport_text for variant in playback_variants):
    raise RuntimeError('host_transport.rs: obsolete playback variants remain after correction')
transport_path.write_text(transport_text, encoding='utf-8')

# The delivery packet owns listener_id while send_pending_control also borrows
# the routing identifier for the duration of the same call. Borrow a temporary
# clone for routing so the original value can move into the packet exactly once.
transport_text = transport_path.read_text(encoding='utf-8')
pending_listener_borrow = "node.send_pending_control(\n            &listener_id,"
cloned_pending_listener_borrow = "node.send_pending_control(\n            &listener_id.clone(),"
pending_borrow_count = transport_text.count(pending_listener_borrow)
if pending_borrow_count != 3:
    raise RuntimeError(
        f'host_transport.rs: expected three pending listener borrows, found {pending_borrow_count}'
    )
transport_text = transport_text.replace(
    pending_listener_borrow,
    cloned_pending_listener_borrow,
)
transport_path.write_text(transport_text, encoding='utf-8')

# Restore the current settings-persistence effect that predates Block 23's new
# trusted-device persistence path.
storage_path = Path('desktop/src-tauri/src/platform/storage_effect_runner.rs')
storage_text = storage_path.read_text(encoding='utf-8')
storage_import_old = (
    'use silent_disco_core::storage::{DatabaseClient, StorageError, TrustedDevice};'
)
storage_import_new = (
    'use silent_disco_core::storage::{'
    'DatabaseClient, StorageError, StoredSettings, TrustedDevice};'
)
if storage_text.count(storage_import_old) != 1:
    raise RuntimeError('storage_effect_runner.rs: expected one storage import anchor')
storage_text = storage_text.replace(storage_import_old, storage_import_new, 1)

trusted_match_anchor = """        StorageEffectRequest::PersistTrustedDevice {
            device_id,
            display_name,
        } => persist_trusted_device(database, device_id, display_name),"""
settings_and_trusted_match = """        StorageEffectRequest::PersistSettings { settings } => {
            persist_settings(database, settings)
        }
        StorageEffectRequest::PersistTrustedDevice {
            device_id,
            display_name,
        } => persist_trusted_device(database, device_id, display_name),"""
if storage_text.count(trusted_match_anchor) != 1:
    raise RuntimeError('storage_effect_runner.rs: expected one trusted-device match anchor')
storage_text = storage_text.replace(
    trusted_match_anchor,
    settings_and_trusted_match,
    1,
)

trusted_helper_anchor = """fn persist_trusted_device(
    database: &DatabaseClient,"""
settings_helper = """fn persist_settings(
    database: &DatabaseClient,
    settings: silent_disco_core::domain::TuningSettings,
) -> Result<StorageCompletion, StorageError> {
    database.save_settings(&StoredSettings {
        tuning: settings,
        updated_at_ms: unix_time_ms(),
    })?;
    Ok(StorageCompletion::SettingsSaved)
}

fn persist_trusted_device(
    database: &DatabaseClient,"""
if storage_text.count(trusted_helper_anchor) != 1:
    raise RuntimeError('storage_effect_runner.rs: expected one trusted-device helper anchor')
storage_text = storage_text.replace(trusted_helper_anchor, settings_helper, 1)

display_name_move = '            current.display_name = display_name;'
display_name_clone = '            current.display_name = display_name.clone();'
if storage_text.count(display_name_move) != 1:
    raise RuntimeError(
        'storage_effect_runner.rs: expected one trusted-device display-name move'
    )
storage_text = storage_text.replace(display_name_move, display_name_clone, 1)
storage_path.write_text(storage_text, encoding='utf-8')
'''

PAYLOAD.write_text(source, encoding='utf-8')
print(
    'adapted Block 23 frontend/current-layout payload: removed 3 stale client calls, '
    'appended 3 current-layout client patches, corrected the first host-session DTO '
    'import, aligned the observer constructor order, removed the obsolete playback '
    'transport-effect arm, fixed three listener routing borrow/move conflicts, '
    'restored settings persistence, and fixed the trusted-device display-name borrow'
)
