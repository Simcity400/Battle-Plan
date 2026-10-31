//! Manpower: a country's recruitment, manpower pools and deployed manpower at an hour, from the
//! family its native walk prepares, with its election schedule and governments in exile.
//!
//! One product crate above `dashboard-base`: the module keeps its dashboard path
//! (`products::country_people`), names what it uses at the crates that own it (dashboard-base,
//! dashboard-series, dashboard-world); the dashboard re-exports it, registers it and prepares
//! and queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod country_people;
}
