from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "desktop/src/screens/HostSetupScreen.tsx"
content = TARGET.read_text(encoding="utf-8")

input_old = '''              <input
                className="mt-1"
                type="checkbox"
'''
input_new = '''              <input
                aria-describedby="remember-approved-devices-help"
                aria-label="Remember approved devices"
                className="mt-1"
                type="checkbox"
'''
if content.count(input_old) != 1:
    raise RuntimeError("expected one remember-approved checkbox")
content = content.replace(input_old, input_new, 1)

label_position = content.index("Remember approved devices")
help_old = '<span className="mt-1 block text-sm text-violet-100/65">'
help_position = content.index(help_old, label_position)
help_new = '''<span
                className="mt-1 block text-sm text-violet-100/65"
                id="remember-approved-devices-help"
              >'''
content = content[:help_position] + help_new + content[help_position + len(help_old) :]

TARGET.write_text(content, encoding="utf-8")
(ROOT / "scripts/fix-block14-checkbox-accessibility.py").unlink()
