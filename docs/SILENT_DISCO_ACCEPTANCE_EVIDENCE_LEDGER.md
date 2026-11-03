# Silent Disco — Append-only Acceptance Evidence Ledger

**Purpose:** Chronological, immutable-in-place evidence pointers for the [current acceptance index](SILENT_DISCO_ACCEPTANCE_INDEX.md), [original gapless TODO](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md), and [active remediation TODO](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md).

**Policy:** Append new dated entries. Do not rewrite or delete older observations when later CI supersedes them. Each entry must distinguish exact source SHA, real hosted run status, tested behavior and unproven claims. A rerun belongs to its original SHA and attempt. The original 167 KB TODO retains all historical notes and checkboxes.

## 2026-11-01 — Reconciled last passing remediation baseline

- **Source:** [`1ab076eb36347225d076b507ea0c3ddbfb440373`](https://github.com/ekkus93/silent_disco/commit/1ab076eb36347225d076b507ea0c3ddbfb440373).
- **Terminal hosted CI:** [Rust/Android CI 41290019520](https://github.com/ekkus93/silent_disco/actions/runs/41290019520) **success**; [source-file policy 41290019534](https://github.com/ekkus93/silent_disco/actions/runs/41290019534) **success**; [status publication 41290333128](https://github.com/ekkus93/silent_disco/actions/runs/41290333128) **success**. These were inspected at the exact SHA.
- **Software scope:** Rust format/strict Clippy/workspace, Android build/JVM/lint/API29 instrumentation and candidate gate as configured in the CI workflow. The previous commits add bounded, priority-aware Android native transport and Rust-core notification delivery under stalled consumers.
- **Not proven:** Desktop CI on this exact SHA, production QUIC/TLS/AEAD, verified approved-only enrollment, all end-to-end Android UI/pressure outcomes, or physical playback/accessibility.

## 2026-11-01 — Replay generation PCM anchor/clock guard

- **Code source:** [`f8c4ebd3f5f1f4daa1047e6749bc2c8fb7f94586`](https://github.com/ekkus93/silent_disco/commit/f8c4ebd3f5f1f4daa1047e6749bc2c8fb7f94586); pinned rustfmt correction [`1be0318e8878c3a204aef822db298ad1537ce8d0`](https://github.com/ekkus93/silent_disco/commit/1be0318e8878c3a204aef822db298ad1537ce8d0).
- **Production change:** After Stop, a reused stream ID's first PCM packet must match the new StreamStart presentation anchor. Later replay packets cannot regress presentation time even with the expected sequence/sample indices. Rejected frames do not advance the replay watermark.
- **Test scope:** `rust/silent-disco-core/src/transport/socket/host/socket_multitrack_audio_reconnect_tests.rs` adds stale packet-zero anchors and regressing successor-clock attempts across four real loopback UDP listener routes, followed by valid contiguous PCM.
- **Observed failure:** [Desktop Lab 41298318512](https://github.com/ekkus93/silent_disco/actions/runs/41298318512) on `f8c4ebd3` failed **pinned rustfmt**; the actual log showed two wrapping differences in `host/mod.rs`. The formatting-only correction was committed at `1be0318e`; no pass is inferred from the failed run.
- **Qualification:** Exact-head hosted CI on `1be0318e` was **pending** when this entry was recorded. Do not promote this entry to a passed claim without inspecting the terminal runs. The change is a v3 replay correctness fence, **not cryptographic authentication**; a malicious peer can still forge plausible timestamps on plaintext v3.

## 2026-11-01 — Full hosted qualification of the five-peer replay baseline

- **Exact source:** [`693cd90c0a571fd63ae8b2a04267b4d88ad05459`](https://github.com/ekkus93/silent_disco/commit/693cd90c0a571fd63ae8b2a04267b4d88ad05459).
- **Terminal GitHub-hosted checks:** [Rust/Android CI 41298699164](https://github.com/ekkus93/silent_disco/actions/runs/41298699164) **success**; [Desktop CI 41298699199](https://github.com/ekkus93/silent_disco/actions/runs/41298699199) **success**; [Desktop Lab live transport 41298699161](https://github.com/ekkus93/silent_disco/actions/runs/41298699161) **success**; [source-file policy 41298699304](https://github.com/ekkus93/silent_disco/actions/runs/41298699304) **success**; [status publication 41299836900](https://github.com/ekkus93/silent_disco/actions/runs/41299836900) **success**. These five workflows were observed terminal on the same SHA.
- **Software behavior:** Host loopback TCP/UDP Stop/replay generation fencing, four-to-five listener continuity, negative stale/duplicate/malformed PCM attempts and native transport test suite. This is not encrypted v4 or physical output-reopen proof.

## 2026-11-01 — Fail-closed playback control transport on writer errors

- **Production source:** [`b53b16d6`](https://github.com/ekkus93/silent_disco/commit/b53b16d6eb1f20a8a6868637aa90786608a036f8), with pinned rustfmt and strict-Clippy follow-up at `1d43d7ff`, `047e145f`, and `29f58bc1`.
- **Behavior:** An authorized TCP listener whose authoritative StreamStart, TrackBoundary, Pause or Stop delivery fails is retired immediately instead of remaining eligible for later UDP PCM while its control timeline is stale. A healthy second listener continues receiving Stop. A real-loopback test exercises route retirement, unaffected peer delivery, stopped audio suppression and same-claimed-ID replacement; its failure injection invokes the shared retirement boundary rather than forcing a kernel TCP error.
- **Observed initial CI failures:** [Desktop Lab 41302654527](https://github.com/ekkus93/silent_disco/actions/runs/41302654527) reported pinned rustfmt; [Rust CI 41302654521](https://github.com/ekkus93/silent_disco/actions/runs/41302654521) reported strict Clippy `unused_self`. Both defects were corrected in follow-up commits, not ignored.
- **Latest validation at record time:** [Desktop Lab 41303135263](https://github.com/ekkus93/silent_disco/actions/runs/41303135263) and [source-file policy 41303135287](https://github.com/ekkus93/silent_disco/actions/runs/41303135287) **passed** at `29f58bc1`. [Rust/Android CI 41303135281](https://github.com/ekkus93/silent_disco/actions/runs/41303135281) and [Desktop CI 41303135198](https://github.com/ekkus93/silent_disco/actions/runs/41303135198) were **not terminal** when recorded. Do not infer exact-head full qualification until those runs complete. No physical-device acceptance is claimed.

## 2026-11-03 — Remediation test/CI matrix deliverable

- **Observed baseline:** `dbbfa34f5461e6d17665f03088f510cdaa2172fb` with terminal hosted success: [Rust/Android CI 41612379225](https://github.com/ekkus93/silent_disco/actions/runs/41612379225), [Desktop CI 41612379167](https://github.com/ekkus93/silent_disco/actions/runs/41612379167), [Desktop Lab 41612379095](https://github.com/ekkus93/silent_disco/actions/runs/41612379095), [source policy 41612379102](https://github.com/ekkus93/silent_disco/actions/runs/41612379102), [status 41616262389](https://github.com/ekkus93/silent_disco/actions/runs/41616262389).
- **Deliverable:** [remediation verification matrix](SILENT_DISCO_REMEDIATION_VERIFICATION_MATRIX.md) explicitly maps Rust, UniFFI, Android JVM/API29, desktop/Tauri, loopback sockets, hostile peers, pressure, visual, supply chain and deferred physical evidence to F1–F6 and closeout.
- **Acceptance boundary:** §0 matrix-documentation subtask is complete; the matrix does not assert security fixes, individual scenario-level execution or device acceptance. The documentation commit requires its own exact-head CI; the cited successful workflows are from the immediately preceding baseline.
