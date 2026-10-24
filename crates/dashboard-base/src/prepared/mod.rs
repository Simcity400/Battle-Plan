//! The prepared store's shared pages: published coverage, binary series and
//! event batches, frame changes, group sums, the hours a family prepared on
//! its own keeps from a block, progressive queries and replies answered from
//! a prepared prefix. The dashboard's pipeline writes them; products read them.

pub mod coverage;
pub mod frame_changes;
pub mod group_sum;
pub mod kept;
#[cfg(any(test, feature = "test-support"))]
pub mod observer_tests;
pub mod pages;
pub mod prefix;
pub mod prepass;
pub use job_priority as priority;
pub mod series;
pub mod state_census;
mod store;
pub mod timing;

pub use store::{files, recording_identity, recording_store, refresh_saved_binding};

/// Query helpers shared by products whose reads join prepared pages with a
/// recorded tail.
pub mod query {
    pub mod earlier;
    pub mod frames;
    pub mod keyframes;
    pub mod observations;
    pub mod progressive;
}
