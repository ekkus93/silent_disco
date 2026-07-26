from pathlib import Path

for filename in (
    "rust/silent-disco-core/src/storage/database.rs",
    "rust/silent-disco-core/src/storage/migrations.rs",
):
    path = Path(filename)
    text = path.read_text()
    old = "assert_eq!(metadata.applied_migrations.len(), 1);" if filename.endswith("database.rs") else "assert_eq!(first.records.len(), 1);"
    new = "assert_eq!(metadata.applied_migrations.len(), 2);" if filename.endswith("database.rs") else "assert_eq!(first.records.len(), 2);"
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{filename}: expected one migration-count assertion, found {count}")
    path.write_text(text.replace(old, new))
