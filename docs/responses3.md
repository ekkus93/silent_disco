# FIX5 Clarifications — responses3.md

Generated 2026-06-28 from review of `SILENT_DISCO_FIX5_SPEC.md` and `SILENT_DISCO_FIX5_TODO.md`.

Claude Code has reviewed the spec and TODO list and identified 2 points needing clarification before implementation begins.

---

## 1. Spec describes a correctness pass, not a styling pass

**Observation**: The spec explicitly states *"This pass is not a styling pass and not a general refactor. It is a correctness/hardening pass."* All 8 issues in the spec are behavioral: false success states, listener stuck in CONNECTING, delivery-before-send ordering, stale progress flags, and tautological tests.

**Question**: Is the scope of FIX5 intentionally correctness-only as the spec describes, or is there a separate styling/visual polish component that should be included?

---

## 2. Two open decisions left unresolved in the TODO

### 2a. P1.4 — `zeroPeerBroadcastCount`: remove or expose in diagnostics?

The TODO lists two valid options:

- **Option A**: Remove the counter entirely (`var zeroPeerBroadcastCount` and all `+= 1` sites deleted).
- **Option B**: Wire it into `summarizeMetrics()` and host diagnostics so it is visible.

Which is preferred?

### 2b. P2.3 — BLE async failure test: internal callbacks or fake service interface?

The TODO lists two approaches to replace the current tautological `MutableSharedFlow` test:

- **Option A**: Add `internal fun emitAdvertiseFailureForTest(errorCode: Int)` and `emitScanFailureForTest(errorCode: Int)` to `BleDiscoveryService` and test the real service flow through those hooks.
- **Option B**: Use a fake `BleDiscoveryService` interface (if one exists or can be extracted) to test ViewModel mapping — scan failure → `listenerState = ERROR`, advertise failure → `hostState = ERROR`.

The spec notes Option B is preferred *if a fake service interface exists*. There is currently no `BleDiscoveryService` interface; the class is concrete. Should one be extracted for this pass, or should Option A (internal test hooks on the concrete class) be used instead?

---

## Ready for implementation once clarified

Once these two points are settled, FIX5 can proceed in Ralph Loop style with the prescriptive code shapes provided in the TODO.
