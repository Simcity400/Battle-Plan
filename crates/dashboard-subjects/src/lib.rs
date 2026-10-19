//! Subjects and autonomy: who was whose subject over a branch, and each subject's autonomy level, overlord, score and thresholds.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::subjects`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod autonomy;
    pub mod subjects;

    dashboard_base::reply_warnings!(subjects::SubjectReply, autonomy::AutonomyReply,);
}
