//! `#[ignore]`d tests that drive a real external listener device/emulator
//! rather than the automated suite's in-process loopback listener. Run
//! explicitly with `-- --ignored --nocapture`; see each test's own doc
//! comment for the exact invocation.

mod automation;
mod melody;

use super::harness::{real_private_lan_address, start_host_session, submit, wait_snapshot_for};
use crate::platform::file_picker::{AudioContainer, SelectedSourceRegistry};
use crate::platform::start_playback;
use automation::{
    automate_manual_connect, connection_payload_json, exercise_pause_resume,
    print_connection_payload, print_diagnostics, wait_for_real_join_and_approve,
};
use melody::{C_MAJOR_SCALE_HZ, stage_melody_source};
use silent_disco_core::domain::PlaybackState;
use silent_disco_core::runtime::{AudioSourcePatch, CoreCommand, HostDraftPatch, InviteCodePatch};
use std::time::Duration;
use tempfile::TempDir;

/// Timeout for state-transition waits in manual device tests only. Real
/// devices/emulators are measurably slower and more congested than the
/// automated suite's in-process loopback listener -- see
/// [`wait_snapshot_for`] for the specific `queue_overflows=930` run that
/// motivated this.
const MANUAL_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Not part of the automated suite: binds a real desktop host on this
/// machine's real LAN address, prints a real connection payload, and waits
/// for an actual external listener (e.g. a phone on the same Wi-Fi network,
/// pasting the printed payload into the app's "Connect manually" screen) to
/// join before streaming a first long "song" (an ascending C major scale),
/// exercising a mid-song pause/resume, then switching mid-session to a
/// second, audibly distinct "song" (a descending scale) -- the same
/// stop -> update draft -> start sequence a real user changing tracks would
/// trigger, including a fresh stream ID for the second song. Covers Block
/// 28.1's full one-listener checklist for the WAV container. Run explicitly
/// with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_a_song_change -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_a_song_change() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor_a = stage_melody_source(
        &temp,
        &registry,
        "song-a",
        &C_MAJOR_SCALE_HZ,
        1.0,
        40,
        AudioContainer::Wav,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor_a, interface_name, interface_index, address);

    print_connection_payload(address, &endpoint, &advertisement);
    wait_for_real_join_and_approve(&handle, &receiver, &network);

    eprintln!(
        "=== song 1/2: \"song-a\", an ascending C major scale (do re mi fa so la ti do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "song-a-started");
    eprintln!("song-a playing for 15s...");
    std::thread::sleep(Duration::from_secs(15));

    exercise_pause_resume(&handle, &network, "song-a");

    eprintln!("song-a playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, "song-a-before-stop");

    eprintln!("=== switching songs: stopping song-a ===");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );

    let descending_scale: Vec<f64> = C_MAJOR_SCALE_HZ.iter().rev().copied().collect();
    let descriptor_b = stage_melody_source(
        &temp,
        &registry,
        "song-b",
        &descending_scale,
        1.0,
        40,
        AudioContainer::Wav,
    );
    let current = handle.current_snapshot().expect("current snapshot");
    submit(
        &handle,
        current.revision.get(),
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor_b.clone()),
            remember_approved_devices: None,
        }),
    );
    // `wait_snapshot`'s fast `TEST_TIMEOUT` (10s) is tuned for the loopback
    // suite; a real device's slower, more congested actor can take longer
    // than that to reflect a draft update, confirmed here (not assumed) by
    // a run against the LG G6 timing out at this exact call with the
    // update still pending -- `wait_snapshot_for`'s longer
    // `MANUAL_TEST_TIMEOUT` is what every other manual-test wait in this
    // file already uses.
    wait_snapshot_for(
        &handle,
        |snapshot| {
            snapshot
                .host_draft
                .audio_source
                .as_ref()
                .is_some_and(|source| source.source_id == descriptor_b.source_id)
        },
        MANUAL_TEST_TIMEOUT,
    );

    eprintln!(
        "=== song 2/2: \"song-b\", a descending C major scale (do ti la so fa mi re do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-b playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));
    print_diagnostics(&handle, &network, "song-b-before-stop");

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

/// FLAC variant of the Block 28.1 one-listener checklist (join, approve,
/// start, pause/resume, stop, diagnostics) -- a single melody rather than a
/// song change, since format decoding (not track-switching, already proven
/// by the WAV variant) is what this run exists to prove. Requires `ffmpeg`
/// on `PATH` to encode the fixture; see `encode_with_ffmpeg` in
/// `manual::melody` for why the desktop app itself cannot produce this file. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_flac -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_flac() {
    run_manual_single_format_session("flac-song", AudioContainer::Flac, "FLAC");
}

/// MP3 variant of the Block 28.1 one-listener checklist -- see
/// [`manual_real_android_listener_plays_flac`]. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_mp3 -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_mp3() {
    run_manual_single_format_session("mp3-song", AudioContainer::Mp3, "MP3");
}

fn run_manual_single_format_session(source_id: &str, container: AudioContainer, label: &str) {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        source_id,
        &C_MAJOR_SCALE_HZ,
        1.0,
        40,
        container,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    eprintln!("=== {label} manual device test ===");
    print_connection_payload(address, &endpoint, &advertisement);
    wait_for_real_join_and_approve(&handle, &receiver, &network);

    eprintln!("=== starting {label} playback: ascending C major scale ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, &format!("{label}-started"));
    eprintln!("playing for 15s...");
    std::thread::sleep(Duration::from_secs(15));

    exercise_pause_resume(&handle, &network, label);

    eprintln!("playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, &format!("{label}-before-stop"));

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("{label} done.");
}

/// Fully automated, no human required: two real Android emulators, driven
/// entirely via `adb`/`uiautomator` (see [`automation::automate_manual_connect`]), join
/// and are approved into the same desktop-hosted session and hear the same
/// stream. This is the project's actual success criterion -- listeners
/// hearing the same audio in sync, plural -- which has never been
/// exercised even against real hardware (see `memory.md`). An emulator is
/// not a substitute for the physical-device acceptance criteria Block 29
/// still needs, but this proves the desktop host's multi-listener path
/// (admission, broadcast fan-out, per-listener sync, delivery accounting)
/// against two genuinely independent OS processes/network stacks instead
/// of the automated suite's single in-process loopback listener.
///
/// Requires two already-running emulators with the app already installed,
/// reachable via the given `adb` serials, and `ffmpeg`-free (WAV only, to
/// keep the two-listener join timing predictable) -- override the serials
/// below if your local setup differs. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_two_emulator_listeners_play_together -- --ignored --nocapture`
#[test]
#[ignore = "requires two running Android emulators with the app installed, driven via adb"]
fn manual_two_emulator_listeners_play_together() {
    const FIRST_SERIAL: &str = "emulator-5554";
    const SECOND_SERIAL: &str = "emulator-5556";

    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        "two-listener-song",
        &C_MAJOR_SCALE_HZ,
        1.0,
        60,
        AudioContainer::Wav,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let payload = connection_payload_json(address, &endpoint, &advertisement);
    eprintln!("connection payload: {payload}");

    eprintln!("=== driving {FIRST_SERIAL} through Connect manually ===");
    automate_manual_connect(FIRST_SERIAL, &payload);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    let after_first = handle.current_snapshot().expect("current snapshot");
    eprintln!(
        "=== {FIRST_SERIAL} approved -- listeners now: {:?} ===",
        after_first
            .listeners
            .iter()
            .map(|l| l.device_id.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    eprintln!("=== driving {SECOND_SERIAL} through Connect manually ===");
    automate_manual_connect(SECOND_SERIAL, &payload);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    let after_second = handle.current_snapshot().expect("current snapshot");
    eprintln!(
        "=== {SECOND_SERIAL} approved -- listeners now: {:?} ===",
        after_second
            .listeners
            .iter()
            .map(|l| l.device_id.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    let ready = handle.current_snapshot().expect("current snapshot");
    assert_eq!(
        ready.listeners.len(),
        2,
        "expected exactly two connected listeners, got {}",
        ready.listeners.len()
    );

    eprintln!("=== starting playback for both listeners ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "two-listeners-started");
    eprintln!("playing for 20s...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, "two-listeners-mid");

    exercise_pause_resume(&handle, &network, "two-listeners");

    eprintln!("playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    let final_snapshot = print_diagnostics(&handle, &network, "two-listeners-before-stop");
    assert_eq!(
        final_snapshot.listeners.len(),
        2,
        "a listener silently dropped out during playback"
    );

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}
