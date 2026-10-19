# Silent Disco CI runner and SSD-write policy

## Incident and cause

On 2026-10-18 the owner reported about 100 GB written by the persistent `silent-disco` runner since boot. The 20:58–21:14 PDT write spike coincides with Actions run 39197161575. Its job logs show `actions/checkout@v5` executing `git clean -ffdx` between Android, Rust, and emulator jobs; build products such as `app/build/`, `app/.cxx/`, `.gradle/`, and `rust/target/` were deleted and recompiled. This identifies avoidable build amplification, not an exact per-job byte accounting.

## Policy

- Default Rust, Android, desktop, Lab, and emulator jobs run on **GitHub-hosted Ubuntu**, for trusted `master` pushes, manual dispatches, and untrusted PRs. No automatic self-hosted fallback if GitHub-hosted capacity is unavailable.
- Documentation-only changes skip the full Rust/Android CI workflow; desktop/Lab already filter changes. Hosted jobs have isolated workspaces, so checkout cleanup cannot destroy the owner's SSD-backed compilation artifacts.
- Routine source-line validation also runs on `ubuntu-24.04`, including `master` pushes and manual dispatch. All routine qualification is GitHub-hosted; there is no self-hosted fallback.
- The physical Android device-probe workflow is also manual-only and **disabled at the job level**; its former push trigger and runnable self-hosted job were removed from the policy surface on 2026-10-19. Neither hardware workflow may execute without fresh owner authorization. The source-line workflow now runs a Python guard with six regression tests that reject non-hosted routine runners and require both hardware exceptions to remain manual-only and job-disabled.
- Block 46.3 physical-device staging is **disabled even for manual dispatch**, because its device acceptance workflow specifically requires a locally attached Android device and the owner does not authorize using the persistent runner. Retain the workflow for future explicit authorization, but its job-level `if: ${{ false }}` prevents scheduling onto `[self-hosted, Linux, X64, silent-disco]`. Do not claim hardware staging complete.
- At `0f08cfb8`, hosted Rust/Android CI, desktop CI, and deterministic Lab passed; Android release-candidate and desktop bundle artifacts were uploaded successfully. The old artifact-quota blocker is no longer observed for these exact-head runs.
- The CI-status publisher reacts to **completed** runs only, reducing duplicate workflow launches.
- Artifact upload failure must not be mistaken for code/build failure or passed publication. The Android candidate publication gate must fail if the candidate cannot be uploaded, while instrumentation remains separately runnable.
- Keep physical gapless audio, Android device/audio and privileged package lifecycle acceptance unchecked absent actual evidence. Hosted emulator tests are not physical acceptance.

## Qualification

Confirm exact-head source-line, Rust formatting/Clippy/tests, Android unit/lint/build/emulator, desktop frontend/Rust/bundles, and deterministic Lab results. For each routine CI job, verify a GitHub-hosted runner from the job log or metadata. Do not dispatch the hardware staging workflow merely to demonstrate the routing change. Remaining writes by other CI repositories are outside this repo's configuration.
