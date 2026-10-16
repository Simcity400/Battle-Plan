//! Offline derivation checks against recorded clocks, temperature passes and attack/power-balance/naval tree witnesses, using the recording's verified content stack. Differences and incomplete comparisons fail. The end-to-end command-dispatch fixture stays with recorder.
use recorder_platform::directories;
pub mod derived;
