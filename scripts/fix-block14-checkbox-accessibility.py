from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "desktop/src/screens/HostSetupScreen.tsx"
content = TARGET.read_text(encoding="utf-8")

label_position = content.index("Remember approved devices")
input_start = content.rfind("<input", 0, label_position)
input_end = content.index("/>", input_start) + 2
input_block = content[input_start:input_end]
if input_start < 0 or 'type="checkbox"' not in input_block:
    raise RuntimeError("could not locate the remember-approved checkbox")
if 'aria-label="Remember approved devices"' not in input_block:
    line_start = content.rfind("\n", 0, input_start) + 1
    indent = content[line_start:input_start]
    properties = (
        "<input\n"
        f'{indent}  aria-describedby="remember-approved-devices-help"\n'
        f'{indent}  aria-label="Remember approved devices"'
    )
    content = content[:input_start] + content[input_start:].replace("<input", properties, 1)

label_position = content.index("Remember approved devices")
help_text_position = content.index("Rust applies the trusted-device policy", label_position)
help_span_start = content.rfind("<span", label_position, help_text_position)
help_span_end = content.index(">", help_span_start)
help_opening = content[help_span_start:help_span_end]
if help_span_start < 0:
    raise RuntimeError("could not locate the remember-approved help text")
if 'id="remember-approved-devices-help"' not in help_opening:
    content = (
        content[:help_span_end]
        + ' id="remember-approved-devices-help"'
        + content[help_span_end:]
    )

TARGET.write_text(content, encoding="utf-8")
(ROOT / "scripts/fix-block14-checkbox-accessibility.py").unlink()
