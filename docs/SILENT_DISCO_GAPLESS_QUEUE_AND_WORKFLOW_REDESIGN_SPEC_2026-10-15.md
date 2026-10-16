# Silent Disco Gapless Queue and Workflow Redesign

**Status:** Draft for product review  
**Date:** 2026-10-15  
**Platforms:** Android and desktop  
**Mockups:** `docs/mockups/SILENT_DISCO_WORKFLOW_MOCKUPS_2026-10-15.html`  
**Companion TODO:** `docs/SILENT_DISCO_GAPLESS_QUEUE_AND_WORKFLOW_REDESIGN_TODO_2026-10-15.md`

## 1. Purpose

Make hosting and joining a silent disco easy to understand without instructional paragraphs, and make a host session support a managed playlist that transitions between songs without audible gaps for listeners.

This specification combines the host/client workflow redesign with the playlist and playback decisions confirmed in product discussion. It supersedes no existing architecture or acceptance record by itself. Implementation must preserve the Rust-authoritative domain/protocol design and the platform adapters described in `AGENTS.md` and the Rust core architecture specification.

## 2. Product decisions

The following decisions are confirmed:

- The host can choose multiple songs before creating a session.
- While live, the host can add songs, reorder upcoming songs, and remove upcoming songs. The currently playing song is locked in the queue.
- Songs transition gaplessly: no intentional silence and no crossfade. All listeners switch at the same sample boundary.
- If the queue reaches its end, playback stops and the host session remains open. Adding songs to an idle queue does not start them automatically; the host presses Play.
- Stop halts the current song but preserves the queue. Play starts the current song from its beginning; upcoming songs remain in order.
- The listener never selects or receives host audio files. The listener joins the session and hears the host's current playlist.
- Android and desktop expose the same user-visible workflow and queue behavior.
- Primary workflow controls stay visible without page scrolling. If a task needs more space, it gets a distinct step or screen with persistent navigation/actions.
- Visual language follows Material 3 Expressive: clear hierarchy, expressive but controlled shape/color/motion, readable type, accessible contrast, and consistent component behavior across Android and desktop.

## 3. Goals and non-goals

### Goals

- Replace the single-file host setup assumption with a first-class ordered playlist.
- Allow adding songs both before and during a session.
- Keep the current stream uninterrupted when the host edits upcoming songs.
- Prepare the next track before the current track ends and preserve a continuous listener audio output across track boundaries.
- Guide hosts and clients through short, role-specific workflows.
- Keep primary controls and status visible on compact Android screens and desktop windows.
- Make the purpose of each screen apparent through labels, layout, and state rather than explanatory prose.
- Surface errors at the action that caused them and provide a direct recovery action.

### Non-goals

- Crossfade or automatic mixing between songs.
- A client-side playlist editor or independent client song selection.
- Streaming music from third-party services; the initial queue contains supported local audio files.
- Replacing the established discovery, approval, invite, sync, or transport semantics.
- A wholesale visual redesign of unrelated Settings, diagnostics, or support-report screens beyond shared design tokens/components needed by these workflows.

## 4. Current-state findings

The supplied master snapshot stores one host audio source in the host draft. Android decodes that source and packetizes it as one stream; desktop stages and streams one source. Natural end-of-file stops playback. The listener path already keeps its native audio output stream session-owned and rebinds playback per track, which is a useful foundation for gapless work. However, current per-track Stop/Start behavior must not be assumed to be gapless: the existing code and protocol need an explicit audit against sample-boundary continuity.

The existing mobile workflow has host music/access steps and a role-first join path. The previous Android UX redesign ledger marks its software work complete, but this new scope adds playlist management, gapless playback, and equivalent desktop workflow acceptance. Existing desktop host UI remains a separate implementation surface.

## 5. Host workflow

### 5.1 Entry and setup wizard

The host starts from the role-first home screen and chooses **Host a disco**. The host wizard is a short, visible sequence:

1. **Music** — select one or more supported files, see the ordered playlist, reorder/remove entries, and identify the first song.
2. **Access** — set session name and listener access/approval mode. Invite-code entry appears only when that mode is selected.
3. **Ready** — concise summary of session name, song count, first song, and access mode; primary action starts the host session.

Wizard navigation uses persistent Back/Next and step indicators. Validation is shown beside the affected field or song. The main page does not require vertical scrolling to find an action. A long playlist opens a dedicated **Manage queue** screen with pinned Add, Done, and navigation actions; the queue contents may scroll inside that screen, but controls do not disappear with the list.

### 5.2 Live host screen

The live host screen keeps these elements visible in the primary viewport:

- session name and live/ready status;
- current song, playback position, and progress;
- Play/Pause and Stop;
- next song and queued-song count;
- Add songs and Manage queue;
- listener count and a direct route to listener management/invites.

The full upcoming queue is edited on the dedicated Manage queue screen. The current song cannot be removed or reordered while playing. Reordering/removing upcoming entries is atomic and cannot disturb current playback or the already-buffered audio except that a removed/reordered prefetched track must be safely cancelled and the correct next track prepared.

### 5.3 Host controls and queue semantics

- **Play:** start the current song from its beginning when stopped; resume at the paused position when paused.
- **Pause:** pause the continuous output timeline. Queue order and prepared next-song state remain intact.
- **Stop:** stop the current song and return its playhead to the beginning. Preserve current selection and all upcoming entries.
- **Add songs:** append selected songs after the existing upcoming entries. It never replaces the current song.
- **Manage queue:** reorder or remove upcoming entries; current item is visibly locked.
- **Queue ended:** remain in the session in a ready/stopped state. A new song added to the empty queue is shown as the current selection but requires an explicit Play action.

Selecting the same underlying file more than once is allowed as separate playlist entries. Each queue entry has its own identity even if it references the same staged source.

## 6. Client workflow

The client starts from role-first home and chooses **Join a disco**. The join wizard is:

1. **Find** — nearby sessions and Scan QR entry. Session cards show only useful identity/status details.
2. **Connect** — confirm the selected session, enter an invite code only if required, and send/join the request.
3. **Waiting** — show a clear pending-approval state when host approval is required, with Cancel in a fixed action area.
4. **Listening** — open automatically only after the client is admitted, clock synchronization is accepted, and playable audio is ready.

The listening screen shows session/song identity, synchronized playback state, connection health, and local volume. Host playback state remains authoritative: clients cannot change queue order, skip songs, or control host playback. A song transition updates the displayed track without navigating away or restarting the listener's output device.

## 7. Gapless playback and synchronization requirements

### 7.1 Definition

Gapless means the last scheduled sample of track N is immediately followed by the first scheduled sample of track N+1 on the shared host timeline. There is no inserted silence, no overlapping crossfade, and no native audio-output teardown/reopen between tracks. All listeners use the same boundary and preserve their existing clock mapping, render stream, and output device.

### 7.2 Preparation and transition

- A selected source is staged or permission-retained through the existing platform boundary and assigned an opaque source identity. Native file paths and Android URIs remain in platform adapters.
- Queue insertion validates the file type, size, metadata, source access, and decoder viability. A failed add leaves the existing queue unchanged and identifies the failed file.
- While a song plays, the host prepares the next queued item off the real-time path. Preparation includes opening the decoder, normalizing to the negotiated PCM format, and filling a bounded prebuffer before the current track reaches its end.
- Current and prefetched audio are scheduled on one continuous sample timeline. The host calculates the boundary from decoded sample counts, not rounded container duration text.
- The listener output callback remains active across a track boundary. The implementation may retain a session stream identity or introduce an explicit track-boundary control; the protocol decision must follow an audit of existing StreamStart, stream IDs, packet sequence, scheduler, and render-ring behavior.
- Queue edits cancel or retarget a prefetched next track without blocking the active sender, listener receive path, actor, or real-time callback.
- Track titles/current-next state update at the same boundary on host and client.

### 7.3 Failure behavior

Preparation is expected to succeed before a song is eligible for gapless playback. Common failures should be caught when adding or preloading a track (source access lost, file changed/corrupt, unsupported or malformed audio, storage/resource limits, decoder error). They must be visible and must not silently advance past an item.

If a late read/decode failure occurs after a track has been accepted, the runtime reports a structured failure. The active track continues when possible; otherwise playback stops at the last valid sample boundary, keeps the session open, and offers Retry/Remove failed item. This exceptional recovery path may interrupt playback; it is not the normal transition behavior. It must never corrupt the remaining queue or claim gapless success after a discontinuity.

## 8. State ownership and source safety

- Rust owns authoritative queue order, current queue-entry identity, playback state, transition timing, and protocol semantics.
- Kotlin/Compose and React/Tauri present Rust snapshots and dispatch typed queue commands.
- Platform adapters own file pickers, URI grants, staging/copying, native paths, decoder handles where platform-specific, and lifecycle-safe cancellation.
- Queue entries carry opaque source IDs plus bounded display metadata; they do not carry native paths or URIs through shared domain state or IPC.
- Queue mutations are bounded, validated, revision-aware, atomic, and observable in Rust snapshots.
- A queue item cannot be removed while it is the active current item. The next item may be edited only if the decoder handoff can be cancelled/retargeted safely.
- Ending the host session clears the active queue and releases staged handles/URI grants as appropriate. Profile close follows existing lifecycle ownership and does not leave background decoders.

## 9. Screen-fit, layout, and visual system

- No primary host/client task screen may place a necessary control below the fold with no visible indication. Primary screens do not rely on page scrolling to expose navigation, status, or actions.
- If a task cannot fit, it gets its own wizard step or full-screen subpage with a fixed header and fixed action area.
- Long collections live in focused screens with pinned actions and clear count/position. The user must not need accidental scrolling to discover a feature.
- Desktop adapts density and uses its available width for clear current/next and listener status. Android uses one-column wizard steps and a fixed bottom action area; queue editing is a separate screen.
- Shared design tokens cover color roles, expressive type scale, shape, elevation, spacing, focus, and motion. Android uses Material 3 Expressive Compose components; desktop follows the same Material 3 Expressive tokens and interaction rules in its React/Tauri shell.
- Motion is brief and meaningful (step transition, queue reorder, state change), respects reduced-motion settings, and never delays playback controls.
- Text is concise. Buttons name the action, empty states state the next action in one short line, and technical wording stays in diagnostics.
- Keyboard navigation, screen-reader labels, focus order, color contrast, touch target size, large text, and compact-window behavior are acceptance requirements.

## 10. Data, compatibility, and migration

The implementation must inspect the current Rust host draft, UniFFI API, Tauri DTO/binding generation, event/snapshot revisions, audio packetizer, listener scheduler, and output-ring lifecycle before choosing the queue representation or wire change. Preserve compatibility for existing one-song setup as a one-entry queue. Do not hand-edit generated TypeScript bindings.

Any protocol change requires versioning/compatibility tests and clear behavior for mixed versions. A listener that cannot support the gapless transition semantics must fail visibly before playback or use a documented compatible fallback that does not claim gapless behavior. Silent gap insertion is not an acceptable hidden fallback.

## 11. Acceptance criteria

### Queue behavior

1. Host can add several songs in the setup wizard and establish a deterministic order.
2. Duplicate file selection creates distinct queue entries without duplicate source staging where safe.
3. Live add/reorder/remove affects upcoming entries only and does not interrupt the active song.
4. Current track is locked while playing; Stop preserves current and upcoming entries; Play restarts current from the beginning.
5. Queue end leaves the host session available and stopped; adding a song does not auto-start it.
6. Invalid/unreadable tracks fail visibly at add/preparation time and leave prior queue state intact.
7. Queue clears and resources are released on session end/profile close.

### Gapless playback

8. Host prebuffers the next track before the current track ends using bounded queues.
9. Audio samples have contiguous presentation/sample indices across a normal track boundary.
10. Android listener output device/callback is not closed or reopened at a track boundary.
11. Every listener observes the same current-track boundary using the existing clock mapping; no listener is routed through Kotlin for per-packet audio.
12. Reordering/removing the prefetched item yields the newly selected next item without stale audio leakage.
13. Pause/resume, Stop/replay, no-listener, reconnect, late join, and end-of-queue behavior remain deterministic.
14. Tests measure boundary silence/overlap and sample continuity; state-only tests are insufficient to claim gapless behavior.

### Workflow and UI

15. Host and client complete the main flows with concise labels and without explanatory paragraphs.
16. Android and desktop expose equivalent flow stages and outcomes.
17. Primary controls remain visible at the minimum supported window/viewport without page scrolling.
18. Queue management has pinned actions and clear current/upcoming sections.
19. M3 Expressive visual tokens and accessibility checks apply to both platforms.
20. Automated tests cover screen states/navigation and physical or packaged cross-platform acceptance verifies actual gapless audio.

## 12. Design mockup notes

The companion HTML mockup is a visual direction and workflow review board, not an implementation contract for exact colors, final copy, or asset choice. It presents host setup, live hosting, client joining, and listening on both desktop and Android-sized frames. Behavior in this specification is authoritative; visual feedback can revise spacing, emphasis, copy, or route organization before implementation begins.
