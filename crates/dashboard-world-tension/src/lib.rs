//! World tension: the world tension the game showed over a branch, each country's contribution to it and the events behind it.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::world_tension`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod world_tension;

    dashboard_base::reply_warnings!(world_tension::TensionReply,);
}
