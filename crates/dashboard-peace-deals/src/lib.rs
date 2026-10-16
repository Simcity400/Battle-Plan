//! Peace deals: captured finalizer actions, with a fallback to dated derived peace outcomes.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::peace_deals`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

pub mod products {
    pub mod peace_deals;

    dashboard_base::reply_warnings!(peace_deals::PeaceDealsReply,);
}
