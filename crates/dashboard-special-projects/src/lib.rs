//! Special projects: each country's project facilities, progress, prototypes and completions, as the game reports them.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::special_projects`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

pub mod products {
    pub mod special_projects;

    dashboard_base::reply_warnings!(special_projects::SpecialProjectsReply,);
}
