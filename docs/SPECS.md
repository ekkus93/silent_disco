# Silent Disco Android PoC — SPECS.md

## 1. Overview

This project is an Android-only proof of concept for an offline silent disco app. The goal is to determine whether a group of Android phones can play the same audio in sufficiently tight sync, without requiring internet access.

This is not a production app spec. It is a viability-focused PoC spec.

## 2. Primary Goal

Validate whether one Android phone can act as:
- the Wi-Fi/session host
- the audio source host
- the authoritative time host

while multiple listener phones:
- discover the host nearby
- request approval to join
- connect locally without internet
- receive timestamped audio packets
- buffer audio ahead of playback
- play the audio at scheduled local times derived from the host clock

## 3. Success Criteria

The PoC is considered viable if it demonstrates all of the following on real devices:

1. A host can create an offline session.
2. Nearby listener phones can discover the session and request to join.
3. The host can manually approve listeners.
4. Listeners can connect and begin synchronized playback.
5. Synchronized playback remains stable for several minutes.
6. Sync quality is perceptually acceptable for a small group.
7. Diagnostics expose enough information to evaluate sync accuracy, drift, jitter, buffer health, and failures.

## 4. Non-Goals for the PoC

The following are explicitly out of scope for this phase:
- iOS support
- internet-based sessions
- cloud accounts or login
- music catalog or streaming service integration
- playlist collaboration
- relay nodes or mesh networking
- star topology with fallback routing
- distributed host election
- polished consumer-grade UI
- large-crowd scaling optimization
- production security hardening

## 5. Platform Scope

- Platform: Android only
- Connectivity: offline / local-only
- Session topology: single host, multiple direct listeners
- Group size target for PoC: small group, roughly 2–6 total devices as an initial validation target

## 6. High-Level Architecture

### 6.1 Roles

There are two primary roles:

#### Host
The host is responsible for:
- creating the session
- advertising/discovering availability
- approving or rejecting join requests
- serving as the authoritative session clock
- serving as the audio source
- packetizing and sending timestamped audio
- monitoring listener health

#### Listener
The listener is responsible for:
- discovering nearby sessions
- requesting to join a session
- waiting for approval
- connecting to the host
- estimating offset to the host clock
- buffering audio packets
- scheduling local playback
- reporting local health/diagnostics

### 6.2 Topology

The PoC uses a strict star topology:
- one host
- many direct listeners
- no relays
- no multi-hop forwarding
- no fallback topology changes during playback

## 7. Networking Model

### 7.1 Primary Transport

Primary session/data transport for the PoC:
- Wi-Fi Direct

Rationale:
- offline capable
- suitable for local peer-to-peer data transport
- better fit for audio payload delivery than BLE

### 7.2 BLE Usage

BLE is optional for the PoC and should be treated as a discovery/metadata assist only.

BLE may be used for:
- host advertisement
- lightweight discovery
- metadata exchange before transport setup
- join request assistance

BLE must not be used for:
- primary audio transport
- authoritative timing/sync transport for playback correctness

### 7.3 No Internet Requirement

The session must function without:
- internet access
- cloud services
- WAN connectivity
- cellular service

## 8. Session Clock and Sync Model

### 8.1 Clock Type

The system must use a monotonic clock, not wall-clock time.

Required conceptual clock source on Android:
- monotonic elapsed time suitable for scheduling and sync

Do not use:
- wall clock timestamps
- timezone-sensitive time
- user-adjustable system time

### 8.2 Authoritative Timeline

The host defines the authoritative session timeline.
All listeners map the host clock onto their own local monotonic clock.

### 8.3 Sync Strategy

Use repeated NTP-style four-timestamp exchange between each listener and the host.

For each sync probe:
- listener records send time `t1`
- host records receive time `t2`
- host records response send time `t3`
- listener records receive time `t4`

Listener calculates:
- estimated host offset
- round-trip time
- jitter confidence based on repeated samples

### 8.4 Sync Model

Listeners maintain a model that maps local monotonic time to host time.

Initial PoC requirement:
- offset estimation is mandatory
- skew estimation is recommended if practical

### 8.5 Sync Maintenance

Listeners perform:
- initial pre-play sync sampling
- ongoing periodic re-sync while connected

The system must support:
- rejection of bad sync samples
- low-RTT sample preference
- re-sync during playback

## 9. Audio Model

### 9.1 Audio Source Authority

The host owns the current stream and defines the authoritative media timeline.

Listeners do not independently decode local copies of the same file for the PoC.

### 9.2 Initial PoC Audio Format

Recommended v1/PoC baseline:
- PCM
- 16-bit
- stereo
- 48 kHz

This is selected for implementation simplicity and timing predictability.

### 9.3 Packet Duration

Recommended initial packet duration:
- 20 ms

This is a starting point and may be tuned during implementation/testing.

### 9.4 Audio Engine

Playback implementation should target Android audio APIs suitable for low-latency and reliable scheduling.

Recommended direction:
- Oboe / AAudio oriented playback path where feasible

## 10. Audio Packet Model

Each audio packet must carry enough metadata for listeners to reconstruct stream order and schedule playback.

### Required conceptual fields
- protocol version
- session id
- stream id
- sequence number
- codec / format id
- sample rate
- channel count
- samples per packet
- first sample index in stream timeline
- authoritative host presentation time for first sample
- payload bytes
- optional checksum / integrity field

### Key rule

Packets must be scheduled for playback using the authoritative host presentation time mapped into local listener time.

Packets must not be played immediately upon arrival.

## 11. Buffering and Playback Strategy

### 11.1 Scheduled Playback

Listeners must:
- buffer ahead of playback
- translate host presentation time into local playback deadline
- schedule audio accordingly

### 11.2 Initial Buffer Target

Recommended initial target buffer:
- approximately 300–500 ms

This is intentionally conservative for PoC stability.

### 11.3 Playback Start Rule

Listeners must not begin playback until:
- approved and connected
- clock sync is established
- minimum startup buffer threshold is reached
- playback start time is known and in the future

### 11.4 Missing or Late Packets

Initial PoC behavior:
- do not block forever waiting for missing packets
- late packets may be dropped if no longer usable
- concealment strategy may be simple silence fill or minimal placeholder behavior

## 12. Drift and Correction Policy

The system must assume clocks drift over time.

### Initial PoC correction strategy
- periodic re-sync
- maintain healthy target buffer
- detect desync and underruns
- allow simple correction strategies before complex resampling/time-stretch

### PoC guidance
Avoid sophisticated time-stretch first unless testing proves it necessary.

### Required behavior
The app must expose diagnostics for:
- offset drift
- jitter
- packet loss
- underruns
- resync events

## 13. Session Authorization Model

### 13.1 Discovery vs Authorization

Discovery and authorization are separate.

A device discovering a session must not imply automatic admission.

### 13.2 Join Model

Recommended PoC join flow:
1. listener discovers host
2. listener sends join request
3. host manually approves or rejects
4. only approved listener proceeds to active session connection/playback path

### 13.3 Default Approval Policy

Default PoC policy:
- manual approval required

### 13.4 Optional Approval Features

Optional for PoC or near-future:
- invite code
- trusted device memory
- trust-for-session vs trust-forever

### 13.5 Explicit Design Constraint

The host must remain in control of admission.
The system must not silently auto-connect arbitrary nearby devices into the session.

## 14. UI Requirements

The UI is role-based and must support both host and listener workflows.

### 14.1 UI Philosophy for PoC

The PoC UI is a control/testing interface, not a polished consumer music app.

It must clearly answer:
- am I hosting or joining?
- am I connected?
- am I approved?
- am I synchronized?
- what is playing?
- is the session healthy?

### 14.2 Primary Screens

#### 1. Home / Role Select
Elements:
- Host a Session button
- Join a Session button
- basic permission/status indicators

#### 2. Create Host Session
Elements:
- session name input
- approval mode controls
- optional invite code input
- remember approved devices toggle
- choose audio file button
- start hosting button

#### 3. Host Control Screen
Elements:
- session status card
- pending join requests list
- approved/connected listener list
- selected audio file information
- Start / Pause / Stop / End Session controls
- diagnostics access

#### 4. Discover Sessions
Elements:
- nearby sessions list
- session cards with host/session metadata
- join button
- scan/refresh action

#### 5. Join / Approval / Connect Progress Screen
Elements:
- current state text
- progress indicator / step state
- retry/cancel actions
- invite code input if applicable

#### 6. Listener Playback Screen
Elements:
- session info
- now playing info
- sync quality indicator
- buffer/connection status summary
- local volume control
- leave session button
- diagnostics access

#### 7. Diagnostics Screen
Must exist for both host and listener roles.

Host diagnostics should show:
- session id
- listener count
- per-listener health summary
- packet/send stats
- stream state

Listener diagnostics should show:
- host/session id
- clock offset estimate
- RTT/jitter estimate
- buffer depth
- packet loss
- underruns
- last packet state
- playback/sync state

### 14.3 Navigation Flow

#### Host Flow
Home → Create Host Session → Host Control Screen → Diagnostics (optional) → End Session → Home

#### Listener Flow
Home → Discover Sessions → Join/Approval/Connect Progress → Listener Playback → Diagnostics (optional) → Leave Session → Home

## 15. UI State Model

### 15.1 Host States
- idle
- creating session
- advertising
- waiting for listeners
- ready
- streaming
- paused
- ending session
- error

### 15.2 Listener States
- idle
- scanning
- session selected
- join requested
- awaiting approval
- approved
- connecting
- syncing clock
- buffering
- playing
- reconnecting
- desynced
- disconnected
- error

These states should be reflected both in UI and internal state handling.

## 16. Diagnostics Requirements

Diagnostics are mandatory for the PoC.

### Host diagnostics minimums
- listener roster and state
- join request state
- stream state
- packet send counts/rates
- listener health summary

### Listener diagnostics minimums
- sync offset estimate
- RTT/jitter estimate
- buffer depth
- packet loss count
- underrun count
- playback state
- reconnect/resync count

## 17. Error Handling Requirements

The app must make failures visible.

### Host-side examples
- failed to create/start session
- failed to advertise
- failed to load audio file
- stream stopped unexpectedly
- listener dropped

### Listener-side examples
- session disappeared
- host rejected join
- transport connection failed
- sync establishment failed
- desync detected
- playback underrun
- disconnected during playback

### UI requirement
Every major failure state must have a visible user-facing indicator and a recovery path where appropriate.

## 18. Security / Trust for PoC

This PoC does not aim for full production-grade security, but it must still implement basic admission control.

Minimum expectations:
- explicit join request
- explicit host approval by default
- no silent auto-admission
- session-bound listener identity at least sufficient for host review

## 19. Open Questions / Items to Validate During Implementation

The following are intentionally left as implementation-level validation items for the PoC:

1. Exact Wi-Fi Direct connection/setup UX details on target Android versions
2. Whether BLE discovery is implemented in PoC phase 1 or deferred
3. Exact packet framing and serialization format
4. Exact sync sampling counts and cadence
5. Exact startup buffer target after testing
6. Exact resync thresholds
7. Exact listener count limit for acceptable performance on real phones
8. Whether skew estimation is necessary in phase 1 or phase 2
9. Whether simple correction is sufficient without time-stretch
10. Whether Host Control and Host Playback remain one screen or are split later

## 20. Future Directions (Out of Scope Now)

Potential later directions include:
- larger group scaling
- relay nodes
- star topology with fallback connections
- hierarchical sync distribution
- trusted devices auto-join modes
- richer invite flows
- iOS port
- productionized UI/UX
- stronger security/authentication
- local library management / playlist support

## 21. Implementation Guidance Summary

Build the PoC to validate synchronization and session viability first.

Optimization order:
1. offline connectivity
2. approval-based joining
3. clock sync
4. timestamped packet playback
5. diagnostics and observability
6. tuning and stabilization
7. UX polish later
