//! Replay recovery, admission records, finalization, append and inspection. replay_io.rs owns load refusal/omission values and atomic JSON publication shared with capture. Recovery validates the committed prefix and terminal evidence; tray sessions publish only admitted loads. Capture lifecycle is owned by recorder.
use recorder_metrics::metrics;
use recorder_platform::windows;
#[cfg(test)]
use recorder_test_support::test_support;
pub mod replay_io;
