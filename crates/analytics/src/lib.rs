//! Country overview queries over the shared replay decoder, without map assets.
//!
//! Histories are reduced during one sequential branch scan to at most 1200
//! envelopes. Only the last completed response is cached. Cancellation never
//! publishes a partial result or replaces the completed cache.

mod dto;
mod reduce;
mod scan;

pub use dto::*;
pub use scan::Analytics;

pub const MAX_POINTS: u32 = 1200;

#[cfg(test)]
mod tests;
