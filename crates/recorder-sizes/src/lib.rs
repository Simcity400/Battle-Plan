//! Offline recording size attribution: block sections, columns, compression levels and round-trip hour verification. Uses the recording format codecs directly. Command routing is owned by recorder; no live game is opened.
#[cfg(test)]
use recorder_test_support::test_support;
pub mod sizes;
