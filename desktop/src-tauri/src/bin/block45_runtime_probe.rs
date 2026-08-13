//! Block 45 desktop-runtime performance evidence entry point.

use serde::Serialize;
use silent_disco_desktop_lib::platform::performance_probe::{
    DesktopRuntimeMetric, measure_desktop_runtime,
};
use std::error::Error;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProbeReport {
    schema_version: u32,
    desktop_runtime: DesktopRuntimeMetric,
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let report = RuntimeProbeReport {
        schema_version: 1,
        desktop_runtime: measure_desktop_runtime()?,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
