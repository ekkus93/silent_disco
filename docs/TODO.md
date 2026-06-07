# Silent Disco Android PoC — TODO.md

## 1. Project Setup

### 1.1 Create Android project skeleton
- [x] Create Android project for PoC app
- [x] Set package/application identifiers
- [x] Configure min SDK / target SDK appropriately for planned networking/audio APIs
- [x] Set up build variants if useful (`debug`, `release`, possibly `pocDebug`)
- [x] Enable logging strategy suitable for networking/audio diagnostics

### 1.2 Establish project architecture
- [x] Choose app architecture approach
  - [x] Define UI layer structure
  - [x] Define state management approach
  - [x] Define service/session manager boundaries
- [x] Separate code by feature area
  - [x] host/session control
  - [x] listener/join flow
  - [x] transport/networking
  - [x] sync/timing
  - [x] audio pipeline
  - [x] diagnostics

### 1.3 Permissions and platform plumbing
- [x] Identify all runtime permissions required
  - [x] nearby/wifi related permissions
  - [x] bluetooth permissions if BLE discovery is included
  - [x] storage/media access for local audio file selection
- [x] Implement permission request flow
- [x] Implement permission denied states in UI

## 2. Core Domain Model

### 2.1 Define core data models
- [x] Create models for:
  - [x] session
  - [x] host info
  - [x] listener info
  - [x] join request
  - [x] approval decision
  - [x] playback state
  - [x] sync state
  - [x] diagnostics snapshot

### 2.2 Define role/state enums
- [x] Create enums/state objects for host lifecycle
- [x] Create enums/state objects for listener lifecycle
- [x] Create enums/state objects for join/approval states
- [x] Create enums/state objects for transport connection states
- [x] Create enums/state objects for playback/sync health badges

### 2.3 Define protocol models
- [x] Create message definitions for control-plane messages
- [x] Create message definitions for sync packets
- [x] Create message definitions for audio packets
- [x] Create session identifiers and stream identifiers
- [x] Define versioning field for protocol messages

## 3. UI Implementation

## 3.1 Home / Role Select screen
- [x] Create Home screen
- [x] Add **Host a Session** button
- [x] Add **Join a Session** button
- [x] Add permission/status summary area
- [x] Wire role buttons into navigation flow

## 3.2 Create Host Session screen
- [x] Create host session creation screen
- [x] Add session name input
- [x] Add approval mode selector
  - [x] manual approval
  - [x] trusted devices auto-approve placeholder if not implemented yet
  - [x] invite code mode placeholder or actual implementation
- [x] Add optional invite code input
- [x] Add remember approved devices toggle
- [x] Add choose audio file action
- [x] Add start hosting action
- [x] Add validation/error handling for missing/invalid inputs

## 3.3 Host Control screen
- [x] Create host control screen
- [x] Add session status card
- [x] Add pending join requests list
- [x] Add approved/connected listeners list
- [x] Add selected audio source section
- [x] Add playback controls
  - [x] start
  - [x] pause
  - [x] stop
  - [x] end session
- [x] Add listener actions
  - [x] approve
  - [x] reject
  - [x] trust placeholder or actual trust action
  - [x] remove/kick listener
- [x] Add diagnostics navigation/button
- [x] Add visible sync/health summary badges

## 3.4 Discover Sessions screen
- [x] Create nearby session discovery screen
- [x] Add scan/refresh action
- [x] Add list of nearby sessions
- [x] Add per-session card content
  - [x] session name
  - [x] host name
  - [x] approval requirement
  - [x] signal/availability hint if available
- [x] Add join action
- [x] Add empty state UI when no sessions found

## 3.5 Join / Approval / Connect Progress screen
- [x] Create listener connection progress screen
- [x] Add current join state text
- [x] Add progress stepper/timeline
  - [x] discovered
  - [x] requested
  - [x] approved
  - [x] connected
  - [x] synced
  - [x] playing
- [x] Add invite code entry if required
- [x] Add cancel action
- [x] Add retry action
- [x] Add rejection/error UI states

## 3.6 Listener Playback screen
- [x] Create listener playback screen
- [x] Add session header
- [x] Add host/session info
- [x] Add sync quality indicator
- [x] Add now-playing section
- [x] Add playback state text
- [x] Add local volume control
- [x] Add leave session action
- [x] Add reconnect action if needed
- [x] Add diagnostics navigation/button

## 3.7 Diagnostics screens/panels
- [x] Implement diagnostics UI for host
- [x] Implement diagnostics UI for listener
- [x] Add copy/share debug info action if useful
- [x] Ensure diagnostics are easy to access during testing

## 4. Navigation and State Flow

### 4.1 Navigation graph
- [x] Implement root navigation flow
  - [x] Home → Host flow
  - [x] Home → Listener flow
- [x] Implement Host flow navigation
- [x] Implement Listener flow navigation
- [x] Implement return-to-home behavior after session end/leave

### 4.2 State-driven UI updates
- [x] Bind host lifecycle state to Host Control UI
- [x] Bind listener lifecycle state to Join/Playback UI
- [x] Ensure error states show correct actions and messages
- [x] Ensure reconnect/resync states are visible

## 5. Audio File Selection and Source Handling

### 5.1 Local audio source selection
- [x] Implement host-side local audio file picker
- [x] Validate supported file types for PoC
- [x] Display selected file metadata
- [x] Handle file access errors cleanly

### 5.2 Audio decode pipeline
- [x] Implement or integrate decode path on host side
- [x] Convert/normalize decoded audio into chosen stream format
  - [x] 48 kHz
  - [x] stereo
  - [x] 16-bit PCM
- [x] Verify stable frame generation at fixed packet duration
- [x] Handle end-of-file behavior

## 6. Host Networking / Session Management

### 6.1 Host session lifecycle
- [x] Implement session creation service/manager
- [x] Generate session id
- [x] Start host advertisement/discovery availability
- [x] Maintain session state
- [x] End/teardown session cleanly

### 6.2 Wi-Fi Direct transport setup
- [x] Implement Wi-Fi Direct-based host setup
- [x] Implement discovery/connection flow for listeners
- [x] Abstract transport setup behind an interface so it can evolve later
- [x] Handle transport errors and retries
Note: the current PoC now uses Android Wi-Fi Direct peer/group orchestration plus real TCP socket channels for control, sync, and audio transport. BLE advertisement/scanning is also wired for minimal host-session discovery metadata, while loopback transport still works for same-process development/testing.

### 6.3 Optional BLE discovery support
- [x] Decide whether BLE discovery is phase 1 or phase 2 within PoC
- [x] If implemented:
  - [x] create BLE advertisement model
  - [x] advertise minimal host metadata
  - [x] implement listener-side scanning
  - [x] bridge BLE discovery into Wi-Fi Direct join flow
Note: BLE discovery is implemented in phase 1 for this PoC, so the deferred-path placeholder work is not applicable here.

## 7. Join Request and Approval Flow

### 7.1 Listener join request
- [x] Implement join request initiation from listener
- [x] Send device/session metadata required for host approval
- [x] Support optional invite code field

### 7.2 Host approval flow
- [x] Deliver pending join requests to host UI
- [x] Implement manual approve action
- [x] Implement reject action
- [x] On approval, allow listener to proceed to active connection/session path
- [x] On rejection, notify listener clearly

### 7.3 Trust model placeholders
- [x] Add trust-state model for future use
- [x] Implement session-only approval now
- [x] Optionally persist trusted-device records if simple to do in PoC

## 8. Protocol and Message Framing

### 8.1 Control-plane messages
- [x] Define concrete message schema for:
  - [x] hello/session announce
  - [x] join request
  - [x] join approval
  - [x] join rejection
  - [x] heartbeat
  - [x] stream start
  - [x] pause
  - [x] stop
  - [x] disconnect
  - [x] resync request/notice

### 8.2 Sync messages
- [x] Define concrete sync packet structure
- [x] Include four-timestamp exchange fields
- [x] Include sequence/correlation identifiers for sync requests

### 8.3 Audio messages
- [x] Define concrete audio packet structure
- [x] Include required header fields
  - [x] version
  - [x] session id
  - [x] stream id
  - [x] packet sequence
  - [x] sample rate
  - [x] channels
  - [x] samples per packet
  - [x] first sample index
  - [x] host presentation timestamp
  - [x] payload
- [x] Choose serialization/framing approach
- [x] Validate packet sizes and overhead

## 9. Clock Sync Implementation

### 9.1 Host timing service
- [x] Implement host timing endpoint/service
- [x] Stamp incoming sync requests with host receive/send times
- [x] Return sync responses quickly and predictably

### 9.2 Listener sync client
- [x] Implement repeated sync probing
- [x] Capture local send/receive timestamps
- [x] Compute per-sample offset estimate
- [x] Compute RTT estimate
- [x] Store sync sample history

### 9.3 Sync estimation logic
- [x] Reject bad/outlier samples
- [x] Prefer lower RTT samples
- [x] Calculate stable initial offset estimate
- [x] Optionally implement skew estimate model
- [x] Expose sync confidence/quality state to UI and diagnostics

### 9.4 Ongoing sync maintenance
- [x] Run periodic re-sync during session
- [x] Update offset estimate over time
- [x] Detect drift growth or unstable timing
- [x] Trigger resync states when thresholds are exceeded

## 10. Host Audio Streaming Pipeline

### 10.1 Host packetization
- [x] Read decoded PCM frames in fixed packet windows
- [x] Segment into 20 ms packets
- [x] Assign sample index timeline
- [x] Assign future host presentation timestamps
- [x] Queue/send packets to listeners

### 10.2 Stream control
- [x] Implement stream start message with future start time
- [x] Implement pause behavior
- [x] Implement stop behavior
- [x] Ensure listeners receive state changes consistently

### 10.3 Host pacing
- [x] Ensure packet send timing is stable
- [x] Prevent bursty or poorly paced packet output where possible
- [x] Instrument packet send timing for diagnostics

## 11. Listener Audio Pipeline

### 11.1 Listener receive path
- [x] Receive audio packets
- [x] Validate session/stream identifiers
- [x] Order packets by sequence/sample index
- [x] Detect missing packets
- [x] Drop packets that are too late to use

### 11.2 Listener buffer implementation
- [x] Build jitter/playback buffer
- [x] Store packets by stream timeline
- [x] Track current fill depth in time and/or samples
- [x] Expose buffer health to diagnostics/UI

### 11.3 Playback scheduling
- [x] Translate host presentation time into local playback deadline
- [x] Start playback only after minimum startup buffer achieved
- [x] Feed scheduled audio into playback engine

### 11.4 Audio output engine
- [x] Implement playback path using appropriate Android audio APIs
- [x] Prefer Oboe / AAudio-oriented implementation if feasible
- [x] Verify stable playback callback/write behavior
- [x] Expose playback timestamp/position if possible

## 12. Missing Packet / Underrun Handling

### 12.1 Packet loss handling
- [x] Detect missing packet ranges
- [x] Decide simple concealment behavior for PoC
  - [x] silence fill
  - [x] zero-fill
  - [x] minimal gap handling
- [x] Count/report packet loss events

### 12.2 Late packet handling
- [x] Detect unusably late packets
- [x] Drop late packets when necessary
- [x] Count/report late-drop events

### 12.3 Underrun handling
- [x] Detect playback underruns
- [x] Surface underruns in diagnostics/UI
- [x] Define simple recovery behavior

## 13. Drift Correction and Resync

### 13.1 Initial drift management
- [x] Maintain target startup/playback buffer
- [x] Use periodic resync to keep offset estimate current
- [x] Detect growing playback error

### 13.2 Correction policy
- [x] Implement simple PoC correction strategy first
- [x] Avoid advanced time-stretch unless clearly needed
- [x] Define threshold for soft correction
- [x] Define threshold for hard resync/rebuffer state

### 13.3 Resync UI/behavior
- [x] Expose resync state to listeners
- [x] Expose listener sync trouble to host
- [x] Provide manual resync action in diagnostics if useful

## 14. Host and Listener Diagnostics

### 14.1 Host diagnostics data
- [x] Implement listener roster snapshot
- [x] Track per-listener state
- [x] Track per-listener last heartbeat / last contact
- [x] Track stream state and packet send counts
- [x] Track join/pending/approved counts

### 14.2 Listener diagnostics data
- [x] Track sync offset estimate
- [x] Track RTT estimate
- [x] Track jitter estimate if implemented
- [x] Track current buffer depth
- [x] Track packet loss count
- [x] Track underrun count
- [x] Track reconnect/resync count
- [x] Track current playback state

### 14.3 Diagnostics presentation
- [x] Render diagnostics cleanly in UI
- [x] Keep values readable during live testing
- [x] Allow copying/exporting diagnostic summary for debugging

## 15. Error Handling and Recovery

### 15.1 Host errors
- [x] Handle failure to create/start session
- [x] Handle failure to advertise/connect transport
- [x] Handle audio file load/decode failure
- [x] Handle stream failure/stoppage

### 15.2 Listener errors
- [x] Handle no sessions found
- [x] Handle host rejection
- [x] Handle connection failure
- [x] Handle sync establishment failure
- [x] Handle disconnect during playback
- [x] Handle session disappearance

### 15.3 UI recovery flows
- [x] Add retry actions where appropriate
- [x] Add clear error messages
- [x] Add back/leave/end session recovery actions

## 16. Logging and Observability

### 16.1 Structured logging
- [x] Add structured logs for:
  - [x] join flow
  - [x] approval actions
  - [x] transport connection lifecycle
  - [x] sync sampling
  - [x] stream start/stop
  - [x] packet send/receive anomalies
  - [x] playback underruns/desyncs

### 16.2 Debug instrumentation
- [x] Add counters/timers for key performance indicators
- [x] Make logs useful for real-device testing
- [x] Avoid logging so much that it breaks timing-sensitive paths

## 17. Test Strategy

### 17.1 Unit-level tests where practical
- [x] Test protocol model serialization/deserialization
- [x] Test sync math helpers
- [x] Test packet ordering helpers
- [x] Test buffer bookkeeping logic
- [x] Test state transitions

### 17.2 Integration/device tests
- [ ] Test host creation on real Android device
- [ ] Test listener discovery/join on second device
- [ ] Test approval flow
- [ ] Test stream start and synchronized playback
- [ ] Test disconnection/reconnection scenarios
- [ ] Test with at least 2–4 devices if available

### 17.3 Manual validation checklist
- [ ] Measure startup latency
- [ ] Observe perceptual sync quality
- [ ] Record diagnostics during multi-minute playback
- [ ] Identify practical listener count ceiling
- [ ] Identify major failure cases

## 18. Open Questions to Resolve During Implementation

- [x] Decide whether BLE discovery is included in phase 1 PoC or deferred
- [x] Choose exact packet framing/serialization format
- [x] Tune sync sample count and cadence
- [x] Tune startup buffer threshold
- [x] Tune late packet / desync thresholds
- [x] Decide whether skew estimation is required immediately
- [ ] Measure actual listener capacity of a host phone
- [x] Decide whether Host Control and Host Playback remain a single screen or split later
Note: sync/window/buffer/late/resync thresholds are now exposed as persisted in-app tuning controls in Diagnostics so real-device trials can adjust them without rebuilding; the remaining capacity item still requires phone testing.

## 19. Nice-to-Haves if Time Permits

- [x] QR code or invite code UX improvement
- [x] Remember trusted devices
- [x] Better diagnostics export
- [x] Manual listener resync action
- [x] Better connection quality visualization

## 20. Explicitly Deferred to Later Versions

- [ ] iOS support
- [ ] mesh/relay/multi-hop networking
- [ ] star topology with fallback connections
- [ ] host failover/election
- [ ] large-group scaling
- [ ] polished consumer-grade design
- [ ] playlists/social features
- [ ] online or hybrid connectivity
