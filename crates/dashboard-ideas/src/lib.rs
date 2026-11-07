//! Ideas: which ideas and advisors every country held, prepared from native recordings.
//!
//! One family crate above `dashboard-base`, which keeps the idea history product and its
//! content join: the family's projection, emit and readers name what they use at the crates
//! that own it (dashboard-base, dashboard-series, dashboard-text, dashboard-world); the
//! dashboard prepares the family and answers the idea history and the event feed from it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod ideas;
}
