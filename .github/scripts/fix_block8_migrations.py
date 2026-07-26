from pathlib import Path

path = Path("rust/silent-disco-core/src/storage/migrations.rs")
text = path.read_text()
replacements = {
    'const MIGRATION_V1_SQL: &str = r#"': 'const MIGRATION_V1_SQL: &str = r"',
    '\n"#;\n\n#[derive(Debug, Clone, Copy)]': '\n";\n\n#[derive(Debug, Clone, Copy)]',
    '    const BAD_MIGRATION_SQL: &str = r#"': '    const BAD_MIGRATION_SQL: &str = r"',
    '\n"#;\n\n    #[test]\n    fn compiled_catalog': '\n";\n\n    #[test]\n    fn compiled_catalog',
    'const fn fnv1a64(bytes: &[u8]) -> u64 {': 'fn fnv1a64(bytes: &[u8]) -> u64 {',
}
for old, new in replacements.items():
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one migration repair match, found {count}: {old!r}")
    text = text.replace(old, new)
path.write_text(text)
Path(".github/scripts/fix_block8_migrations.py").unlink()
