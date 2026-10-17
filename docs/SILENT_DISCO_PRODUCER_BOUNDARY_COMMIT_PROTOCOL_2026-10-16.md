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

## Serialized actor reservation primitive — 2026-10-17

The actor now has a dedicated synchronous reservation message and a bounded acknowledgment channel. Unlike the earlier asynchronous `submit_audio_event(TrackBoundaryScheduled)`, the new `CoreActorHandle::reserve_host_track_boundary` acknowledges only after the actor has verified the active stream, applied its queue-boundary lock, advanced/published its snapshot, and enqueued notifications. Queue mutations and reservations share the actor's serialized input order; cancellation is checked before acceptance and by the waiting producer. Unit regressions cover edit-first, reservation-first, and wrong-stream outcomes. **This only supplies the actor-side primitive.** The packetizer and production desktop/Android entry points do not yet call it; the queue-edit race remains OPEN until those integrations and executable validation are complete.

## Producer and desktop wiring checkpoint (2026-10-17)

Commits `613d9b7` and `07a4c70` add a synchronous actor reservation message, a cancellation-aware reply channel, active-stream identity enforcement, and actor-level edit/reservation regressions. A producer's success acknowledgment now means the serialized actor has applied and published the scheduled boundary, **not** merely admitted an asynchronous event to its queue. The actor rejects removed/reordered targets and boundaries from retired streams. The actor checks cancellation before processing a pending reservation.

Commits `e442763`, `696f7e4`, `a7f09c9`, and `5a07d59` add an optional Rust packetizer reservation callback, wire it to the desktop start path, and add gated-producer regressions. When this callback is present, the shared worker invokes it after choosing the next entry but **before putting the next boundary or audio in the bounded frame queue**. A rejected reservation fails closed and explicitly cancels/joins the already-open next decoder. The desktop host waits for its stream to enter Playing before asking the actor to reserve, so very short songs cannot be rejected merely because startup is still binding the stream.

**Still open:** Android's UniFFI streamer is constructed through the existing `FfiHostPlaylistStreamer.open` and does not yet possess a reference to `FfiCoreHandle` for authoritative reservations. Add an explicit, lifetime-safe actor authority to that streamer, regenerate the UniFFI Kotlin bindings using repository tooling (never hand-edit generated files), update the production Android `HostPlaylistStreamController` call, and exercise the producer/actor race on Android. Existing direct packetizer callers that use `spawn_playlist` remain ungated; only `spawn_playlist_reserved` ensures atomic actor commitment. The desktop integration also needs full executed Rust/Tauri regression and real-path qualification; no checkbox has been closed based on unexecuted tests.

At exact master `5a07d59b0b107de1e7e9a9d65d3b9aee2f2f2695`, GitHub Actions CI run [39017115444](https://github.com/ekkus93/silent_disco/actions/runs/39017115444), desktop CI [39017115342](https://github.com/ekkus93/silent_disco/actions/runs/39017115342), and source-file-limit [39017115540](https://github.com/ekkus93/silent_disco/actions/runs/39017115540) all failed before disclosing executable job steps; Ralph Bridge also reports job-log retrieval `JOB_NOT_FOUND`. No executable Rust compiler/binding generator is available in this session. Treat these as unqualified changes; verify compilation, strict Clippy/formatting, queue linearization, production host paths, and real audio continuity before marking §3 or §9 complete.

## Android production producer checkpoint (2026-10-17)

At `6cbb956626b6c26f992d360bcb664b41d34917c1`, Rust UniFFI adds an `open_authoritative` constructor that clones an active `FfiCoreHandle` actor authority and supplies `spawn_playlist_reserved` with its synchronous cancellation-aware reservation callback. The Android production `UniFfiHostCoreController.openHostPlaylist` passes the same host actor that admits queue edits, while the direct constructor remains for source validation and isolated fakes. The Android host frame consumer no longer resubmits an asynchronous `TrackBoundaryScheduled` for a producer that has already reserved it. The desktop startup callback now also stops waiting if playback has stopped or failed (`5d9afecb700e`). The larger UniFFI host playlist test module was moved to `host_playlist/tests.rs` to keep source files below the repository's 800-line gate (`b1a53fdc4812`). A native test covers rejection when the FFI authority has already shut down.

**Not yet qualified:** No Rust compiler or UniFFI Kotlin generator was available in this execution environment, and the Ralph Bridge `rust` local-validation profile was rejected. Automatic GitHub Actions checks for the changes failed with no actionable job-step logs. Regenerate Kotlin bindings through `scripts/generate-uniffi-kotlin.sh`, run `RUST_TEST_THREADS=1 bash scripts/check-rust.sh`, `./gradlew testDebugUnitTest lintDebug assembleDebug assembleRelease connectedDebugAndroidTest --stacktrace --console=plain`, the desktop checks, and deterministic actor/Android host interleavings at an exact head. Confirm no stale frame can leak after queue edits, including Stop/replay; retain §3 and §9 as open until executable and physical evidence is recorded.
