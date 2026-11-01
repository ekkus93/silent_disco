# Silent Disco — Current Acceptance Index

**Snapshot:** 2026-11-01 UTC; source baseline `29f58bc15bdec96afa255990fa459b38b44f4489` (not a release signoff).
**Scope:** [original gapless checklist](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md), [active security/correctness remediation checklist](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md), and [append-only evidence ledger](SILENT_DISCO_ACCEPTANCE_EVIDENCE_LEDGER.md).

This index is a navigation and status snapshot, **not** a substitute for the two canonical checklists. The original 2026-10-15 TODO, including its historical test notes and checkboxes, is intentionally unchanged. Counts below are checklist-item counts, not engineering-completion percentages. A green build does not establish encrypted transport, measured physical gapless audio, pixel-golden approval, or hands-on accessibility.

## Original gapless checklist — 58/72 accepted, 14 open

| Canonical section | Checked | Open | Open acceptance scope |
| --- | ---: | ---: | --- |
| §0 Handoff/baseline | 4 | 0 | — |
| §1 Feasibility/protocol | 6 | 0 | — |
| §2 Rust playlist | 8 | 0 | — |
| §3 Source staging/prebuffer | 9 | 0 | — |
| §4 [Continuous playback](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md#4-continuous-gapless-hostlistener-playback) | 7 | 3 | Late join/reconnect current-track and upcoming-boundary parity; Pause/Resume/Stop/replay across queued tracks; comprehensive virtual producer/edit/failure continuity |
| §5 [Android host](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md#5-android-host-experience) | 5 | 1 | Complete Compose state/navigation/compact/large-text/process-restore test matrix |
| §6 Desktop host | 6 | 0 | — |
| §7 [Client join/listening](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md#7-client-joinlistening-experiences) | 5 | 1 | Android/React approval, timeout, reconnect, late-join, viewport and accessibility matrix |
| §8 [Design/accessibility](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md#8-shared-design-system-and-accessibility) | 2 | 3 | Material 3 Expressive parity; focus/semantics/contrast/insets/reduced motion; reviewed Android and desktop pixel goldens |
| §9 [Qualification/closeout](SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md#9-qualification-and-closeout) | 6 | 5 | Four explicitly **deferred physical/device** gates; final exact-head evidence reconciliation |
| **Total** | **58** | **14** | **10 non-deferred open + 4 deferred device items** |

## Active code-review remediation — 4/60 accepted, 56 open

The [2026-10-27 remediation TODO](SILENT_DISCO_CODE_REVIEW_REMEDIATION_TODO_2026-10-27.md) supersedes earlier security assumptions, not the original playback acceptance record. The four accepted items are the LAN threat model/ADR, verified-enrollment design, Android backup exclusion policy, and the linked concise acceptance index/append-only ledger. **53 non-deferred remediation items remain open**, plus three explicitly deferred §10 hardware items. Do not sum the two TODOs: their requirements overlap.

| Priority | Finding / acceptance dependency | Current evidence and remaining blocker |
| --- | --- | --- |
| P0 | F2 secure v4 transport | [QUIC/TLS 1.3 ADR](SILENT_DISCO_SECURE_TRANSPORT_ADR_2026-10-29.md), P-256 proof/trust and replay scaffolding; **no production authenticated QUIC/TLS control or encrypted PCM**, no host pinning/OS-key integration |
| P0 | F1 approved-only admission | Rust verified-key policy scaffold; v3 deliberately rejects durable trust. Android/desktop cannot auto-trust claimed IDs; secure host/listener handshake and persisted verified records remain |
| P1 | F3 authoritative events | Bounded Rust socket delivery, explicit busy refusal, real-socket pressure/reconnect tests; complete Rust actor → Android bridge → UI timeout/spinner matrix remains |
| P1 | F4 Android native lifecycle | Generation-gated handle ownership and blocked-send tests; full native-loopback multi-generation host/listener stress remains |
| P1 | F5 Android bridge bounds | Bounded priority queues and diagnostic counters; cross-layer pressure, process restoration and UI final-state acceptance remain |
| P2 | F6 restore | Backup disabled and installed-manifest tested; key-loss/corrupt-record/revocation/restore UX and simulations remain |
| P2 | Product/audit | Android pixel goldens, desktop webview pixels, Compose/React accessibility, fuzzing, SBOM and final evidence matrix remain |

## Evidence and closeout rules

1. Use the [append-only ledger](SILENT_DISCO_ACCEPTANCE_EVIDENCE_LEDGER.md) for exact source SHA, hosted workflow/run, terminal status, test scope and explicit exclusions. A passing CI result applies **only** to its own SHA.
2. Treat focused Rust/FFI socket→scheduler→native callback PCM assertions as **software** proof; neither emulator nor Lab output establishes physical DAC/Bluetooth latency or no-reopen measurements.
3. Keep the original §9 four physical/device items and remediation §10 three owner-deferred items unchecked. Do not infer them from simulations.
4. Update this snapshot when checkboxes or their acceptance evidence change; append a new dated ledger entry rather than rewriting historical observations.
