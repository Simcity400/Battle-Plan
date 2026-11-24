dashboard_base::register_products! {
    FACTIONS_FACTIONLINES => crate::products::factions::FactionLines,
    VOLUNTEERS_VOLUNTEERSEXPEDITIONARIES => crate::products::volunteers::VolunteersExpeditionaries,
    DIPLOMACY_DIPLOMATICPACTS => crate::products::diplomacy::DiplomaticPacts,
    CASUALTIES_CASUALTYPAIRS => crate::products::casualties::CasualtyPairs,
    CASUALTIES_CASUALTYBREAKDOWN => crate::products::casualties::CasualtyBreakdown,
    CASUALTIES_CASUALTYLABELS => crate::products::casualties::CasualtyLabels,
    PEACE_PEACEDEALS => crate::products::peace::PeaceDeals,
    TERRITORY_TRANSFERS_TERRITORYTRANSFERS => crate::products::territory_transfers::TerritoryTransfers,
    OCCUPATION_RESERVE_RESERVEHISTORY => crate::products::occupation_reserve::ReserveHistory,
    FACTION_STATE_HISTORY_FACTIONSTATE => crate::products::faction_state::history::FactionState,
    WARS_WARS => crate::products::wars::Wars,
    WARS_WARLOSSES => crate::products::wars::WarLosses,
    SCOREBOARD_SCOREBOARDCOLUMNS => crate::products::scoreboard::ScoreboardColumns,
    SCOREBOARD_SCOREBOARD => crate::products::scoreboard::Scoreboard,
}

dashboard_base::register_hourly_products! {
    FACTIONS_FACTIONROSTERS => crate::products::factions::FactionRosters,
    DIPLOMACY_DIPLOMATICRELATIONS => crate::products::diplomacy::DiplomaticRelations,
    CASUALTY_MATRIX_CASUALTYMATRIX => crate::products::casualty_matrix::CasualtyMatrix,
    WARGOALS_WARGOALS => crate::products::wargoals::Wargoals,
    OCCUPATION_RESERVE_OCCUPATIONRESERVE => crate::products::occupation_reserve::OccupationReserve,
    OCCUPATION_STATES_OCCUPIEDSTATES => crate::products::occupation::states::OccupiedStates,
    OCCUPATION_DETAIL_OCCUPATIONDETAIL => crate::products::occupation::detail::OccupationDetail,
    FACTION_STATE_COUNTRY_GOALS_COUNTRYFACTIONGOALS => crate::products::faction_state::country_goals::CountryFactionGoals,
    FACTION_STATE_DETAILS_FACTIONDETAILS => crate::products::faction_state::details::FactionDetails,
    WARS_CONTRIBUTIONS_WARCONTRIBUTIONS => crate::products::wars::contributions::WarContributions,
    FACTION_STATE_REWARDS_FACTIONGOALREWARDS => crate::products::faction_state::rewards::FactionGoalRewards,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_OCCUPATIONRESERVE: ExactPanel = exact_panel::<crate::products::occupation_reserve::OccupationReserve>;
pub const EXACT_OCCUPIEDSTATES: ExactPanel = exact_panel::<crate::products::occupation::states::OccupiedStates>;
