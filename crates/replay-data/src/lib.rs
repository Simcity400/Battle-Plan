//! Shared recorded rows, cadence, exact values and their codecs.

pub mod cadence;
#[doc(hidden)]
pub mod checkpoint;
pub use replay_rows::{codec, day_night, display, event_feed_rows, fixed_point, naval_trees, temperature};
pub mod country_flags;
pub mod format;
pub mod identity;
pub mod intel_estimates;
#[doc(hidden)]
pub mod measurement;
pub mod military_aid;
/// Each table's descriptor ([`rows::RowOps`]), named as its table.
#[doc(hidden)]
pub mod ops {
    pub use replay_tables_a::*;
    pub use replay_tables_b::*;
    pub use replay_tables_c::*;
    pub use replay_tables_d::*;
    pub use replay_tables_e::*;
}
pub mod rows;
pub mod scope;
pub mod sliced;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use replay_rows::test_support;
#[doc(hidden)]
pub use replay_rows::value;
pub mod witness;

pub use replay_rows::{BranchId, FORMAT_VERSION, Hour, SessionId};

pub mod country_stability;
pub mod war_participation;
