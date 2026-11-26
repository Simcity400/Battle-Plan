dashboard_base::register_products! {
    ECONOMY_RESOURCE_MONTHS_RESOURCEMONTHS => crate::products::economy::resource_months::ResourceMonths,
    CONVOYS_CONVOYHEALTH => crate::products::convoys::ConvoyHealth,
    ECONOMY_MARKET_LEDGER_MARKETLEDGERTAIL => crate::products::economy::market::ledger::MarketLedgerTail,
    ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDS => crate::products::economy::fuel::awards::CountryFuelAwards,
    ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDHISTORY => crate::products::economy::fuel::awards::CountryFuelAwardHistory,
    ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMAND => crate::products::economy::fuel::demand::CountryFuelDemand,
    ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMANDHISTORY => crate::products::economy::fuel::demand::CountryFuelDemandHistory,
}

dashboard_base::register_hourly_products! {
    TRADE_FLOW_TRADEFLOW => crate::products::trade_flow::TradeFlow,
    ECONOMY_FUEL_BRANCHES_FUELBRANCHES => crate::products::economy::fuel_branches::FuelBranches,
    ECONOMY_RESOURCES_RESOURCEBALANCE => crate::products::economy::resources::ResourceBalance,
    ECONOMY_LEND_LEASE_BALANCE_LENDLEASEBALANCE => crate::products::economy::lend_lease_balance::LendLeaseBalance,
    ECONOMY_MARKET_MARKETLENDLEASE => crate::products::economy::market::MarketLendLease,
    ECONOMY_FUEL_COUNTRYFUEL => crate::products::economy::fuel::CountryFuel,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_FUELBRANCHES: ExactPanel = exact_panel::<crate::products::economy::fuel_branches::FuelBranches>;
pub const EXACT_LENDLEASEBALANCE: ExactPanel = exact_panel::<crate::products::economy::lend_lease_balance::LendLeaseBalance>;
pub const EXACT_MARKETLENDLEASE: ExactPanel = exact_panel::<crate::products::economy::market::MarketLendLease>;
