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
old_parameter = '''    fn open_profile_sync(
        &self,
        paths: DesktopProfilePaths,'''
new_parameter = '''    fn open_profile_sync(
        &self,
        paths: &DesktopProfilePaths,'''
if source.count(old_parameter) != 1:
    raise SystemExit("test open_profile_sync parameter precondition changed")
source = source.replace(old_parameter, new_parameter)

old_first_call = '''                paths,
                id,'''
new_first_call = '''                &paths,
                id,'''
if source.count(old_first_call) != 1:
    raise SystemExit("first test profile-path call precondition changed")
source = source.replace(old_first_call, new_first_call)

old_cloned_call = '''                    paths.clone(),'''
new_borrowed_call = '''                    &paths,'''
if source.count(old_cloned_call) != 4:
    raise SystemExit("cloned test profile-path call precondition changed")
source = source.replace(old_cloned_call, new_borrowed_call)

app_state.write_text(source)
