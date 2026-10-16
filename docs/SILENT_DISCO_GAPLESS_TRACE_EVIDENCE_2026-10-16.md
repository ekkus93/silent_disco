# Gapless playback evidence preflight

The authoritative acceptance matrix remains in `SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md` §9. This optional, standard-library-only helper validates *captured trace consistency* before engineering review. It **does not certify physical or acoustic gaplessness**, authenticate the device that captured the data, or replace hosted CI.

Run `python3 scripts/check-gapless-evidence.py /path/to/trace.json` for basic consistency checking. For collected live-queue-edit playback evidence, run `python3 scripts/check-gapless-evidence.py --require-golden-boundaries --require-sink-identity /path/to/trace.json`. Neither invocation independently establishes the physical truth of the submitted measurements. Run its unit tests with `python3 -m unittest discover -s scripts/tests -p 'test_check_gapless_evidence.py' -v`.

## Canonical capture JSON (schema version 1)

Capture one continuous-stream playback interval beginning before the first transition and ending after the last transition. Normalize raw logs to the schema below; do not invent counters or infer absent packets. The host and listener packet windows must match exactly (including sequence numbers), so late-join tests must use an interval **after** the listener has joined and synchronized. Validate initial late-join synchronization separately.

```json
{
  "schema_version": 1,
  "test_case": "desktop host to Android listener, WAV to FLAC",
  "stream_id": "stream-42",
  "sample_rate_hz": 48000,
  "expected_boundaries": [{"entry_id": "second-song", "first_sample_index": 10601}],
  "forbidden_entry_ids": ["removed-song"],
  "host": {
    "packets": [
      {"sequence": 10, "first_sample_index": 9600, "frame_count": 960},
      {"sequence": 11, "first_sample_index": 10560, "frame_count": 960}
    ],
    "boundaries": [{"entry_id": "second-song", "first_sample_index": 10601}],
    "boundary_scheduled": 1,
    "boundary_applied": 1,
    "boundary_discontinuities": 0
  },
  "listeners": [{
    "id": "android-listener-1",
    "packets": [
      {"sequence": 10, "first_sample_index": 9600, "frame_count": 960},
      {"sequence": 11, "first_sample_index": 10560, "frame_count": 960}
    ],
    "boundaries": [{"entry_id": "second-song", "first_sample_index": 10601}],
    "boundary_scheduled": 1,
    "boundary_applied": 1,
    "output_device_id_before": "speaker-A",
    "output_device_id_after": "speaker-A",
    "output_runtime_id_before": "oboe-stream-01",
    "output_runtime_id_after": "oboe-stream-01",
    "stream_id_before": "stream-42",
    "stream_id_after": "stream-42",
    "output_opens_before": 1,
    "output_opens_after": 1,
    "output_closes_before": 0,
    "output_closes_after": 0,
    "concealed_frames_before": 0,
    "concealed_frames_after": 0,
    "missing_boundary_frames": 0,
    "duplicate_boundary_frames": 0
  }]
}
```

Each `frame_count` is decoded PCM **sample frames**, not bytes or channel-interleaved scalar samples. Boundary indices need not align with audio datagram boundaries: the example's second track starts at sample 10601, inside the second datagram. The verifier checks contiguous packet sequence/sample indices, strictly increasing boundaries inside the captured window, listener identity/index agreement, recorded scheduling/application counts, no stream reopen/close, no growth in concealment counters, and zero reported missing/duplicate boundary frames. Counter values should be deltas scoped to the capture, except explicitly named before/after values. On reconnect, scheduling counters may exceed captured unique boundary counts; the verifier deliberately uses a minimum rather than false equality.

A passing result only means the submitted normalized JSON satisfies those structural invariants. Preserve raw host and listener diagnostics, build SHAs, device model, OS versions, timestamp/clock data, transport logs, actual output/loopback measurements or recordings, and any user-observable glitches alongside this trace. Verify those independently against the specification's device matrix. Never check off §9 on this script's result alone. Failed capture/telemetry acquisition is **inconclusive**, not a pass.

## Independent queue-edit and sink-identity assertions

For the reopened §3 boundary-commit race, an uninterrupted PCM sample timeline is **not enough**: the sender may play a removed/obsolete next track with perfectly contiguous samples. Record the actor-approved result of each edit and derive `expected_boundaries` independently from the accepted final queue order and decoded per-source frame counts. The list contains the full expected `(entry_id, first_sample_index)` sequence within the capture window; do **not** copy it from observed `TrackBoundary` frames. A stale/removed track must also be listed in `forbidden_entry_ids`, when appropriate. The verifier rejects a host and listener that agree on the **wrong** song, or on a one-frame-shifted boundary. Passing requires both independently expected and observed lists to agree.

For physical-output continuity, capture the listener's active `output_device_id`, native `output_runtime_id`, and protocol `stream_id` both before and after the transition. The strict option requires all six nonempty fields and rejects a device/runtime restart, even if the open/close counters were reset or incorrectly scoped. These identities must come from the actual playback path; do not fabricate them when native telemetry is unavailable. The optional provenance fields are useful in non-strict preflight, but acceptance-grade evidence should include them. This complements rather than replaces the output open/close counters and actual recorded output measurements.

This verifier intentionally checks **one uninterrupted observation window**. It is not suitable for a session spanning an intentional Stop/replay, a stream change, or a late join whose earlier packets were never observed. Split such a test into uninterrupted windows and validate the separate lifecycle/event transitions independently. It does not currently cross-validate monotonic-clock deadlines, speaker acoustics, or independent packet-loss telemetry. `PASS` remains a structural trace result, **not physical gapless acceptance**.
