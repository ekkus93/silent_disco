#!/usr/bin/env python3
"""Normalize the second repair pass's exact call-site preconditions."""

from pathlib import Path


second_pass = Path("scripts/repair-desktop-block10-clippy.py")
source = second_pass.read_text()
old_count = '''    2,
    "open_runtime(&paths, profile_id",
)'''
new_count = '''    1,
    "open_runtime(&paths, profile_id",
)'''
if source.count(old_count) != 1:
    raise SystemExit("second repair pass production call-count precondition changed")
second_pass.write_text(source.replace(old_count, new_count))

app_state = Path("desktop/src-tauri/src/app_state.rs")
source = app_state.read_text()
old_call = "match open_runtime(paths, profile_id, provider, notifications) {"
new_call = "match open_runtime(&paths, profile_id, provider, notifications) {"
if source.count(old_call) != 1:
    raise SystemExit("test open_runtime call precondition changed")
app_state.write_text(source.replace(old_call, new_call))
