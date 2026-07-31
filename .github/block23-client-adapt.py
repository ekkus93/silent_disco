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
PAYLOAD.write_text(source, encoding='utf-8')
print('adapted Block 23 frontend client payload: removed 3 stale calls and appended 3 current-layout patches')
