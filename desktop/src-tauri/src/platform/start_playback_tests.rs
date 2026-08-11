//! Automated and manual coverage for `start_playback` orchestration, split
//! by concern:
//! - [`fixtures`]: staged WAV source generation shared by the automated
//!   suite.
//! - [`harness`]: real actor/transport driving helpers shared by every
//!   test, automated and manual alike.
//! - [`streaming_tests`]: audio/sync delivery and pacing coverage.
//! - [`lifecycle_tests`]: playback state-machine, duplicate-command, and
//!   failure-reporting coverage.
//! - [`manual`]: `#[ignore]`d tests that drive a real external listener
//!   device/emulator, plus their exclusive melody fixtures and `adb`
//!   automation.

mod fixtures;
mod harness;
mod lifecycle_tests;
mod manual;
mod streaming_tests;
