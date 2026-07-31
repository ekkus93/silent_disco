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

# Match DesktopCoreObserver::new's current constructor order.
replace_once(
    'desktop/src-tauri/src/app_state.rs',
    '''    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        storage_dispatcher.clone(),
        Arc::clone(&network),
    );''',
    '''    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        Arc::clone(&network),
        storage_dispatcher.clone(),
    );''',
)
'''
PAYLOAD.write_text(source, encoding='utf-8')
print(
    'adapted Block 23 frontend client payload: removed 3 stale calls, '
    'appended 3 current-layout client patches, corrected the first '
    'host-session DTO import, and aligned the observer constructor order'
)
