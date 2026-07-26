from pathlib import Path

path = Path("app/src/main/java/com/ekkus/silentdisco/core/persistence/AndroidStorageRepository.kt")
text = path.read_text(encoding="utf-8")
old = "import com.ekkus.silentdisco.core.rust.RustLegacyImportOutcome\n"
new = "import com.ekkus.silentdisco.core.rust.LEGACY_ANDROID_IMPORT_VERSION\nimport com.ekkus.silentdisco.core.rust.RustLegacyImportOutcome\n"
if text.count(old) != 1:
    raise RuntimeError(f"expected one import anchor, found {text.count(old)}")
path.write_text(text.replace(old, new), encoding="utf-8")
