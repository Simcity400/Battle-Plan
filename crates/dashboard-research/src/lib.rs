//! Research-history products over the recording's research slots and assignments.

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

use dashboard_base::{Source, queries as answer};
use dashboard_core::dto::Artwork;
use dashboard_text::metadata;
#[cfg(test)]
pub use dashboard_base::test_fixtures;

pub mod products {
    pub use dashboard_base::products::*;
    pub mod research;
}

pub mod registrations;
