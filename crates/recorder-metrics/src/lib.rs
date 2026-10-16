//! Exact inclusive recording-interval metrics, recorded failure accounting and native-nanosecond percentiles. The command parser and JSON schema live in metrics.rs. Command routing is owned by recorder; synthetic fixtures are dev-dependencies.
#[cfg(test)]
use recorder_test_support::test_support;
pub mod metrics;
