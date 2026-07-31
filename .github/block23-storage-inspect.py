#!/usr/bin/env python3
from pathlib import Path

payload = Path('.github/apply-block23.py')
source = payload.read_text(encoding='utf-8')
source += '''
print('=== BLOCK23 GENERATED STORAGE EFFECT RUNNER ===')
print(Path('desktop/src-tauri/src/platform/storage_effect_runner.rs').read_text(encoding='utf-8'))
print('=== END BLOCK23 GENERATED STORAGE EFFECT RUNNER ===')
'''
payload.write_text(source, encoding='utf-8')
print('enabled read-only generated storage-runner diagnostic')
