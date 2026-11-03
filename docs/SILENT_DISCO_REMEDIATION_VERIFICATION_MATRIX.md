# Silent Disco — Remediation Verification Matrix

**Established:** 2026-11-03. **Observed exact master baseline:** `dbbfa34f5461e6d17665f03088f510cdaa2172fb`.  
**Scope:** The planning/test-matrix deliverable in remediation TODO §0. This is a coverage map, **not** a declaration that F1–F6 have passed.

## Evidence interpretation

A successful workflow run proves only that its configured jobs completed; it does **not** prove every scenario in the table was executed. Scenario-level acceptance requires a named test or artifact, positive and adversarial assertions, and the same exact head SHA. The baseline below is an observed hosted CI result, not a substitute for test-case mapping. Never infer physical-device results from emulators or loopback.

| Verification layer | Relevant entry points / suite | Required scenarios and assertions | Gate / status at baseline |
| --- | --- | --- | --- |
| Rust core and protocol | `bash scripts/check-rust.sh`; `rust/silent-disco-core/` | Admission policy Manual/Invite/Approved-only; authorized lifecycle ordering; generation fencing; malformed frames; replay, identity and nonce negatives once secure protocol exists | F1–F3; **not accepted** |
| UniFFI/native boundary | `rust/silent-disco-ffi/`; Android host/listener native controllers | FFI generation ownership; stale authorize/send/poll; close while in-flight; key-bound verified identity crossing boundary | F1/F4; **not accepted** |
| Android JVM | `app/src/test/` | Queue saturation with stalled collectors; distinct approvals; terminal event retention; profile switches; restore with missing/corrupt identity keys; navigation/config state | F4–F6; **not accepted** |
| Android API29 emulator | `app/src/androidTest/`; configured hosted CI | Native loopback join/refusal, close/rebind stress, restored identity UX, host/listener reconnect, process/config lifecycle; collect test names and emulator evidence | F1/F4–F6; **not accepted** |
| Desktop React / Tauri | `desktop/src/`, `desktop/src-tauri/`; `npm run check` and Rust checks in AGENTS.md | Verified trust projection, explicit join errors, desktop keyboard/focus and lifecycle, pixel screenshot vs DOM-only evidence | F1/F2/§8; **not accepted** |
| Real loopback TCP/UDP | `rust/silent-disco-core/src/transport/socket/`; Desktop Lab live transport workflow | Multi-peer join/deny/reconnect, Stop/replay, exact PCM/sample indices, track boundaries, no native output reopen, high telemetry pressure and bounded busy refusals | F2/F3/§9; **partial v3 transport evidence only** |
| Hostile peer / security | Secure-transport ADR plus new negative fixtures and authenticated real-socket integration | Forged host/listener ID, invalid key/cert/tag, plaintext downgrade, duplicate/old packet, nonce wrap, retired generation, tampered control, QR mismatch, unauthorized audio | F1/F2; **missing authenticated-v4 acceptance** |
| Resource pressure | Rust bounded event queue, Android bounded bridges; multi-peer loopback and slow collectors | Saturate authoritative events; explicit refusal or delivery; no ghost approval/Stop loss; bounded bytes, counters, finite retry, shutdown under pressure | F3/F5; **not accepted end-to-end** |
| Android visual and accessibility | Emulator screenshot/golden and Compose tests | Theme/font scale/orientation/density, approval/reconnect/error, insets, reduced motion, keyboard and focus; publish diff artifacts | §8; **not accepted** |
| Desktop pixel visual | GitHub-hosted desktop webview capture | Real pixels and reviewed baseline update policy; separate from DOM snapshots | §8; **not accepted** |
| Supply chain | Pinned audit/SBOM and CI scanners | Reproducible dependency manifest, severity policy, failure visibility, no secret leakage | §7; **not accepted** |
| Physical hardware (owner deferred) | Physical Android and desktop devices; objective audio measurement | DAC/Bluetooth/headphone output, real-device end-to-end continuity and hands-on accessibility | §10; **DEFERRED, never infer from software CI** |

## Observed hosted CI at baseline

All five selected workflows were **completed / success** for exact `dbbfa34f5461e6d17665f03088f510cdaa2172fb`:

| Workflow | Run | What can be claimed |
| --- | --- | --- |
| Rust/Android CI | [41612379225](https://github.com/ekkus93/silent_disco/actions/runs/41612379225) | Configured CI workflow passed; inspect jobs/artifacts before naming individual tests |
| Desktop CI | [41612379167](https://github.com/ekkus93/silent_disco/actions/runs/41612379167) | Configured desktop workflow passed |
| Desktop Lab live transport | [41612379095](https://github.com/ekkus93/silent_disco/actions/runs/41612379095) | Configured lab workflow passed; does not establish encrypted v4 |
| Source file line limit | [41612379102](https://github.com/ekkus93/silent_disco/actions/runs/41612379102) | Source line policy passed |
| Publish CI Status | [41616262389](https://github.com/ekkus93/silent_disco/actions/runs/41616262389) | Status publication succeeded |

## Acceptance protocol for each future completed item

Record the implementation path, named positive and negative tests, exact SHA, workflow/run ID, and actual assertion results. Mark a requirement complete only when **all** of its acceptance clauses are proven. Refresh the SHA before any CAS commit, and re-run the applicable workflows on the new head. Preserve the original TODO and the append-only evidence ledger. No checked item here implies production secure transport, authenticated trust, or physical playback qualification.
