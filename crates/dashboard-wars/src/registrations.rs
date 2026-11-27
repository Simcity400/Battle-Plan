use dashboard_base::products::at_hour::at_hour;

dashboard_base::register_products! {
    DIPLOMACY_DIPLOMATICPACTS => crate::products::diplomacy::DiplomaticPacts,
    CASUALTIES_CASUALTYBREAKDOWN => crate::products::casualties::CasualtyBreakdown; interval(at_hour, |r| r.through),
    PEACE_DEALS_TAIL => crate::products::peace::tail::PeaceDealsTail; panel(crate::products::peace::tail::dated_panel),
    TERRITORY_TRANSFERS_TERRITORYTRANSFERS => crate::products::territory_transfers::TerritoryTransfers,
    OCCUPATION_RESERVE_RESERVEHISTORY => crate::products::occupation_reserve::ReserveHistory,
    FACTION_STATE_HISTORY_FACTIONSTATE => crate::products::faction_state::history::FactionState,
    WARS_WARS => crate::products::wars::Wars,
    WARS_WARTIMELINEAT => crate::products::wars::WarTimelineAt,
    SCOREBOARD_SCOREBOARDCOLUMNS => crate::products::scoreboard::ScoreboardColumns,
    SCOREBOARD_SCOREBOARD => crate::products::scoreboard::Scoreboard,

    @hourly
    FACTIONS_FACTIONTOTALSAT => exact<crate::products::factions::FactionTotalsAt>,
    DIPLOMACY_DIPLOMATICRELATIONS => exact<crate::products::diplomacy::DiplomaticRelations>,
    VOLUNTEERS_VOLUNTEERFLOW => exact<crate::products::volunteers::VolunteerFlow>,
    CASUALTY_MATRIX_CASUALTYMATRIX => exact<crate::products::casualty_matrix::CasualtyMatrix>,
    WARGOALS_WARGOALS => exact<crate::products::wargoals::Wargoals>,
    OCCUPATION_RESERVE_OCCUPATIONRESERVE => exact<crate::products::occupation_reserve::OccupationReserve>,
    OCCUPATION_STATES_OCCUPIEDSTATES => exact<crate::products::occupation::states::OccupiedStates>,
    OCCUPATION_DETAIL_OCCUPATIONDETAIL => exact<crate::products::occupation::detail::OccupationDetail>,
    FACTION_STATE_COUNTRY_GOALS_COUNTRYFACTIONGOALS => exact<crate::products::faction_state::country_goals::CountryFactionGoals>,
    FACTION_STATE_DETAILS_FACTIONDETAILS => exact<crate::products::faction_state::details::FactionDetails>,
    FACTION_STATE_POOLS_FACTIONPOOLS => exact<crate::products::faction_state::pools::FactionPools>,
    WARS_CONTRIBUTIONS_WARCONTRIBUTIONS => exact<crate::products::wars::contributions::WarContributions>,
}
