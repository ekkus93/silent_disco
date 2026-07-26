from pathlib import Path

path = Path("rust/silent-disco-ffi/src/android_storage.rs")
text = path.read_text(encoding="utf-8")
replacements = {
    "use core::ffi::c_void;\n": "",
    "#[derive(Debug)]\nstruct StorageEntry {": "struct StorageEntry {",
    "#[derive(Debug)]\nstruct StorageRegistry {": "struct StorageRegistry {",
    "        ANDROID_LEGACY_IMPORT_VERSION, DatabaseClient, DatabaseConfig, DatabaseWorker,\n": "        DatabaseClient, DatabaseConfig, DatabaseWorker,\n",
    "        .map_or(core::ptr::null_mut(), jni::objects::JObject::into_raw)": "        .map_or(core::ptr::null_mut(), |value| value.into_raw())",
    "    use serde_json::Value;\n\n    use super::{\n        ANDROID_LEGACY_IMPORT_VERSION, LegacyImportDto, SettingsDto, TrustedDeviceDto,\n": "    use serde_json::Value;\n    use silent_disco_core::storage::ANDROID_LEGACY_IMPORT_VERSION;\n\n    use super::{\n        LegacyImportDto, SettingsDto, TrustedDeviceDto,\n",
}
for old, new in replacements.items():
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match, found {count}: {old!r}")
    text = text.replace(old, new)
path.write_text(text, encoding="utf-8")
