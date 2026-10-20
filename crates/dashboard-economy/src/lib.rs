//! Economy widget data: fuel by military branch, the international market
//! and lend-lease, lend-lease balances, production licences, industrial
//! organisations, equipment designs and convoy efficiency.
//!
//! One product crate above [dashboard-base](../dashboard-base/README.md),
//! laid out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md):
//! each family is prepared on its own and answers its prepared queries; the
//! dashboard re-exports the modules at `products::<name>`, registers them and
//! prepares and queries them in its pipeline.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod convoy_health;
    pub mod fuel_branches;
    pub mod keyed;
    pub mod lend_lease_balance;
    pub mod market;
    pub mod production_catalogue;
    #[cfg(test)]
    pub(crate) mod test_support;

    dashboard_base::reply_warnings!(
        convoy_health::ConvoyHealthReply,
        fuel_branches::FuelBranchesReply,
        lend_lease_balance::LendLeaseBalanceReply,
        market::MarketHistoryReply,
        market::MarketLedgerReply,
        production_catalogue::ProductionCatalogueReply,
    );
}
