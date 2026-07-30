from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}: {old[:120]!r}")
    write(path, content.replace(old, new, 1))


replace_once(
    "desktop/src/screens/HostSetupScreen.test.tsx",
    """const selectHostRole = vi.fn();
const updateHostDraft = vi.fn();
const createHostSession = vi.fn();
""",
    """const { selectHostRole, updateHostDraft, createHostSession } = vi.hoisted(() => ({
  selectHostRole: vi.fn(),
  updateHostDraft: vi.fn(),
  createHostSession: vi.fn(),
}));
""",
)

replace_once(
    "desktop/src/app/selectors.ts",
    "import type { RootState } from \"./store\";\n",
    "import { createSelector } from \"@reduxjs/toolkit\";\n\nimport type { RootState } from \"./store\";\n",
)
replace_once(
    "desktop/src/app/selectors.ts",
    """export const selectPendingCommandReceipts = (state: RootState) =>
  Object.values(state.core.pendingCommandReceipts);
""",
    """const selectPendingCommandReceiptMap = (state: RootState) =>
  state.core.pendingCommandReceipts;
export const selectPendingCommandReceipts = createSelector(
  [selectPendingCommandReceiptMap],
  (receipts) => Object.values(receipts),
);
""",
)

(ROOT / "scripts/fix-block14-test-hoisting-and-selector.py").unlink()
