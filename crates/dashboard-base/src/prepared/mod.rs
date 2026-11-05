//! The prepared store's shared pages: published coverage and the gates decoded
//! from it, each family's publication roots and ledgers, binary series and event batches, frame
//! changes, group sums, the hours a family prepared on its own keeps from a
//! block, progressive queries, replies answered from a prepared prefix, and the replies a family
//! answers from its series-engine publication. The dashboard's pipeline writes them; products
//! read them.

pub mod codec;
pub mod coverage;
pub mod engine;
pub mod entries;
pub mod facts;
pub mod frame_changes;
pub mod gates;
pub mod group_sum;
pub mod kept;
#[cfg(any(test, feature = "test-support"))]
pub mod observer_tests;
pub mod pages;
pub mod prefix;
pub mod prepass;
pub mod published;
pub use job_priority as priority;
pub mod roots;
pub mod series;
pub mod series_read;
pub mod served;
pub mod state_census;
mod store;

pub use store::{
    SettingsStamp, files, rebind_settings, recording_folder, recording_identity, recording_store,
    refresh_saved_binding, settings_stamp,
};

/// Query helpers shared by products whose reads join prepared pages with a
/// recorded tail.
pub mod query {
    pub mod earlier;
    pub mod frames;
    pub mod keyframes;
    pub mod native;
    pub mod observations;
    pub mod progressive;
}
