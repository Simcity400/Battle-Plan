//! The prepared store's shared pages: published coverage, binary series and
//! event batches, frame changes, group sums, the hours a family prepared on
//! its own keeps from a block, and progressive queries. The
//! dashboard's pipeline writes them; products read them.

pub mod coverage;
pub mod frame_changes;
pub mod group_sum;
pub mod kept;
pub mod pages;
#[cfg(any(test, feature = "test-support"))]
pub mod observer_tests;
pub mod prepass;
pub use job_priority as priority;
pub mod series;
mod store;
pub mod timing;

pub use store::{files, recording_store};

/// Query helpers shared by products whose reads join prepared pages with a
/// recorded tail.
pub mod query {
    pub mod frames;
    pub mod progressive;
}
