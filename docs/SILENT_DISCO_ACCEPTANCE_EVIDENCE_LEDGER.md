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
