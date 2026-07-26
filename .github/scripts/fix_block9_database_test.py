from pathlib import Path

path = Path("rust/silent-disco-core/src/storage/database.rs")
text = path.read_text(encoding="utf-8")
old = "assert_eq!(metadata.applied_migrations.len(), 1);"
new = '''assert_eq!(
            metadata.applied_migrations.len(),
            usize::try_from(LATEST_SCHEMA_VERSION).expect("schema version fits usize"),
        );'''
if text.count(old) != 1:
    raise RuntimeError(f"expected one database migration-count assertion, found {text.count(old)}")
path.write_text(text.replace(old, new), encoding="utf-8")
