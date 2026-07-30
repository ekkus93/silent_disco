#!/usr/bin/env python3
import subprocess

BASE_INSTALLER_COMMIT = "34e5d932c8001ef079b05a5b6337799990fb0a96"
installer = subprocess.check_output(
    [
        "git",
        "show",
        f"{BASE_INSTALLER_COMMIT}:scripts/apply-desktop-block17.py",
    ],
    text=True,
)
installer = installer.replace('sha2 = "=0.10.9"', 'sha2 = "=0.11.0"')
installer = installer.replace(
    '"pub(crate) mod profile_lock;\\n",',
    '"pub mod profile_lock;\\n",',
)
installer = installer.replace(
    '"pub(crate) mod profile_lock;\\npub(crate) mod source_staging;\\npub(crate) mod source_staging_control;\\n",',
    '"pub mod profile_lock;\\npub(crate) mod source_staging;\\npub(crate) mod source_staging_control;\\n",',
)
exec(compile(installer, "scripts/apply-desktop-block17.py", "exec"))
