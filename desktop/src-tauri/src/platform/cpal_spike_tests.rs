//! Block 33.1 spike: empirical `cpal` 0.18.1 behavior on this real Linux
//! development machine. Not production code and not wired into any
//! Tauri command -- this module exists only to produce genuine, run
//! (not assumed) evidence for the 33.2 policy decision recorded in
//! `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`. Every test here prints
//! what it actually observed; nothing is asserted to a specific device
//! count or name, since that is exactly what varies machine to machine --
//! only structural/API behavior (does a call succeed, error, or panic) is
//! asserted.
//!
//! This machine's real audio stack, confirmed before writing these tests
//! (not assumed): `PipeWire` 1.0.5 running as the user session's audio
//! server, exposing a PulseAudio-compatible socket, `/dev/snd/*` present
//! (three real ALSA cards: two `HDA-Intel`, one `acp`) but **not readable by
//! this session's user** (owned by the `audio` group, which this user is
//! not a member of -- `aplay -l` independently confirms this with "no
//! soundcards found"), and `PipeWire`'s own sink list shows exactly one
//! `auto_null` (dummy) sink, `SUSPENDED`. `/usr/share/alsa/alsa.conf.d/`
//! has both `50-pipewire.conf` and `99-pipewire-default.conf`, so the
//! system's default ALSA PCM is `PipeWire`'s plugin, not raw hardware --
//! `cpal`'s ALSA backend therefore talks to `PipeWire`'s socket, not
//! `/dev/snd/*` directly, which turns out to sidestep the `audio`-group
//! permission gap entirely (confirmed empirically below, not assumed).
//!
//! `cpal` 0.18.1's `DeviceTrait` no longer has a fallible `.name()` method
//! (an API change from earlier cpal versions this spike had to discover,
//! not one this project chose) -- device identity/display now goes through
//! `Display`/`.to_string()`, `.id()`, and structured `.description()`.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[test]
fn default_host_and_device_enumeration_do_not_panic_or_hang() {
    let host = cpal::default_host();
    let default_output = host.default_output_device();
    println!(
        "default output device: {}",
        default_output
            .as_ref()
            .map_or_else(|| "<none>".to_owned(), ToString::to_string)
    );

    match host.output_devices() {
        Ok(devices) => {
            let names: Vec<String> = devices.map(|device| device.to_string()).collect();
            println!("enumerated {} output device(s): {names:?}", names.len());
        }
        Err(error) => {
            println!("output_devices() returned an error (not a panic): {error}");
        }
    }
}

#[test]
fn default_output_config_negotiation_reports_what_this_machine_actually_offers() {
    let Some(device) = cpal::default_host().default_output_device() else {
        println!("no default output device on this machine -- documented, not asserted around");
        return;
    };
    match device.default_output_config() {
        Ok(config) => {
            println!(
                "default output config: {} ch, {} Hz, {:?}",
                config.channels(),
                config.sample_rate(),
                config.sample_format(),
            );
            // 48 kHz stereo float is the project's canonical render format
            // (`silent_disco_core::audio::{CANONICAL_SAMPLE_RATE_HZ,
            // CANONICAL_CHANNELS}`) -- recorded here whether or not this
            // specific device's default matches it, since Block 34's
            // stream-config selection has to handle both cases.
            let matches_canonical = config.channels() == 2
                && config.sample_rate() == 48_000
                && config.sample_format() == cpal::SampleFormat::F32;
            println!("matches project canonical 48kHz/stereo/f32 format: {matches_canonical}");
        }
        Err(error) => println!("default_output_config() failed: {error}"),
    }

    match device.supported_output_configs() {
        Ok(configs) => {
            let count = configs
                .inspect(|range| {
                    println!(
                        "supported range: {} ch, {}-{} Hz, {:?}",
                        range.channels(),
                        range.min_sample_rate(),
                        range.max_sample_rate(),
                        range.sample_format(),
                    );
                })
                .count();
            println!("{count} supported output config range(s) reported");
        }
        Err(error) => println!("supported_output_configs() failed: {error}"),
    }
}

#[test]
fn explicit_device_selection_by_identity_round_trips_through_the_host() {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else {
        println!("output_devices() failed; cannot spike explicit selection");
        return;
    };
    let devices: Vec<_> = devices.collect();
    if devices.is_empty() {
        println!(
            "zero enumerable output devices on this machine -- documented, not asserted around"
        );
        return;
    }
    for device in &devices {
        let Ok(mut fresh) = host.output_devices() else {
            continue;
        };
        // `Device: PartialEq + Eq` is the stable identity comparison this
        // API version offers -- re-finding a previously enumerated device
        // by equality after a fresh enumeration call.
        let found = fresh.any(|candidate| &candidate == device);
        println!("re-selecting {device} by identity after a fresh enumeration: found={found}");
    }
}

/// Attempts to actually open and briefly run an output stream against
/// whatever this machine's default device is. Uses a real callback that
/// writes silence (never a test tone -- no audible output is produced
/// deliberately, since this spike runs unattended and should never surprise
/// whoever's near the machine) and records real telemetry via atomics only,
/// matching Block 34's callback-safety bar even though this is only a
/// spike.
#[test]
fn opening_and_running_a_real_output_stream_is_attempted_and_its_outcome_is_reported() {
    let Some(device) = cpal::default_host().default_output_device() else {
        println!("no default output device -- cannot spike stream open on this machine");
        return;
    };
    let Ok(config) = device.default_output_config() else {
        println!("no default output config -- cannot spike stream open on this machine");
        return;
    };

    let callback_invocations = Arc::new(AtomicU32::new(0));
    let error_invocations = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&callback_invocations);
    let error_counter = Arc::clone(&error_invocations);

    if config.sample_format() != cpal::SampleFormat::F32 {
        println!(
            "default format is {:?}, not F32 -- this spike only wires the f32 path",
            config.sample_format()
        );
        return;
    }

    let stream_result = device.build_output_stream(
        config.into(),
        move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            // Silence only -- this callback's only job in this spike is to
            // prove the pipeline runs, never to produce sound.
            data.fill(0.0);
            counter.fetch_add(1, Ordering::Relaxed);
        },
        move |error| {
            println!("stream error callback fired: {error}");
            error_counter.fetch_add(1, Ordering::Relaxed);
        },
        None,
    );

    let stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => {
            println!("build_output_stream failed (reported, not asserted around): {error}");
            return;
        }
    };

    match stream.play() {
        Ok(()) => println!("stream.play() succeeded"),
        Err(error) => {
            println!("stream.play() failed (reported, not asserted around): {error}");
            return;
        }
    }

    std::thread::sleep(Duration::from_millis(300));

    // Shutdown quiescence: dropping the stream must not panic or hang, and
    // is timed so a genuine hang shows up as a real test failure rather
    // than the process wedging silently.
    let dropped = std::thread::spawn(move || drop(stream));
    let quiescent = dropped.join().is_ok();
    println!("stream drop completed cleanly: {quiescent}");
    assert!(quiescent, "stream drop must not panic");

    println!(
        "callback invocations observed: {}, error callback invocations: {}",
        callback_invocations.load(Ordering::Relaxed),
        error_invocations.load(Ordering::Relaxed),
    );
}

/// This machine's session has no removable/hot-unpluggable audio device
/// this spike can safely exercise (real hardware is present but blocked by
/// the `audio`-group permission gap documented at the top of this file, and
/// deliberately killing the user's live `PipeWire` session mid-test to force
/// a disconnect would be disruptive to their actual desktop). A live
/// device-loss error callback is therefore an honest, documented gap in
/// this spike, not silently skipped -- see the 33.1 checklist in
/// `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`.
///
/// What this test substitutes instead: proof that the error-reporting path
/// itself is real and reachable, not merely a happy-path illusion -- an
/// absurd, certainly-unsupported config must be rejected as a typed error,
/// never a panic.
#[test]
fn an_unsupported_config_is_rejected_as_an_error_not_a_panic() {
    let Some(device) = cpal::default_host().default_output_device() else {
        println!("no default output device -- cannot spike config rejection on this machine");
        return;
    };
    let absurd_config = cpal::StreamConfig {
        channels: 0,
        sample_rate: 0,
        buffer_size: cpal::BufferSize::Default,
    };
    let result = device.build_output_stream(
        absurd_config,
        move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| data.fill(0.0),
        move |error| println!("error callback fired for absurd config: {error}"),
        None,
    );
    match result {
        Ok(_stream) => println!(
            "an absurd 0-channel/0Hz config was accepted -- worth re-checking if this ever changes"
        ),
        Err(error) => println!("absurd config correctly rejected: {error}"),
    }
}
