dashboard_base::register_products! {
    NAVAL_SUPREMACY_NAVALSUPREMACY => crate::products::naval_supremacy::NavalSupremacy; day(DayThroughHour, "firstHour", "lastHour", |r| r.valid_through),
    NAVAL_BATTLES_TAIL => crate::products::naval::battles::tail::NavalBattlesTail; LogTail(crate::products::naval::battles::tail::dated_panel),
    NAVAL_BATTLES_NAVALBATTLEDETAILS => crate::products::naval::battles::NavalBattleDetails,
    NAVAL_SUNK_TAIL => crate::products::naval::sunk::tail::ShipsSunkTail; LogTail(crate::products::naval::sunk::tail::dated_panel),
    NAVAL_LOSSES_NAVALLOSSESAT => crate::products::naval::losses::NavalLossesAt,

    @hourly
    MINEFIELDS_MINEFIELDS => whole_frame<crate::products::minefields::Minefields>,
    NAVAL_YARDS_NAVALREPAIRS => whole_frame<crate::products::naval_yards::NavalRepairs>,
    NAVAL_YARDS_NAVALREFITS => whole_frame<crate::products::naval_yards::NavalRefits>,
    NAVAL_MANAGEMENT_NAVALMANAGEMENT => whole_frame<crate::products::naval_management::NavalManagement>,
}
