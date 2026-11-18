//! Intelligence products over recorded agency, cipher and operative state.

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

use dashboard_base::{Source, queries as answer};
use dashboard_text::metadata;

pub mod products {
    pub use dashboard_base::products::*;
    pub use dashboard_base::products::log_page as logs;
    pub use dashboard_base::queries::country_ids;
    pub mod intelligence;
}

pub mod registrations;
