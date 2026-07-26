from __future__ import annotations

import base64
import zlib
from pathlib import Path

WORKER = (
    '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'
)
Path("rust/silent-disco-core/src/storage/worker.rs").write_bytes(
    zlib.decompress(base64.b64decode(WORKER))
)

models_path = Path("rust/silent-disco-core/src/storage/models.rs")
models = models_path.read_text()
replacements = {
    ".map_err(|_| StorageModelValidationError::DisplayName)?;": ".map_err(|()| StorageModelValidationError::DisplayName)?;",
    ".map_err(|_| StorageModelValidationError::PrivateKeyReference)?;": ".map_err(|()| StorageModelValidationError::PrivateKeyReference)?;",
    ".map_err(|_| StorageModelValidationError::SessionName)?;": ".map_err(|()| StorageModelValidationError::SessionName)?;",
    ".map_err(|_| StorageModelValidationError::FailureCode)?;": ".map_err(|()| StorageModelValidationError::FailureCode)?;",
    ".map_err(|_| StorageModelValidationError::FailureMessage)?;": ".map_err(|()| StorageModelValidationError::FailureMessage)?;",
    """        if let Some(public_key) = &self.public_key {
            if public_key.is_empty() || public_key.len() > MAX_PUBLIC_KEY_BYTES {
                return Err(StorageModelValidationError::PublicKey);
            }
        }""": """        if let Some(public_key) = &self.public_key
            && (public_key.is_empty() || public_key.len() > MAX_PUBLIC_KEY_BYTES)
        {
            return Err(StorageModelValidationError::PublicKey);
        }""",
}
for old, new in replacements.items():
    count = models.count(old)
    if count != 1:
        raise SystemExit(f"expected one model repair match, found {count}: {old!r}")
    models = models.replace(old, new)
models_path.write_text(models)

settings_path = Path("rust/silent-disco-core/src/storage/settings_repository.rs")
settings = settings_path.read_text()
old = "struct RawSettings {"
if settings.count(old) != 1:
    raise SystemExit("RawSettings declaration changed unexpectedly")
settings_path.write_text(settings.replace(old, "#[derive(Clone, Copy)]\nstruct RawSettings {"))

Path(".github/scripts/fix_block8_clippy.py").unlink()
