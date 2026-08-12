use serde::Serialize;
use tauri::Manager;

pub(crate) mod app_shutdown;
#[allow(
    clippy::too_many_lines,
    reason = "desktop startup keeps rollback ownership in one linear transaction"
)]
pub mod app_state;
pub mod bindings;
pub mod diagnostics_dto;
pub mod dto;
mod host_commands;
pub mod host_session_dto;
#[cfg(feature = "lab-mode")]
#[allow(
    dead_code,
    reason = "Block 42 wires most of this module (clock advance, node \
              lifecycle, the scenario runner, the recorder, and recording \
              capture/save) into lab_commands.rs's real Tauri command \
              surface, but deliberately leaves replay, recording load, and \
              the fault-injection transport wrapper unreached from any \
              command -- see lab_commands.rs's own module doc comment for \
              exactly what Block 42's UI does and does not expose; a later \
              Lab Mode block is the natural place to wire the rest"
)]
pub(crate) mod lab;
// Block 42: DTOs are always compiled (see this module's own doc comment
// for why) -- only `lab` and `lab_commands` stay behind `lab-mode`.
#[cfg(feature = "lab-mode")]
mod lab_commands;
pub mod lab_dto;
pub mod notification_buffer;
pub mod notification_channel;
pub mod platform;
pub mod profile;
pub mod runtime_dto;
pub mod shutdown;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreSmokeDto {
    major: u16,
    minor: u16,
    patch: u16,
    smoke: String,
}

fn core_smoke(input: u64) -> CoreSmokeDto {
    let version = silent_disco_core::core_version();
    CoreSmokeDto {
        major: version.major,
        minor: version.minor,
        patch: version.patch,
        smoke: silent_disco_core::deterministic_smoke(input).to_string(),
    }
}

#[tauri::command]
fn get_core_smoke(input: u64) -> CoreSmokeDto {
    core_smoke(input)
}

/// Reports whether this build was compiled with the `lab-mode` feature
/// (Block 37.1 "add frontend build flag derived from backend capability,
/// not only JavaScript environment"). Always compiled and always
/// registered, in every build -- a production release therefore always
/// answers `false`, verifiably, rather than merely omitting a frontend
/// dev-only code path a build misconfiguration could still enable.
#[tauri::command]
fn get_lab_mode_available() -> bool {
    cfg!(feature = "lab-mode")
}

/// Runs the Silent Disco desktop shell.
///
/// # Errors
///
/// Returns a Tauri startup or event-loop error instead of converting it into a
/// successful process exit.
pub fn run() -> tauri::Result<()> {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state::DesktopAppState::new())
        .manage(platform::file_picker::SelectedSourceRegistry::new())
        .manage(platform::source_staging_control::SourceStagingControl::new())
        .manage(app_shutdown::AppShutdownCoordinator::new());
    #[cfg(feature = "lab-mode")]
    let builder = builder.manage(lab_commands::LabAppState::new());

    builder
        .invoke_handler(tauri::generate_handler![
            get_core_smoke,
            get_lab_mode_available,
            app_state::open_profile,
            app_state::get_current_snapshot,
            app_state::get_storage_inspection,
            host_commands::select_host_role,
            host_commands::select_audio_source,
            host_commands::cancel_audio_source_staging,
            host_commands::update_host_draft,
            host_commands::create_host_session,
            host_commands::get_host_session_state,
            host_commands::approve_join_request,
            host_commands::reject_join_request,
            host_commands::remove_listener,
            host_commands::end_host_session,
            host_commands::start_host_playback,
            host_commands::pause_host_playback,
            host_commands::resume_host_playback,
            host_commands::stop_host_playback,
            host_commands::set_host_monitor_enabled,
            host_commands::get_host_diagnostics,
            host_commands::export_host_diagnostics,
            host_commands::get_host_network_state,
            host_commands::set_host_network_preference,
            host_commands::create_host_invitation,
            app_state::attach_notifications,
            app_state::close_profile,
            app_shutdown::get_app_shutdown_state,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_get_state,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_open_scenario_file,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_save_scenario_file,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_set_link_faults,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_run_loaded_scenario,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_pause_loaded_scenario,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_resume_loaded_scenario,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_advance_virtual_time,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_start_node,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_stop_node,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_stop_all_nodes,
            #[cfg(feature = "lab-mode")]
            lab_commands::lab_export_recording_file
        ])
        // Block 36.3 "close event initiates controlled shutdown": the
        // native close is always prevented first (buying time for a
        // bounded, non-blocking teardown attempt); `handle_close_requested`
        // then decides whether to actually exit, start that teardown, or
        // treat this as an idempotent duplicate request.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                app_shutdown::handle_close_requested(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::{core_smoke, get_lab_mode_available};

    #[test]
    fn smoke_dto_uses_real_shared_core() {
        let value = core_smoke(42);
        let version = silent_disco_core::core_version();

        assert_eq!(value.major, version.major);
        assert_eq!(value.minor, version.minor);
        assert_eq!(value.patch, version.patch);
        assert_eq!(
            value.smoke,
            silent_disco_core::deterministic_smoke(42).to_string()
        );
    }

    /// Block 37.3 "production build has no Lab entry": run without
    /// `--features lab-mode` (this crate's default, and the configuration
    /// used to build a real production release), the always-compiled
    /// availability command must answer `false` -- verifiable by any
    /// caller, not merely true because nothing links `lab.rs`.
    #[cfg(not(feature = "lab-mode"))]
    #[test]
    fn lab_mode_is_unavailable_in_a_production_build() {
        assert!(!get_lab_mode_available());
    }

    /// Block 37.3 "Lab build is labeled": the same command answers `true`
    /// only when this crate was actually compiled with `--features
    /// lab-mode` -- run explicitly (`cargo test --features lab-mode`) to
    /// exercise this branch, since it is never part of the default build.
    #[cfg(feature = "lab-mode")]
    #[test]
    fn lab_mode_is_available_in_a_lab_build() {
        assert!(get_lab_mode_available());
    }
}
