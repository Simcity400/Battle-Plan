//! Country balances: every country's recorded political and command power, with their net drops.
//!
//! One product crate above `dashboard-base`: the family its native walk prepares
//! (`products::country_balances`), named at the crates that own what it uses (dashboard-base,
//! dashboard-series, dashboard-world); the dashboard binds, prepares and queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod country_balances;
}
