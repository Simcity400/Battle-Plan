//! Party history: one country's politics over a branch, each party's popularity, which party ruled, and when elections were held.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::party_history`), names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

pub mod products {
    pub mod party_history;

    dashboard_base::reply_warnings!(party_history::PartyHistoryReply,);
}
