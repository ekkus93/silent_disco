//! Real-external-listener plumbing shared by every manual device test:
//! building the connection payload a phone pastes in, driving a real
//! Android emulator through the app's "Connect manually" flow via `adb`,
//! polling for a real join request, and printing host-side diagnostics.

use super::super::harness::{next_transport_effect, submit, wait_snapshot_for};
use super::MANUAL_TEST_TIMEOUT;
use crate::platform::network::DesktopHostNetworkControl;
use silent_disco_core::domain::PlaybackState;
use silent_disco_core::runtime::{CoreCommand, CoreNotification, CoreSnapshot};
use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub(super) fn connection_payload_json(
    address: Ipv4Addr,
    endpoint: &silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
) -> String {
    format!(
        "{{\"hostAddress\":\"{address}\",\"controlPort\":{},\"syncPort\":{},\"audioPort\":{},\"sessionId\":\"{}\",\"protocolVersion\":{},\"inviteCodeRequired\":false,\"expiresAtMs\":null}}",
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        advertisement.session_id.as_str(),
        advertisement.protocol_version,
    )
}

pub(super) fn print_connection_payload(
    address: Ipv4Addr,
    endpoint: &silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
) {
    eprintln!(
        "=== paste this connection payload into the Android app's Connect manually screen ==="
    );
    eprintln!(
        "{}",
        connection_payload_json(address, endpoint, advertisement)
    );
}

/// Backslash-escapes the JSON special characters `adb shell input text`
/// otherwise silently drops. Confirmed empirically: an unescaped payload
/// arrived in the app's `EditText` missing every `{`, `}`, `"`, and `,`,
/// which the app then correctly rejected as invalid JSON -- a good example
/// of the app doing the right thing with bad input, but useless for driving
/// it automatically. Escaping fixes the input path, not the app.
fn escape_for_adb_input_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if matches!(ch, '{' | '}' | '"' | ',') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Drives the app's "Connect manually" flow on a real Android
/// emulator/device via `adb` + `uiautomator dump` text lookup -- not
/// hardcoded coordinates, so it tolerates whichever permission dialogs
/// happen to already be granted on `serial` (first-run location/nearby-
/// device prompts vs. an emulator that already granted them). Manual-test-
/// only external dependency, same pattern as `encode_with_ffmpeg` in
/// `manual::melody`: requires `adb` and `uiautomator` (bundled with the Android SDK) on
/// `PATH`, and the app already installed on `serial`. Panics loudly on
/// failure (missing `adb`, or the automation script itself exiting
/// non-zero) rather than silently giving up partway through.
pub(super) fn automate_manual_connect(serial: &str, payload_json: &str) {
    let escaped = escape_for_adb_input_text(payload_json);
    let status = Command::new("bash")
        .arg("-c")
        .arg(AUTOMATE_CONNECT_SCRIPT)
        .arg("automate_connect") // becomes $0 inside the script
        .arg(serial)
        .arg(&escaped)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => panic!(
            "bash is not on PATH -- required to run the adb UI-automation script for {serial}"
        ),
        Err(error) => panic!("failed to run adb UI-automation script for {serial}: {error}"),
    };
    assert!(
        status.success(),
        "adb UI automation failed for {serial} -- see the script's own output above for which \
         step failed (adb/uiautomator on PATH? app installed on {serial}?)"
    );
}

/// Reusable across every manual multi-device test: force-stops and
/// relaunches the app, navigates role selection -> "Find a session",
/// tolerantly taps through whichever of the nearby-access/location/nearby-
/// devices dialogs actually appear (in order, skipping any that don't),
/// reaches "Connect manually", types the escaped payload, dismisses the
/// keyboard (it otherwise visually covers the Connect button -- confirmed
/// empirically, not a guess), and taps Connect.
const AUTOMATE_CONNECT_SCRIPT: &str = r#"
set -euo pipefail
SERIAL="$1"
PAYLOAD="$2"
DUMP="$(mktemp)"
trap 'rm -f "$DUMP"' EXIT

dump_ui() {
  adb -s "$SERIAL" shell uiautomator dump /sdcard/window_dump.xml >/dev/null 2>&1
  adb -s "$SERIAL" pull /sdcard/window_dump.xml "$DUMP" >/dev/null 2>&1
}

tap_exact_text() {
  local target="$1"
  local bounds
  bounds=$(grep -o "text=\"$target\"[^>]*bounds=\"\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]\"" "$DUMP" \
    | grep -o '\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]' | head -1) || true
  if [ -z "$bounds" ]; then
    return 1
  fi
  local x1 y1 x2 y2
  x1=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\1/')
  y1=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\2/')
  x2=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\3/')
  y2=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\4/')
  local cx=$(( (x1 + x2) / 2 ))
  local cy=$(( (y1 + y2) / 2 ))
  echo "  [$SERIAL] tapping \"$target\" at $cx,$cy"
  adb -s "$SERIAL" shell input tap "$cx" "$cy"
  return 0
}

try_tap_any() {
  local deadline=$((SECONDS + 10))
  while [ $SECONDS -lt $deadline ]; do
    dump_ui
    for candidate in "$@"; do
      if tap_exact_text "$candidate"; then
        sleep 1
        return 0
      fi
    done
    sleep 1
  done
  echo "  [$SERIAL] none of [$*] appeared within 10s; continuing"
  return 0
}

echo "[$SERIAL] relaunching app"
adb -s "$SERIAL" shell am force-stop com.ekkus.silentdisco
adb -s "$SERIAL" shell monkey -p com.ekkus.silentdisco -c android.intent.category.LAUNCHER 1 >/dev/null
sleep 3

echo "[$SERIAL] role screen -> Find a session"
try_tap_any "Find a session"
echo "[$SERIAL] optional nearby-access app dialog"
try_tap_any "Continue"
echo "[$SERIAL] optional system location dialog"
try_tap_any "While using the app"
echo "[$SERIAL] optional system nearby-devices dialog"
try_tap_any "Allow"
echo "[$SERIAL] nearby-session screen -> Connect manually"
try_tap_any "Connect manually"

dump_ui
echo "[$SERIAL] tapping payload field"
tap_exact_text "Connection payload" || echo "  [$SERIAL] payload field not found by exact text; trying anyway"
sleep 1
adb -s "$SERIAL" shell input text "$PAYLOAD"
sleep 1
adb -s "$SERIAL" shell input keyevent KEYCODE_BACK
sleep 1

echo "[$SERIAL] waiting for the payload validation summary to render"
deadline=$((SECONDS + 10))
while [ $SECONDS -lt $deadline ]; do
  dump_ui
  if grep -q 'text="Protocol version: ' "$DUMP"; then
    break
  fi
  sleep 1
done

echo "[$SERIAL] tapping Connect"
tap_exact_text "Connect" || { echo "[$SERIAL] Connect button not found"; exit 1; }
sleep 2
echo "[$SERIAL] automation done"
"#;

/// Polls for a real external device's join request (unlike the automated
/// suite's `join_and_approve_listener`, which connects a loopback listener
/// itself) and approves it. Shared by every manual device test in this
/// module.
pub(super) fn wait_for_real_join_and_approve(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    network: &DesktopHostNetworkControl,
) {
    eprintln!("waiting up to 8 minutes for a real join request...");
    let deadline = Instant::now() + Duration::from_mins(8);
    let joined = loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if !snapshot.pending_join_requests.is_empty() {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a real join request"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let request = &joined.pending_join_requests[0];
    eprintln!(
        "real join request received from device_id={} display_name={}",
        request.device_id.as_str(),
        request.display_name
    );
    let request_id = request.request_id.clone();
    submit(
        handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");

    // `dispatch_transport_effect` only enqueues the send onto the transport
    // worker and returns -- it does not block until the real delivery
    // completes. The worker reports that back asynchronously as a
    // `TransportEvent::DeliveryCompleted` fact, which is what actually
    // moves this device from `pending_join_requests` into `listeners`
    // (`record_approved_listener`). Returning as soon as dispatch merely
    // *queues* the send, without waiting for that fact to land, is exactly
    // the gap that let two real listeners both report "approved" while the
    // snapshot briefly had zero, then one, connected listener -- confirmed
    // empirically against two real Android emulators before this fix.
    let device_id = request.device_id.clone();
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.listeners.iter().any(|l| l.device_id == device_id),
        MANUAL_TEST_TIMEOUT,
    );
    eprintln!("approved and authorized.");
}

/// Pauses playback, holds it long enough for a human to notice the silence,
/// resumes, and prints diagnostics at each step. Block 28.1's "exercise
/// pause/resume/stop" checklist item, safe now that this same session's
/// Block 27.3 work fixed both the duplicate-Start and duplicate-Resume
/// bugs it found while implementing that checklist item on the automated
/// side.
pub(super) fn exercise_pause_resume(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    network: &DesktopHostNetworkControl,
    label: &str,
) {
    eprintln!("=== pausing -- you should hear the audio stop ===");
    network.pause_playback().expect("pause playback");
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.playback_state == PlaybackState::Paused,
        MANUAL_TEST_TIMEOUT,
    );
    print_diagnostics(handle, network, &format!("{label}-paused"));
    std::thread::sleep(Duration::from_secs(1));

    eprintln!("=== resuming -- audio should continue from where it paused, not restart ===");
    network.resume_playback().expect("resume playback");
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.playback_state == PlaybackState::Playing,
        MANUAL_TEST_TIMEOUT,
    );
    print_diagnostics(handle, network, &format!("{label}-resumed"));
}

/// Prints everything the desktop host itself can observe for Block 28.1's
/// "record sync, RTT, packet-loss, and underrun diagnostics" checklist
/// item. Packet loss, underruns, and concealment are observed on the
/// *listener* (the Android device), not the host -- this only prints what
/// the host side actually knows (sync offset/RTT/confidence per listener,
/// plus broadcast delivery and queue pressure from Block 26.3); read the
/// rest from the Android app's own diagnostics screen.
pub(super) fn print_diagnostics(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    network: &DesktopHostNetworkControl,
    label: &str,
) -> CoreSnapshot {
    let snapshot = handle.current_snapshot().expect("current snapshot");
    for listener in &snapshot.listeners {
        match listener.synchronization {
            Some(sync) => eprintln!(
                "[{label}] listener={} sync confidence={:?} offset_ms={:.2} rtt_ms={:.2} \
                 drift_ppm={:.2}",
                listener.display_name,
                sync.confidence,
                sync.offset_ms,
                sync.round_trip_ms,
                sync.drift_ppm,
            ),
            None => eprintln!(
                "[{label}] listener={} has not yet completed a sync exchange",
                listener.display_name
            ),
        }
    }
    if let Ok(Some(active)) = network.active_host_session() {
        let broadcast = active.broadcast;
        eprintln!(
            "[{label}] broadcast: attempted={} fully_delivered={} partially_delivered={} \
             without_recipients={} queue_depth={} queue_peak={} queue_overflows={}",
            broadcast.frames_attempted,
            broadcast.frames_fully_delivered,
            broadcast.frames_partially_delivered,
            broadcast.frames_without_recipients,
            broadcast.queue_depth,
            broadcast.queue_peak_depth,
            broadcast.queue_overflows,
        );
    }
    eprintln!(
        "[{label}] packet loss / underrun / concealment are listener-side -- read them from the \
         Android app's own diagnostics screen, not from this desktop log."
    );
    snapshot
}
