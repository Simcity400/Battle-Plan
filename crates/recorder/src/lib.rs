//! Passive recording, finalization and offline inspection. The `recorder`
//! binary is the command line; [`controller`] is the small API the tray
//! recorder drives in-process.
#[cfg(test)]
mod command_tests;
pub mod controller;
#[cfg(test)]
mod derived_command_tests;
use recorder_derived::derived;
use recorder_metrics::metrics;
use recorder_movement_oracle::movement_oracle;
use recorder_oracle::oracle;
use recorder_platform::directories;
pub mod recording;
mod ship_name_witness;
use recorder_platform::windows;
use recorder_replay::replay_io;
use recorder_sizes::sizes;
#[cfg(test)]
use recorder_test_support::test_support;
