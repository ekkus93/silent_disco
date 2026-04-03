# Silent Disco Android PoC — TODO.md

## 1. Project Setup

### 1.1 Create Android project skeleton
- [ ] Create Android project for PoC app
- [ ] Set package/application identifiers
- [ ] Configure min SDK / target SDK appropriately for planned networking/audio APIs
- [ ] Set up build variants if useful (`debug`, `release`, possibly `pocDebug`)
- [ ] Enable logging strategy suitable for networking/audio diagnostics

### 1.2 Establish project architecture
- [ ] Choose app architecture approach
  - [ ] Define UI layer structure
  - [ ] Define state management approach
  - [ ] Define service/session manager boundaries
- [ ] Separate code by feature area
  - [ ] host/session control
  - [ ] listener/join flow
  - [ ] transport/networking
  - [ ] sync/timing
  - [ ] audio pipeline
  - [ ] diagnostics

### 1.3 Permissions and platform plumbing
- [ ] Identify all runtime permissions required
  - [ ] nearby/wifi related permissions
  - [ ] bluetooth permissions if BLE discovery is included
  - [ ] storage/media access for local audio file selection
- [ ] Implement permission request flow
- [ ] Implement permission denied states in UI

## 2. Core Domain Model

### 2.1 Define core data models
- [ ] Create models for:
  - [ ] session
  - [ ] host info
  - [ ] listener info
  - [ ] join request
  - [ ] approval decision
  - [ ] playback state
  - [ ] sync state
  - [ ] diagnostics snapshot

### 2.2 Define role/state enums
- [ ] Create enums/state objects for host lifecycle
- [ ] Create enums/state objects for listener lifecycle
- [ ] Create enums/state objects for join/approval states
- [ ] Create enums/state objects for transport connection states
- [ ] Create enums/state objects for playback/sync health badges

### 2.3 Define protocol models
- [ ] Create message definitions for control-plane messages
- [ ] Create message definitions for sync packets
- [ ] Create message definitions for audio packets
- [ ] Create session identifiers and stream identifiers
- [ ] Define versioning field for protocol messages

## 3. UI Implementation

## 3.1 Home / Role Select screen
- [ ] Create Home screen
- [ ] Add **Host a Session** button
- [ ] Add **Join a Session** button
- [ ] Add permission/status summary area
- [ ] Wire role buttons into navigation flow

## 3.2 Create Host Session screen
- [ ] Create host session creation screen
- [ ] Add session name input
- [ ] Add approval mode selector
  - [ ] manual approval
  - [ ] trusted devices auto-approve placeholder if not implemented yet
  - [ ] invite code mode placeholder or actual implementation
- [ ] Add optional invite code input
- [ ] Add remember approved devices toggle
- [ ] Add choose audio file action
- [ ] Add start hosting action
- [ ] Add validation/error handling for missing/invalid inputs

## 3.3 Host Control screen
- [ ] Create host control screen
- [ ] Add session status card
- [ ] Add pending join requests list
- [ ] Add approved/connected listeners list
- [ ] Add selected audio source section
- [ ] Add playback controls
  - [ ] start
  - [ ] pause
  - [ ] stop
  - [ ] end session
- [ ] Add listener actions
  - [ ] approve
  - [ ] reject
  - [ ] trust placeholder or actual trust action
  - [ ] remove/kick listener
- [ ] Add diagnostics navigation/button
- [ ] Add visible sync/health summary badges

## 3.4 Discover Sessions screen
- [ ] Create nearby session discovery screen
- [ ] Add scan/refresh action
- [ ] Add list of nearby sessions
- [ ] Add per-session card content
  - [ ] session name
  - [ ] host name
  - [ ] approval requirement
  - [ ] signal/availability hint if available
- [ ] Add join action
- [ ] Add empty state UI when no sessions found

## 3.5 Join / Approval / Connect Progress screen
- [ ] Create listener connection progress screen
- [ ] Add current join state text
- [ ] Add progress stepper/timeline
  - [ ] discovered
  - [ ] requested
  - [ ] approved
  - [ ] connected
  - [ ] synced
  - [ ] playing
- [ ] Add invite code entry if required
- [ ] Add cancel action
- [ ] Add retry action
- [ ] Add rejection/error UI states

## 3.6 Listener Playback screen
- [ ] Create listener playback screen
- [ ] Add session header
- [ ] Add host/session info
- [ ] Add sync quality indicator
- [ ] Add now-playing section
- [ ] Add playback state text
- [ ] Add local volume control
- [ ] Add leave session action
- [ ] Add reconnect action if needed
- [ ] Add diagnostics navigation/button

## 3.7 Diagnostics screens/panels
- [ ] Implement diagnostics UI for host
- [ ] Implement diagnostics UI for listener
- [ ] Add copy/share debug info action if useful
- [ ] Ensure diagnostics are easy to access during testing

## 4. Navigation and State Flow

### 4.1 Navigation graph
- [ ] Implement root navigation flow
  - [ ] Home → Host flow
  - [ ] Home → Listener flow
- [ ] Implement Host flow navigation
- [ ] Implement Listener flow navigation
- [ ] Implement return-to-home behavior after session end/leave

### 4.2 State-driven UI updates
- [ ] Bind host lifecycle state to Host Control UI
- [ ] Bind listener lifecycle state to Join/Playback UI
- [ ] Ensure error states show correct actions and messages
- [ ] Ensure reconnect/resync states are visible

## 5. Audio File Selection and Source Handling

### 5.1 Local audio source selection
- [ ] Implement host-side local audio file picker
- [ ] Validate supported file types for PoC
- [ ] Display selected file metadata
- [ ] Handle file access errors cleanly

### 5.2 Audio decode pipeline
- [ ] Implement or integrate decode path on host side
- [ ] Convert/normalize decoded audio into chosen stream format
  - [ ] 48 kHz
  - [ ] stereo
  - [ ] 16-bit PCM
- [ ] Verify stable frame generation at fixed packet duration
- [ ] Handle end-of-file behavior

## 6. Host Networking / Session Management

### 6.1 Host session lifecycle
- [ ] Implement session creation service/manager
- [ ] Generate session id
- [ ] Start host advertisement/discovery availability
- [ ] Maintain session state
- [ ] End/teardown session cleanly

### 6.2 Wi-Fi Direct transport setup
- [ ] Implement Wi-Fi Direct-based host setup
- [ ] Implement discovery/connection flow for listeners
- [ ] Abstract transport setup behind an interface so it can evolve later
- [ ] Handle transport errors and retries

### 6.3 Optional BLE discovery support
- [ ] Decide whether BLE discovery is phase 1 or phase 2 within PoC
- [ ] If implemented:
  - [ ] create BLE advertisement model
  - [ ] advertise minimal host metadata
  - [ ] implement listener-side scanning
  - [ ] bridge BLE discovery into Wi-Fi Direct join flow
- [ ] If deferred:
  - [ ] leave abstraction hooks/placeholders

## 7. Join Request and Approval Flow

### 7.1 Listener join request
- [ ] Implement join request initiation from listener
- [ ] Send device/session metadata required for host approval
- [ ] Support optional invite code field

### 7.2 Host approval flow
- [ ] Deliver pending join requests to host UI
- [ ] Implement manual approve action
- [ ] Implement reject action
- [ ] On approval, allow listener to proceed to active connection/session path
- [ ] On rejection, notify listener clearly

### 7.3 Trust model placeholders
- [ ] Add trust-state model for future use
- [ ] Implement session-only approval now
- [ ] Optionally persist trusted-device records if simple to do in PoC

## 8. Protocol and Message Framing

### 8.1 Control-plane messages
- [ ] Define concrete message schema for:
  - [ ] hello/session announce
  - [ ] join request
  - [ ] join approval
  - [ ] join rejection
  - [ ] heartbeat
  - [ ] stream start
  - [ ] pause
  - [ ] stop
  - [ ] disconnect
  - [ ] resync request/notice

### 8.2 Sync messages
- [ ] Define concrete sync packet structure
- [ ] Include four-timestamp exchange fields
- [ ] Include sequence/correlation identifiers for sync requests

### 8.3 Audio messages
- [ ] Define concrete audio packet structure
- [ ] Include required header fields
  - [ ] version
  - [ ] session id
  - [ ] stream id
  - [ ] packet sequence
  - [ ] sample rate
  - [ ] channels
  - [ ] samples per packet
  - [ ] first sample index
  - [ ] host presentation timestamp
  - [ ] payload
- [ ] Choose serialization/framing approach
- [ ] Validate packet sizes and overhead

## 9. Clock Sync Implementation

### 9.1 Host timing service
- [ ] Implement host timing endpoint/service
- [ ] Stamp incoming sync requests with host receive/send times
- [ ] Return sync responses quickly and predictably

### 9.2 Listener sync client
- [ ] Implement repeated sync probing
- [ ] Capture local send/receive timestamps
- [ ] Compute per-sample offset estimate
- [ ] Compute RTT estimate
- [ ] Store sync sample history

### 9.3 Sync estimation logic
- [ ] Reject bad/outlier samples
- [ ] Prefer lower RTT samples
- [ ] Calculate stable initial offset estimate
- [ ] Optionally implement skew estimate model
- [ ] Expose sync confidence/quality state to UI and diagnostics

### 9.4 Ongoing sync maintenance
- [ ] Run periodic re-sync during session
- [ ] Update offset estimate over time
- [ ] Detect drift growth or unstable timing
- [ ] Trigger resync states when thresholds are exceeded

## 10. Host Audio Streaming Pipeline

### 10.1 Host packetization
- [ ] Read decoded PCM frames in fixed packet windows
- [ ] Segment into 20 ms packets
- [ ] Assign sample index timeline
- [ ] Assign future host presentation timestamps
- [ ] Queue/send packets to listeners

### 10.2 Stream control
- [ ] Implement stream start message with future start time
- [ ] Implement pause behavior
- [ ] Implement stop behavior
- [ ] Ensure listeners receive state changes consistently

### 10.3 Host pacing
- [ ] Ensure packet send timing is stable
- [ ] Prevent bursty or poorly paced packet output where possible
- [ ] Instrument packet send timing for diagnostics

## 11. Listener Audio Pipeline

### 11.1 Listener receive path
- [ ] Receive audio packets
- [ ] Validate session/stream identifiers
- [ ] Order packets by sequence/sample index
- [ ] Detect missing packets
- [ ] Drop packets that are too late to use

### 11.2 Listener buffer implementation
- [ ] Build jitter/playback buffer
- [ ] Store packets by stream timeline
- [ ] Track current fill depth in time and/or samples
- [ ] Expose buffer health to diagnostics/UI

### 11.3 Playback scheduling
- [ ] Translate host presentation time into local playback deadline
- [ ] Start playback only after minimum startup buffer achieved
- [ ] Feed scheduled audio into playback engine

### 11.4 Audio output engine
- [ ] Implement playback path using appropriate Android audio APIs
- [ ] Prefer Oboe / AAudio-oriented implementation if feasible
- [ ] Verify stable playback callback/write behavior
- [ ] Expose playback timestamp/position if possible

## 12. Missing Packet / Underrun Handling

### 12.1 Packet loss handling
- [ ] Detect missing packet ranges
- [ ] Decide simple concealment behavior for PoC
  - [ ] silence fill
  - [ ] zero-fill
  - [ ] minimal gap handling
- [ ] Count/report packet loss events

### 12.2 Late packet handling
- [ ] Detect unusably late packets
- [ ] Drop late packets when necessary
- [ ] Count/report late-drop events

### 12.3 Underrun handling
- [ ] Detect playback underruns
- [ ] Surface underruns in diagnostics/UI
- [ ] Define simple recovery behavior

## 13. Drift Correction and Resync

### 13.1 Initial drift management
- [ ] Maintain target startup/playback buffer
- [ ] Use periodic resync to keep offset estimate current
- [ ] Detect growing playback error

### 13.2 Correction policy
- [ ] Implement simple PoC correction strategy first
- [ ] Avoid advanced time-stretch unless clearly needed
- [ ] Define threshold for soft correction
- [ ] Define threshold for hard resync/rebuffer state

### 13.3 Resync UI/behavior
- [ ] Expose resync state to listeners
- [ ] Expose listener sync trouble to host
- [ ] Provide manual resync action in diagnostics if useful

## 14. Host and Listener Diagnostics

### 14.1 Host diagnostics data
- [ ] Implement listener roster snapshot
- [ ] Track per-listener state
- [ ] Track per-listener last heartbeat / last contact
- [ ] Track stream state and packet send counts
- [ ] Track join/pending/approved counts

### 14.2 Listener diagnostics data
- [ ] Track sync offset estimate
- [ ] Track RTT estimate
- [ ] Track jitter estimate if implemented
- [ ] Track current buffer depth
- [ ] Track packet loss count
- [ ] Track underrun count
- [ ] Track reconnect/resync count
- [ ] Track current playback state

### 14.3 Diagnostics presentation
- [ ] Render diagnostics cleanly in UI
- [ ] Keep values readable during live testing
- [ ] Allow copying/exporting diagnostic summary for debugging

## 15. Error Handling and Recovery

### 15.1 Host errors
- [ ] Handle failure to create/start session
- [ ] Handle failure to advertise/connect transport
- [ ] Handle audio file load/decode failure
- [ ] Handle stream failure/stoppage

### 15.2 Listener errors
- [ ] Handle no sessions found
- [ ] Handle host rejection
- [ ] Handle connection failure
- [ ] Handle sync establishment failure
- [ ] Handle disconnect during playback
- [ ] Handle session disappearance

### 15.3 UI recovery flows
- [ ] Add retry actions where appropriate
- [ ] Add clear error messages
- [ ] Add back/leave/end session recovery actions

## 16. Logging and Observability

### 16.1 Structured logging
- [ ] Add structured logs for:
  - [ ] join flow
  - [ ] approval actions
  - [ ] transport connection lifecycle
  - [ ] sync sampling
  - [ ] stream start/stop
  - [ ] packet send/receive anomalies
  - [ ] playback underruns/desyncs

### 16.2 Debug instrumentation
- [ ] Add counters/timers for key performance indicators
- [ ] Make logs useful for real-device testing
- [ ] Avoid logging so much that it breaks timing-sensitive paths

## 17. Test Strategy

### 17.1 Unit-level tests where practical
- [ ] Test protocol model serialization/deserialization
- [ ] Test sync math helpers
- [ ] Test packet ordering helpers
- [ ] Test buffer bookkeeping logic
- [ ] Test state transitions

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

- [ ] Decide whether BLE discovery is included in phase 1 PoC or deferred
- [ ] Choose exact packet framing/serialization format
- [ ] Tune sync sample count and cadence
- [ ] Tune startup buffer threshold
- [ ] Tune late packet / desync thresholds
- [ ] Decide whether skew estimation is required immediately
- [ ] Measure actual listener capacity of a host phone
- [ ] Decide whether Host Control and Host Playback remain a single screen or split later

## 19. Nice-to-Haves if Time Permits

- [ ] QR code or invite code UX improvement
- [ ] Remember trusted devices
- [ ] Better diagnostics export
- [ ] Manual listener resync action
- [ ] Better connection quality visualization

## 20. Explicitly Deferred to Later Versions

- [ ] iOS support
- [ ] mesh/relay/multi-hop networking
- [ ] star topology with fallback connections
- [ ] host failover/election
- [ ] large-group scaling
- [ ] polished consumer-grade design
- [ ] playlists/social features
- [ ] online or hybrid connectivity
