//! Country projections over shared immutable replay sources, without map assets.
//!
//! The dashboard runtime shares sources and completed results, bounds queued
//! work and output bytes, and fairly advances exact-window and history scans.
//! Cancellation belongs to a subscriber and never publishes a partial result.
//! The legacy single-country facade uses the same scanner and retains its last
//! completed response for existing callers.

mod cache;
mod dto;
mod focus;
mod job;
mod land_owners;
mod projection;
mod query;
mod reduce;
mod runtime;
mod scan;
mod source;

pub use cache::QueryResultHandle;
pub use dto::*;
pub use focus::{FocusBranch, FocusCache, focus_branch, focus_history, focus_order};
pub use job::{CountryScan, ScanStep};
pub use land_owners::land_owners;
pub use query::*;
pub use runtime::{AnalyticsRuntime, QuerySubscription, RuntimeLimits, RuntimeStats};
pub use scan::Analytics;
pub use source::{SourceHandle, source_revision};

pub const MAX_POINTS: u32 = 1200;

#[cfg(test)]
mod tests;
