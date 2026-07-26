from pathlib import Path

path = Path("rust/silent-disco-ffi/src/android_database_abi.rs")
text = path.read_text()
old = "struct JniSettingsFields {"
new = "#[derive(Clone, Copy)]\nstruct JniSettingsFields {"
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected one JniSettingsFields declaration, found {count}")
path.write_text(text.replace(old, new))
