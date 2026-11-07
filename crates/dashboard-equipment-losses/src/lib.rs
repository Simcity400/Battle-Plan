//! Equipment losses: one country's equipment losses over a branch, by cause, equipment type and design.
//!
//! One product crate above `dashboard-base`: the
//! module keeps its dashboard path (`products::equipment_losses`) and names
//! what it uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod products {
    pub mod equipment_losses;

    dashboard_base::reply_warnings!(equipment_losses::LossesReply,);
}
