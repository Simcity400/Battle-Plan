dashboard_base::register_products! {
    DIPLOMACY_DIPLOMATICPACTS => crate::products::diplomacy::DiplomaticPacts,
    CASUALTIES_CASUALTYBREAKDOWN => crate::products::casualties::CasualtyBreakdown,
    PEACE_PEACEDEALS => crate::products::peace::PeaceDeals,
    TERRITORY_TRANSFERS_TERRITORYTRANSFERS => crate::products::territory_transfers::TerritoryTransfers,
    OCCUPATION_RESERVE_RESERVEHISTORY => crate::products::occupation_reserve::ReserveHistory,
    FACTION_STATE_HISTORY_FACTIONSTATE => crate::products::faction_state::history::FactionState,
    WARS_WARS => crate::products::wars::Wars,
    WARS_WARTIMELINEAT => crate::products::wars::WarTimelineAt,
    SCOREBOARD_SCOREBOARDCOLUMNS => crate::products::scoreboard::ScoreboardColumns,
    SCOREBOARD_SCOREBOARD => crate::products::scoreboard::Scoreboard,
}

dashboard_base::register_hourly_products! {
    FACTIONS_FACTIONTOTALSAT => crate::products::factions::FactionTotalsAt,
    DIPLOMACY_DIPLOMATICRELATIONS => crate::products::diplomacy::DiplomaticRelations,
    VOLUNTEERS_VOLUNTEERFLOW => crate::products::volunteers::VolunteerFlow,
    CASUALTY_MATRIX_CASUALTYMATRIX => crate::products::casualty_matrix::CasualtyMatrix,
    WARGOALS_WARGOALS => crate::products::wargoals::Wargoals,
    OCCUPATION_RESERVE_OCCUPATIONRESERVE => crate::products::occupation_reserve::OccupationReserve,
    OCCUPATION_STATES_OCCUPIEDSTATES => crate::products::occupation::states::OccupiedStates,
    OCCUPATION_DETAIL_OCCUPATIONDETAIL => crate::products::occupation::detail::OccupationDetail,
    FACTION_STATE_COUNTRY_GOALS_COUNTRYFACTIONGOALS => crate::products::faction_state::country_goals::CountryFactionGoals,
    FACTION_STATE_DETAILS_FACTIONDETAILS => crate::products::faction_state::details::FactionDetails,
    FACTION_STATE_POOLS_FACTIONPOOLS => crate::products::faction_state::pools::FactionPools,
    WARS_CONTRIBUTIONS_WARCONTRIBUTIONS => crate::products::wars::contributions::WarContributions,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_OCCUPATIONRESERVE: ExactPanel = exact_panel::<crate::products::occupation_reserve::OccupationReserve>;
pub const EXACT_OCCUPIEDSTATES: ExactPanel = exact_panel::<crate::products::occupation::states::OccupiedStates>;
pub const EXACT_FACTIONTOTALSAT: ExactPanel = exact_panel::<crate::products::factions::FactionTotalsAt>;
pub const EXACT_FACTIONPOOLS: ExactPanel = exact_panel::<crate::products::faction_state::pools::FactionPools>;
pub const EXACT_DIPLOMATICRELATIONS: ExactPanel = exact_panel::<crate::products::diplomacy::DiplomaticRelations>;
pub const EXACT_WARCONTRIBUTIONS: ExactPanel = exact_panel::<crate::products::wars::contributions::WarContributions>;
