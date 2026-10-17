# Producer-side boundary commit protocol (2026-10-16)

Status: design handoff; **not implemented or qualified**. Target: section 3 of the gapless queue TODO.

## Verified race

The shared playlist worker queues a TrackBoundary and successor PCM into a bounded 32-frame channel. Desktop and Android announce TrackBoundaryScheduled to the authoritative actor only after consuming that boundary from the channel. An edit accepted between enqueue and consume may remove the already-committed next track; the worker has no opportunity to retarget that buffered successor, and the actor rejects the later announcement.

## Required linearization point

Before emitting a successor boundary or successor audio, the Rust producer must synchronously reserve the exact (stream identity, entry ID, first sample index) against the same serialized actor that handles queue mutations. If reservation wins, the actor must reject edits to the committed next entry while permitting safe edits behind it. If an edit wins, the producer must refresh its next-track decoder from the authoritative revision and retry the reservation, or fail closed without emitting stale successor audio. No Android URI or desktop path may enter actor state.

The producer must not wait for the consumer of its own bounded output channel while making the reservation; that would deadlock under backpressure. It must not hold a decoder queue lock across actor callbacks. Cancellation, Stop, and session teardown must abort a pending reservation wait. Pause may preserve a reservation, but a fresh Play must invalidate the prior stream's reservation. The boundary event should include the stream ID so delayed events from a stopped stream cannot reserve a boundary in a new stream with the same entry IDs.

## Required deterministic tests

1. Pause the consumer while the worker has filled its bounded queue. Remove the prefetched next entry through the actor and then release the consumer. Verify that no boundary or PCM from the removed entry reaches the listener.
2. Reverse the ordering: reserve the boundary first, then try to remove/reorder that entry. Verify the actor rejects the mutation and the emitted boundary still matches the queue.
3. Apply a queued edit before reservation and verify the producer retargets and emits only the replacement track at the exact sample index, with contiguous PCM and sequence numbers.
4. Repeat under Stop/replay, pause/resume, natural queue end, decoder failure, and a disconnected consumer; ensure no hangs or stale boundary application.
5. Exercise the production Android FFI and desktop host paths, not just a standalone packetizer test. Verify same-stream identity and exact-head Rust/Android/desktop gates before checking this TODO item.

This note does not replace device-level output continuity evidence or restore unavailable GitHub Actions execution.

## EOF retarget progress, 2026-10-16

Commits `b10491a` and `013b2e1` recheck the pending upcoming-list update after the outgoing decoder completes and before promoting its prebuffered successor. If that update removes the last successor, the worker now finalizes the existing packetizer with an **empty final PCM chunk**, flushing any carried partial packet at the genuine queue end. A regression checks its exact sample/sequence position. This supersedes the earlier implementation-note statement that this specific late-removal case must stop with a typed packetizer failure.

The producer-to-consumer enqueue race remains unresolved. No reservation callback is connected to the actor, so section 3 of the TODO stays unchecked. Do not confuse the EOF retarget fix with full boundary-commit linearization.

## Exact-head qualification observation (2026-10-17)

At master `0bc29d14d998976ef8abbb2022a10ca8649a1072`, Rust/Android CI run 39010477546 attempt 2 failed with zero executed job steps, and the bridge could not retrieve either failing job log (`JOB_NOT_FOUND`). The `rust` local-validation profile is not authorized/available. This is an infrastructure qualification blocker, not evidence that the new Rust tests passed or failed. The producer/actor boundary reservation remains unimplemented and must be wired to both production hosts before the queue-retargeting checkbox can close.
