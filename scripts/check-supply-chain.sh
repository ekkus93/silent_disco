#!/usr/bin/env bash
# Supply-chain gate (remediation TODO §7): dependency vulnerability audit for
# every locked dependency set, git-history secret scan, and CycloneDX SBOMs.
#
# Triage policy: any vulnerability, unsound, unmaintained or yanked finding
# fails, and is fixed by updating the dependency. Only a finding with no
# reachable fix may be excepted, with a reason and an expiry, in:
#   rust/deny.toml, desktop/src-tauri/deny.toml   (cargo-deny)
#   desktop/npm-audit-exceptions.json             (scripts/npm_audit_triage.py)
#   osv-scanner.toml                              (Android Gradle lockfiles)
# An expired exception fails again; an unused Rust or npm exception fails too.
# A missing or wrong-version tool, or an advisory database that cannot be
# fetched, fails the gate rather than skipping a check.
#
# Tools (exact versions, installed by .github/workflows/supply-chain.yml):
#   cargo-deny 0.20.2, cargo-cyclonedx 0.5.9, osv-scanner 2.7.0,
#   gitleaks 8.30.1, npm (with desktop/'s pinned Node).
#
# Usage: scripts/check-supply-chain.sh [output-dir]   (default build/supply-chain)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="${1:-$repo_root/build/supply-chain}"
mkdir -p "$out_dir/sbom"
out_dir="$(cd "$out_dir" && pwd)"
cd "$repo_root"

failures=()

require_version() {
  local name="$1" expected="$2"
  shift 2
  local actual
  if ! actual="$("$@" 2>&1)"; then
    echo "error: $name is not installed or failed to run ($*)" >&2
    exit 2
  fi
  if [[ "$actual" != *"$expected"* ]]; then
    echo "error: $name must be version $expected, found: $actual" >&2
    exit 2
  fi
}

require_version cargo-deny 0.20.2 cargo-deny --version
require_version cargo-cyclonedx 0.5.9 cargo-cyclonedx cyclonedx --version
require_version osv-scanner 2.7.0 osv-scanner --version
require_version gitleaks 8.30.1 gitleaks version

step() { echo; echo "== $*"; }

# 1. Rust advisories and sources, per workspace and its own deny.toml.
for workspace in rust desktop/src-tauri; do
  step "cargo-deny: $workspace"
  # cargo-deny reads the deny.toml next to the manifest.
  if ! cargo-deny --manifest-path "$workspace/Cargo.toml" --locked \
    check advisories sources; then
    failures+=("cargo-deny $workspace")
  fi
done

# 2. npm, every dependency including dev tooling.
step "npm audit: desktop"
# npm audit exits non-zero whenever it finds anything; the triage script
# decides, and also fails when the audit itself did not run.
npm_report="$out_dir/npm-audit.json"
(cd desktop && npm audit --json >"$npm_report") || true
if ! python3 -I scripts/npm_audit_triage.py desktop/npm-audit-exceptions.json <"$npm_report"; then
  failures+=("npm audit desktop")
fi

# 3. Android Gradle dependencies, from the Gradle lockfiles.
step "osv-scanner: Android Gradle lockfiles"
for lockfile in app/gradle.lockfile buildscript-gradle.lockfile; do
  if [[ ! -s "$lockfile" ]]; then
    echo "error: $lockfile is missing; regenerate it with --write-locks" >&2
    failures+=("missing $lockfile")
  fi
done
if ! osv-scanner scan source --config osv-scanner.toml --lockfile app/gradle.lockfile \
  --lockfile buildscript-gradle.lockfile; then
  failures+=("osv-scanner gradle")
fi

# 4. Secrets anywhere in git history (needs a full clone, not a shallow one).
step "gitleaks: git history"
if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
  echo "error: shallow clone; gitleaks needs the full history" >&2
  failures+=("gitleaks shallow clone")
elif ! gitleaks git --no-banner --redact --exit-code 1 .; then
  failures+=("gitleaks")
fi

# 5. SBOMs. Generation failures fail the gate; contents are not judged here.
step "SBOMs into $out_dir/sbom"
for workspace in rust desktop/src-tauri; do
  if ! cargo-cyclonedx cyclonedx --manifest-path "$workspace/Cargo.toml" \
    --all-features --format json --spec-version 1.5 --quiet; then
    failures+=("cargo-cyclonedx $workspace")
    continue
  fi
  while IFS= read -r -d '' bom; do
    mv "$bom" "$out_dir/sbom/$(basename "$bom")"
  done < <(find "$workspace" -path "$workspace/target" -prune -o \
    -name '*.cdx.json' -print0)
done
# From the lockfile, as CI has no node_modules installed.
if ! (cd desktop && npm sbom --sbom-format cyclonedx --package-lock-only >"$out_dir/sbom/desktop-npm.cdx.json"); then
  failures+=("npm sbom")
fi
if ! osv-scanner scan source --config osv-scanner.toml --lockfile app/gradle.lockfile \
  --lockfile buildscript-gradle.lockfile --all-packages \
  --format cyclonedx-1-5 --output "$out_dir/sbom/android-gradle.cdx.json" >/dev/null; then
  failures+=("android sbom")
fi
ls -l "$out_dir/sbom"

echo
if ((${#failures[@]} > 0)); then
  echo "supply-chain gate FAILED: ${failures[*]}" >&2
  exit 1
fi
echo "supply-chain gate passed"
