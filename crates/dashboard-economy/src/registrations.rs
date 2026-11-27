dashboard_base::register_products! {
    ECONOMY_RESOURCE_MONTHS_RESOURCEMONTHS => crate::products::economy::resource_months::ResourceMonths,
    CONVOYS_CONVOYHEALTH => crate::products::convoys::ConvoyHealth; panel(crate::products::convoys::panel),
    ECONOMY_MARKET_LEDGER_MARKETLEDGERTAIL => crate::products::economy::market::ledger::MarketLedgerTail; panel(crate::products::economy::market::ledger::dated_panel),
    ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDS => crate::products::economy::fuel::awards::CountryFuelAwards,
    ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDHISTORY => crate::products::economy::fuel::awards::CountryFuelAwardHistory,
    ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMAND => crate::products::economy::fuel::demand::CountryFuelDemand,
    ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMANDHISTORY => crate::products::economy::fuel::demand::CountryFuelDemandHistory,

    @hourly
    TRADE_FLOW_TRADEFLOW => exact<crate::products::trade_flow::TradeFlow>,
    ECONOMY_FUEL_BRANCHES_FUELBRANCHES => exact<crate::products::economy::fuel_branches::FuelBranches>,
    ECONOMY_RESOURCES_RESOURCEBALANCE => exact<crate::products::economy::resources::ResourceBalance>,
    ECONOMY_LEND_LEASE_BALANCE_LENDLEASEBALANCE => exact<crate::products::economy::lend_lease_balance::LendLeaseBalance>,
    ECONOMY_MARKET_MARKETLENDLEASE => exact<crate::products::economy::market::MarketLendLease>,
    ECONOMY_FUEL_COUNTRYFUEL => exact<crate::products::economy::fuel::CountryFuel>,
}
