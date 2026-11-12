//! The first real-device run of secure transport (v4 QUIC): a real desktop
//! host with the switch on, a real Android listener that joins from the
//! signed QR invitation, scanned by a person with the phone's camera.

use super::super::harness::real_private_lan_address;
use super::super::harness::start_host_session_configured;
use super::automation::{
    assert_stream_reached_a_listener, exercise_pause_resume, print_diagnostics,
    wait_for_real_join_and_approve,
};
use super::melody::{C_MAJOR_SCALE_HZ, stage_melody_source};
use crate::platform::file_picker::{AudioContainer, SelectedSourceRegistry};
use crate::platform::invitation::{build_signed_invitation, current_wall_clock_ms};
use crate::platform::invitation_identity::tests_support::fixed_identity_for_tests;
use crate::platform::start_playback;
use silent_disco_core::transport::SECURE_V4_ENDPOINT_VERSION;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

/// Not part of the automated suite. Binds a real v4 desktop host on this
/// machine's LAN address, renders its signed QR invitation on this screen,
/// drives the phone (`SILENT_DISCO_DEVICE_TEST_ADB_SERIAL`) to its QR
/// scanner, waits for a person to scan, taps the verified-invitation dialog's
/// Connect (which connects at once), approves the join, then plays,
/// pauses and resumes one song while printing host diagnostics. Run with:
/// `SILENT_DISCO_DEVICE_TEST_ADB_SERIAL=<serial> cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_joins_over_secure_transport -- --ignored --nocapture`
/// On a carrier-NAT network also set `SILENT_DISCO_DEVICE_TEST_ALLOW_SHARED_ADDRESS_SPACE=1`.
#[test]
#[ignore = "reason: requires a real Android listener on the same LAN and a person to scan the QR; owner: secure transport v4 (remediation §1 item 2)"]
fn manual_real_android_listener_joins_over_secure_transport() {
    let serial = std::env::var("SILENT_DISCO_DEVICE_TEST_ADB_SERIAL")
        .expect("set SILENT_DISCO_DEVICE_TEST_ADB_SERIAL to the phone's adb serial");
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        "song-v4",
        &C_MAJOR_SCALE_HZ,
        1.0,
        60,
        AudioContainer::Wav,
    );
    let identity = fixed_identity_for_tests(21);
    let tls = identity.host_tls_identity().expect("host TLS identity");
    let (actor, handle, receiver, _advertisement, network, endpoint) =
        start_host_session_configured(descriptor, interface_name, interface_index, address, |n| {
            n.set_host_tls_identity(tls).expect("TLS identity");
            n.set_secure_transport(true).expect("secure transport on");
        });
    assert_eq!(endpoint.control_port, endpoint.audio_port, "one QUIC port");
    let active = network
        .active_host_session()
        .expect("active session")
        .expect("host is bound");
    assert_eq!(
        active.advertisement.protocol_version,
        SECURE_V4_ENDPOINT_VERSION
    );
    eprintln!(
        "=== v4 host bound at {}:{} (QUIC), advertising protocol version {} ===",
        endpoint.address, endpoint.control_port, active.advertisement.protocol_version
    );

    show_invitation_and_connect(&handle, &network, &identity, &serial, &temp);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    eprintln!("=== listener approved over v4; starting playback ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "v4-started");
    std::thread::sleep(Duration::from_secs(20));
    exercise_pause_resume(&handle, &network, "v4");
    std::thread::sleep(Duration::from_secs(25));
    print_diagnostics(&handle, &network, "v4-before-stop");
    assert_stream_reached_a_listener(&network, "v4");

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

/// Late join and reconnect over v4, with one QR scan. The host starts a
/// two-minute song before any listener exists; the phone joins mid-stream
/// from the signed QR and must pick up the stream already in progress. Then
/// the phone's Wi-Fi is turned off for longer than the QUIC idle timeout,
/// turned back on, and the app's own "Try again", "Connect manually" and
/// "Connect" rejoin
/// (the pinned host key is still held in memory); the host must approve
/// the same key-bound listener again and the phone must resume the stream.
/// Run as the test above, with
/// `manual_real_android_listener_late_joins_and_reconnects_over_secure_transport`.
#[test]
#[ignore = "reason: requires a real Android listener on the same LAN and a person to scan the QR; owner: secure transport v4 (remediation §1 item 2)"]
fn manual_real_android_listener_late_joins_and_reconnects_over_secure_transport() {
    let serial = std::env::var("SILENT_DISCO_DEVICE_TEST_ADB_SERIAL")
        .expect("set SILENT_DISCO_DEVICE_TEST_ADB_SERIAL to the phone's adb serial");
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        "song-v4-rejoin",
        &C_MAJOR_SCALE_HZ,
        1.0,
        150,
        AudioContainer::Wav,
    );
    let identity = fixed_identity_for_tests(22);
    let tls = identity.host_tls_identity().expect("host TLS identity");
    let (actor, handle, receiver, _advertisement, network, endpoint) =
        start_host_session_configured(descriptor, interface_name, interface_index, address, |n| {
            n.set_host_tls_identity(tls).expect("TLS identity");
            n.set_secure_transport(true).expect("secure transport on");
        });
    eprintln!(
        "=== v4 host bound at {}:{} (QUIC); starting playback before any listener ===",
        endpoint.address, endpoint.control_port
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "v4-playing-alone");

    show_invitation_and_connect(&handle, &network, &identity, &serial, &temp);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    eprintln!("=== late joiner approved mid-stream; listening for 15s ===");
    std::thread::sleep(Duration::from_secs(15));
    print_diagnostics(&handle, &network, "v4-late-joined");
    assert_stream_reached_a_listener(&network, "v4-late-joined");

    eprintln!("=== dropping the phone's Wi-Fi for 12s (QUIC idle timeout is 8s) ===");
    adb(&serial, &["shell", "svc", "wifi", "disable"]);
    std::thread::sleep(Duration::from_secs(12));
    let dropped = print_diagnostics(&handle, &network, "v4-wifi-off");
    eprintln!(
        "listeners while the phone is offline: {}",
        dropped.listeners.len()
    );
    adb(&serial, &["shell", "svc", "wifi", "enable"]);

    let status = Command::new("bash")
        .arg("-c")
        .arg(RECONNECT_SCRIPT)
        .arg("reconnect")
        .arg(&serial)
        .arg(endpoint.address.to_string())
        .status()
        .expect("run the reconnect automation script");
    assert!(
        status.success(),
        "reconnect automation failed; see its output above"
    );
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    eprintln!("=== rejoined over v4; listening for 20s ===");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, "v4-rejoined");

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

fn adb(serial: &str, args: &[&str]) {
    let status = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .args(args)
        .status()
        .expect("run adb");
    assert!(status.success(), "adb {args:?} failed");
}

/// After the phone's Wi-Fi is back: waits until it can reach the host
/// again, then uses the app's own recovery path: "Try again" on the
/// "Host disconnected" card (which returns to Nearby sessions), "Connect
/// manually", then "Connect" on the still-filled form.
const RECONNECT_SCRIPT: &str = r#"
set -euo pipefail
SERIAL="$1"; HOST="$2"
DUMP="$(mktemp)"
trap 'rm -f "$DUMP"' EXIT
dump_ui() {
  adb -s "$SERIAL" shell uiautomator dump /sdcard/window_dump.xml >/dev/null 2>&1 || return 1
  adb -s "$SERIAL" pull /sdcard/window_dump.xml "$DUMP" >/dev/null 2>&1 || return 1
}
tap_exact_text() {
  local label="$1" bounds
  bounds=$(grep -o "text=\"$1\"[^>]*bounds=\"\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]\"" "$DUMP" \
    | grep -o '\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]' | head -1) || true
  [ -n "$bounds" ] || return 1
  local nums; nums=$(echo "$bounds" | tr -c '0-9' ' ')
  set -- $nums
  echo "  [$SERIAL] tapping \"$label\" at $(( ($1 + $3) / 2 )),$(( ($2 + $4) / 2 ))"
  adb -s "$SERIAL" shell input tap $(( ($1 + $3) / 2 )) $(( ($2 + $4) / 2 ))
}
wait_tap() {
  local deadline=$((SECONDS + $2))
  while [ $SECONDS -lt $deadline ]; do
    if dump_ui && tap_exact_text "$1"; then sleep 1; return 0; fi
    # uiautomator lists only what is on screen; the status card can sit
    # below the fold of the scrolling form, so scroll down and look again.
    adb -s "$SERIAL" shell input swipe 540 1700 540 700 300
    sleep 1
  done
  echo "[$SERIAL] \"$1\" did not appear within $2s"; return 1
}

echo "[$SERIAL] waiting for the host to be reachable again"
deadline=$((SECONDS + 60))
until adb -s "$SERIAL" shell ping -c 1 -W 1 "$HOST" >/dev/null 2>&1; do
  [ $SECONDS -lt $deadline ] || { echo "[$SERIAL] host unreachable 60s after Wi-Fi came back"; exit 1; }
  sleep 1
done
echo "[$SERIAL] host reachable; recovering through the app"
# "Try again" returns to Nearby sessions; the manual form keeps the
# payload (and the app keeps the pinned host key), so reopen it and connect.
wait_tap "Try again" 30
wait_tap "Connect manually" 15
wait_tap "Connect" 15
echo "[$SERIAL] reconnect requested"
"#;

/// Builds the v4 host's signed invitation, shows it as a QR on this screen,
/// opens the phone's scanner and waits for a person to scan, then taps
/// through to the connection.
fn show_invitation_and_connect(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    network: &crate::platform::network::DesktopHostNetworkControl,
    identity: &crate::platform::invitation_identity::DesktopHostSigningIdentity,
    serial: &str,
    temp: &TempDir,
) {
    let active = network
        .active_host_session()
        .expect("active session")
        .expect("host is bound");
    let snapshot = handle.current_snapshot().expect("snapshot");
    let invitation = build_signed_invitation(
        &active.advertisement,
        active.endpoint,
        &snapshot.host_draft,
        identity,
        current_wall_clock_ms(),
    )
    .expect("signed v4 invitation");
    let payload_file = temp.path().join("invitation.txt");
    std::fs::write(&payload_file, &invitation.payload).expect("write invitation");
    let png = temp.path().join("invitation.png");
    let desktop_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("desktop directory");
    let status = Command::new("bash")
        .arg("-c")
        .arg(SCAN_AND_CONNECT_SCRIPT)
        .arg("scan_and_connect")
        .arg(serial)
        .arg(&payload_file)
        .arg(&png)
        .arg(desktop_dir)
        .status()
        .expect("run the QR display and phone automation script");
    assert!(
        status.success(),
        "QR display or phone automation failed; see its output above"
    );
}

/// Renders the invitation as a QR on this screen, opens the phone's QR
/// scanner, waits (up to 5 minutes, the invitation's lifetime) for a person
/// to scan it, then taps the verified dialog's Connect, which connects at
/// once; the form's Connect is tapped only if it is still offered. Text
/// lookups via `uiautomator`, not coordinates.
const SCAN_AND_CONNECT_SCRIPT: &str = r#"
set -euo pipefail
SERIAL="$1"; PAYLOAD_FILE="$2"; PNG="$3"; DESKTOP_DIR="$4"
DUMP="$(mktemp)"
trap 'rm -f "$DUMP"; kill "${VIEWER:-0}" 2>/dev/null || true' EXIT

dump_ui() {
  adb -s "$SERIAL" shell uiautomator dump /sdcard/window_dump.xml >/dev/null 2>&1 || return 1
  adb -s "$SERIAL" pull /sdcard/window_dump.xml "$DUMP" >/dev/null 2>&1 || return 1
}
tap_exact_text() {
  local label="$1" bounds
  bounds=$(grep -o "text=\"$1\"[^>]*bounds=\"\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]\"" "$DUMP" \
    | grep -o '\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]' | head -1) || true
  [ -n "$bounds" ] || return 1
  local nums; nums=$(echo "$bounds" | tr -c '0-9' ' ')
  set -- $nums
  echo "  [$SERIAL] tapping \"$label\" at $(( ($1 + $3) / 2 )),$(( ($2 + $4) / 2 ))"
  adb -s "$SERIAL" shell input tap $(( ($1 + $3) / 2 )) $(( ($2 + $4) / 2 ))
}
try_tap_any() {
  local deadline=$((SECONDS + ${WAIT:-10}))
  while [ $SECONDS -lt $deadline ]; do
    if dump_ui; then
      for target in "$@"; do
        if tap_exact_text "$target"; then sleep 1; return 0; fi
      done
    fi
    sleep 1
  done
  echo "  [$SERIAL] none of [$*] appeared; continuing"
  return 1
}

node -e "require(process.argv[1]).toFile(process.argv[2], require('fs').readFileSync(process.argv[3], 'utf8'), {width: 1000, margin: 4, errorCorrectionLevel: 'M'}, (e) => { if (e) { console.error(e); process.exit(1); } })" \
  "$DESKTOP_DIR/node_modules/qrcode" "$PNG" "$PAYLOAD_FILE"

echo "[$SERIAL] relaunching app and opening the QR scanner"
adb -s "$SERIAL" shell am force-stop com.ekkus.silentdisco
adb -s "$SERIAL" shell monkey -p com.ekkus.silentdisco -c android.intent.category.LAUNCHER 1 >/dev/null
sleep 3
# A verified invitation from an earlier run can be restored a few seconds
# after launch and cover the screen; dismiss it so this run scans a fresh one.
deadline=$((SECONDS + 10))
while [ $SECONDS -lt $deadline ]; do
  if dump_ui && grep -q 'text="Verified invitation"' "$DUMP"; then
    tap_exact_text "Cancel" || true
    sleep 1
    break
  fi
  sleep 1
done
try_tap_any "Find a session" || true
try_tap_any "Continue" || true
try_tap_any "While using the app" || true
try_tap_any "Allow" || true
try_tap_any "Scan QR" || { echo "[$SERIAL] Scan QR button not found"; exit 1; }
WAIT=5 try_tap_any "While using the app" "Only this time" "Allow" || true

# Show the QR only now that the phone's scanner is open, so the code
# appearing on screen is the signal to scan.
eog "$PNG" >/dev/null 2>&1 &
VIEWER=$!
echo ">>> SCAN THE QR NOW: point the phone at the QR code on the desktop screen <<<"
WAIT=300 try_tap_any "Connect" || { echo "[$SERIAL] no verified invitation within 5 minutes"; exit 1; }
kill "$VIEWER" 2>/dev/null || true
echo "[$SERIAL] verified invitation accepted"
# The dialog's Connect connects straight away (after any permission prompt);
# the form's own Connect appears only if it did not. The host-side wait for
# the join request is the real check, so this tap is best effort.
WAIT=8 try_tap_any "While using the app" "Allow" || true
WAIT=5 try_tap_any "Connect" || echo "  [$SERIAL] already connecting from the dialog"
echo "[$SERIAL] automation done"
"#;
