pub use dashboard_base::products::at_hour::*;
#[cfg(any(test, feature = "export-types"))]
pub(super) use dashboard_base::products::at_hour::entry_for_export as local_entry;
#[cfg(not(any(test, feature = "export-types")))]
pub(super) use dashboard_base::products::at_hour::entry as local_entry;

use crate::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

pub fn panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    let kind = asked.get("panel")?.as_str()?;
    let entry = super::AT_HOUR.iter().find(|entry| entry.registered.key == kind)?;
    Some((entry.panel)(source, load, asked))
}


#[cfg(test)]
mod registry_tests;

pub fn dated_panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    use super::{doctrine_mastery::DoctrineMastery, economy::lend_lease_balance::LendLeaseBalance, intelligence::agencies::AgencyComparison, occupation_reserve::OccupationReserve, production::ProductionLinesAt, research::slots::ResearchSlots, scalars::GameScore, special_projects, territory::TerritoryShare};
    Some(match asked["panel"].as_str()? {
        "special-projects" => special_projects::dated_panel(source, load, asked),
        "special-project-details" => exact_panel::<special_projects::details::SpecialProjectDetailsAt>(source, load, asked),
        "doctrine-mastery" => exact_panel::<DoctrineMastery>(source, load, asked),
        "research-slots" => exact_panel::<ResearchSlots>(source, load, asked),
        "nuclear-arsenal" => exact_panel::<super::scalars::NuclearArsenal>(source, load, asked),
        "game-score" => exact_panel::<GameScore>(source, load, asked),
        "production-lines-at" => exact_panel::<ProductionLinesAt>(source, load, asked),
        "agency-comparison" => exact_panel::<AgencyComparison>(source, load, asked),
        "territory-share" => exact_panel::<TerritoryShare>(source, load, asked),
        "occupation-reserve-manpower" => exact_panel::<OccupationReserve>(source, load, asked),
        "lend-lease-balance" => exact_panel::<LendLeaseBalance>(source, load, asked),
        "occupied-states" => exact_panel::<super::occupation::states::OccupiedStates>(source, load, asked),
        _ => return None,
    })
}
