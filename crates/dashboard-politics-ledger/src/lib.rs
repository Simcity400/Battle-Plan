//! The politics ledger: political power, command power and their spending by country, with the ideas, advisors and laws behind each line.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::politics_ledger`) and names what it
//! uses at the crates that own it (dashboard-base, dashboard-text and
//! dashboard-core); the dashboard re-exports it, registers it and prepares and
//! queries it.

pub mod products {
    pub mod politics_ledger;

    dashboard_base::reply_warnings!(politics_ledger::PoliticsLedgerReply,);
}
