#!/usr/bin/env python3
from __future__ import annotations

import ast
from pathlib import Path

payload = Path('.github/apply-block23.py')
source = payload.read_text(encoding='utf-8')
tree = ast.parse(source)
for node in ast.walk(tree):
    if not isinstance(node, ast.Expr) or not isinstance(node.value, ast.Call):
        continue
    call = node.value
    if not isinstance(call.func, ast.Name) or call.func.id != 'replace_once' or len(call.args) < 3:
        continue
    try:
        path = ast.literal_eval(call.args[0])
    except (ValueError, TypeError):
        continue
    if path != 'desktop/src/core/client.ts':
        continue
    old = ast.literal_eval(call.args[1])
    new = ast.literal_eval(call.args[2])
    print('=== CLIENT REPLACEMENT ===')
    print('OLD_REPR=', repr(old))
    print('NEW_REPR=', repr(new))
