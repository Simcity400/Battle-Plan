//! Place names: provinces' and states' names at hours, from the family their native walk
//! prepares.
//!
//! One product crate above `dashboard-base`: the module keeps its dashboard path
//! (`products::place_names`), names what it uses at the crates that own it (dashboard-base,
//! dashboard-series, dashboard-world); the dashboard re-exports it, registers it and prepares
//! and queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod place_names;
}
