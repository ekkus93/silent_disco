use serde::Serialize;

pub mod bindings;
pub mod dto;
pub mod platform;
pub mod profile;

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

/// Runs the Silent Disco desktop shell.
///
/// # Errors
///
/// Returns a Tauri startup or event-loop error instead of converting it into a
/// successful process exit.
pub fn run() -> tauri::Result<()> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_core_smoke])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::core_smoke;

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
}
