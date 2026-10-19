//! Resource balance and trade flows: one country's strategic resources over a branch, and who trades what with whom.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md): the
//! modules keep their dashboard paths (`products::resource_balance`,
//! `products::trade_flows`) and name what they use at the crates that own it
//! (dashboard-base, dashboard-text and dashboard-core); the dashboard
//! re-exports them, registers them and prepares and queries them.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod market_lendlease;
    pub mod resource_balance;
    pub mod trade_flows;

    dashboard_base::reply_warnings!(
        resource_balance::ResourceBalanceReply,
        trade_flows::TradeFlowsReply,
        market_lendlease::MarketLendLeaseReply,
    );
}
