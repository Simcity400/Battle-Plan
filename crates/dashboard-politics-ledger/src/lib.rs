//! The politics ledger: political power, command power and their spending by country, with the ideas, advisors and laws behind each line.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! module keeps its dashboard path (`products::politics_ledger`), names the shared
//! layer through the same `crate::` paths it used in the dashboard, and the
//! dashboard re-exports it, registers it and prepares and queries it.

// The shared layer at the paths the product names.
pub use dashboard_base::*;

pub mod products {
    pub use dashboard_base::products::*;

    pub mod politics_ledger;

    dashboard_base::reply_warnings!(politics_ledger::PoliticsLedgerReply,);
}
