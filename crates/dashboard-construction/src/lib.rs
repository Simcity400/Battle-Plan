//! Construction: every construction project in a branch and the civilian factory output spent on it.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::construction_log`,
//! `products::construction_spending`) and name what they use at the crates that
//! own it (dashboard-base, dashboard-text and dashboard-core); the dashboard
//! re-exports them, registers them and prepares and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod construction_log;
    pub mod construction_spending;

    dashboard_base::reply_warnings!(
        construction_log::ConstructionLogReply,
        construction_log::ConstructionLogManifestReply,
        construction_spending::ConstructionSpendingReply,
    );
}
