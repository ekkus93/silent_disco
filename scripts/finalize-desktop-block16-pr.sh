#!/usr/bin/env bash
set -euo pipefail

: "${SOURCE_SHA:?SOURCE_SHA must be set}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID must be set}"

bash scripts/check-source-file-line-counts.sh

pushd desktop >/dev/null
npm install --package-lock-only --ignore-scripts
popd >/dev/null

pushd desktop/src-tauri >/dev/null
cargo generate-lockfile
popd >/dev/null

pushd desktop >/dev/null
npm ci
popd >/dev/null

mkdir -p desktop/dist
printf '<!doctype html><title>Silent Disco Block 16 validation</title>\n' > desktop/dist/index.html

pushd desktop >/dev/null
npm run bindings:check
npm run format:check
npm run lint
npm run typecheck
npm test -- --run
npm run build
popd >/dev/null

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all
python - <<'PY'
from pathlib import Path

path = Path("desktop/src-tauri/src/app_state.rs")
text = path.read_text()
replacements = [
    (
        """/// Returns a structured desktop error for invalid profile IDs, path or lock failures,
/// unavailable secure identity, storage or actor startup failure, bridge failure, or
/// lifecycle races. Partial startup is cleaned up before the error is returned.
""",
        """/// Returns a structured error for profile, path, lock, identity, storage, actor, bridge, or lifecycle failure; partial startup is cleaned up before returning.
""",
    ),
    (
        """/// The current authoritative snapshot is dispatched first. Replacing a subscription stops
/// and joins the old worker before the new subscription becomes active.
""",
        """/// The current snapshot is dispatched first; replacing a subscription stops and joins the old worker before the new subscription becomes active.
""",
    ),
    (
        """/// Returns a structured error when no profile is ready, the bridge has failed, the worker
/// cannot start, or the blocking attachment task fails.
""",
        """/// Returns a structured error when no profile is ready or bridge, worker start, or blocking attachment fails.
""",
    ),
    (
        """/// Returns a structured error when lifecycle state is unavailable, another open/close
/// is in progress, the close worker fails, or actor/database/profile-lock cleanup fails.
""",
        """/// Returns a structured error for lifecycle, concurrent open/close, worker, actor, database, or profile-lock cleanup failure.
""",
    ),
]
for old, new in replacements:
    old_count = text.count(old)
    new_count = text.count(new)
    if old_count == 1 and new_count == 0:
        text = text.replace(old, new, 1)
    elif old_count == 0 and new_count == 1:
        pass
    else:
        raise SystemExit(
            f"unexpected app-state documentation state: old={old_count}, new={new_count}"
        )
path.write_text(text)
PY
bash scripts/check-source-file-line-counts.sh

pushd desktop/src-tauri >/dev/null
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo check --all-features
popd >/dev/null

pushd desktop >/dev/null
npm run tauri build
popd >/dev/null

lock_status="$(git status --porcelain -- desktop/package-lock.json desktop/src-tauri/Cargo.lock)"
if [[ -n "${lock_status}" ]]; then
  printf '%s\n' "${lock_status}"
  echo "Desktop lockfiles must be committed exactly as generated." >&2
  exit 1
fi

git fetch origin agent/desktop-block16-closure
test "$(git rev-parse origin/agent/desktop-block16-closure)" = "${SOURCE_SHA}"

python - <<'PY'
from pathlib import Path
import os

todo = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
text = todo.read_text()
start = text.index("## Block 16 — Implement secure audio file selection")
end = text.index("## Block 17 — Implement atomic source staging", start)
block = text[start:end]
unchecked = block.count("- [ ]")
checked = block.count("- [x]")
evidence_marker = "**Completion evidence:** Secure single-file selection"
if unchecked == 19 and checked == 0:
    block = block.replace("- [ ]", "- [x]")
    evidence = (
        "**Completion evidence:** Secure single-file selection, bounded signature inspection, "
        "opaque backend registration, authoritative capability publication, profile-lifecycle "
        "cleanup, frontend integration, and all automated gates passed in GitHub Actions run "
        f"`{os.environ['GITHUB_RUN_ID']}` from source commit `{os.environ['SOURCE_SHA']}`. "
        "Physical interaction with a native desktop file dialog was not performed by this CI run.\n\n"
    )
    marker = "**Acceptance:** File selection grants only the access needed to stage one explicit source."
    if block.count(marker) != 1:
        raise SystemExit("Block 16 acceptance marker was not found exactly once")
    block = block.replace(marker, evidence + marker)
    todo.write_text(text[:start] + block + text[end:])
elif unchecked == 0 and checked == 19 and evidence_marker in block:
    pass
else:
    raise SystemExit(
        f"unexpected Block 16 state: unchecked={unchecked}, checked={checked}"
    )

memory = Path("memory.md")
heading = "## 2026-07-30 — Desktop Block 16 secure audio source selection complete"
if heading not in memory.read_text():
    entry = f'''

## 2026-07-30 — Desktop Block 16 secure audio source selection complete

- Source commit validated: `{os.environ['SOURCE_SHA']}`.
- Final implementation validation run: `{os.environ['GITHUB_RUN_ID']}`.
- Added backend-owned native file selection for one explicit WAV, FLAC, or MP3 source; cancellation remains distinct from failure and no unrestricted filesystem capability is exposed.
- Inspection verifies canonicalized regular files, an 8 GiB size bound, bounded/sanitized display names, fixed-size content signatures, explicit unsupported formats, and opaque source IDs. Native paths remain only in a single backend registry and are cleared fail-visibly when the profile closes.
- The authoritative actor receives only the redacted descriptor. Profile readiness waits for the acknowledged capability snapshot, and React waits for a newer Rust snapshot rather than mutating source state optimistically.
- Tests cover cancellation, dialog failure, missing files, directories, empty/oversized files, Unicode bounds, deceptive extensions, malformed MP3 headers, canonicalization and permission failures, deterministic identities, registry rollback/clear, capability publication, and frontend cancellation/error behavior.
- Automated validation passed source-size enforcement, generated-binding verification, frontend format/lint/typecheck/tests/build, Rust format/strict Clippy/tests/check, lockfile reproducibility, and Linux Tauri bundle creation. Native dialog interaction on a physical desktop session remains unclaimed.
'''
    memory.write_text(memory.read_text().rstrip() + entry)
PY

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add desktop/src-tauri/src/app_state.rs docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md memory.md
git rm --ignore-unmatch .github/workflows/finalize-desktop-block16-once.yml
git rm --ignore-unmatch .github/workflows/finalize-desktop-block16-pr-base.yml
git rm --ignore-unmatch .github/workflows/finalize-desktop-block16-pr.yml
git rm --ignore-unmatch .github/BLOCK16_FINALIZE_REQUEST
git rm --ignore-unmatch scripts/finalize-desktop-block16-pr.sh

if git diff --cached --quiet; then
  echo "Desktop Block 16 branch is already finalized."
else
  git commit -m "Complete Desktop Block 16 secure audio selection"
  git push origin HEAD:agent/desktop-block16-closure
fi
