# Silent Disco Ralph Bridge source-write incident — 2026-10-21

Status: **Open — platform-level source-write denial; no source change accepted.**

- UTC observed: 2026-10-21, approximately 13:35 UTC.
- Repository: `ekkus93/silent_disco`; target: existing `master` only.
- Expected master SHA: `e64691a19d96388873b3c3a50f5af29560b58db9`.
- Observed master SHA after denial: `e64691a19d96388873b3c3a50f5af29560b58db9` (unchanged).
- Ralph Bridge runtime: 0.6.2, build `0acea4b23362e2ccecab8f6c9db4418b162426cf`, instance `c75f6fc5b7794249adb6bf78a7d3605b`, GitHub App authentication.
- Method: `commit_files`, sanitized arguments: repo, branch, exact expected head, two UTF-8 Rust upserts, Conventional Commit message.
- Target paths: `rust/silent-disco-core/src/transport/socket/host/node.rs` and `rust/silent-disco-core/src/transport/socket/host/authorization_order_tests.rs`.
- Proposed behavior: serialize explicit authorized-peer disconnect with the control delivery/replay mutex, with a deterministic concurrent-disconnect regression. This would prevent explicit disconnect from retiring a route during an already-accepted targeted control send. This is a separate proposed race fix from the previously qualified post-gate route lookup correction.
- Exact error: `This tool call was blocked by OpenAI's safety checks. Please double check what you are sending.`
- Operation ID: none returned; accepted write: none.
- Retry: **no retry of the blocked payload**; post-error exact-head read confirms no mutation. No branch, PR, workflow dispatch, or self-hosted job created.
- Qualification: not executed; do not mark any TODO checkbox complete based on this proposal. Existing latest code head `5eb35bd0` passed hosted Rust/Android, desktop, and source-line CI; documentation head `e64691a19d96388873b3c3a50f5af29560b58db9` passed source-line CI only.
- Classification: platform safety denial, **not** a Ralph Bridge/GitHub authentication, permission, rate-limit, or CI failure. Do not evade the denial with a modified version of the same blocked source write.

Unresolved: complete real-device gapless/reconnect/output continuity, accessibility/goldens, the separately recorded 44.1 kHz presentation-timestamp drift, and remaining race permutations under the authoritative TODO.
