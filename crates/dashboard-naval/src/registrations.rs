dashboard_base::register_products! {
    NAVAL_SUPREMACY_NAVALSUPREMACY => crate::products::naval_supremacy::NavalSupremacy,
    NAVAL_BATTLES_TAIL => crate::products::naval::battles::tail::NavalBattlesTail,
    NAVAL_BATTLES_NAVALBATTLEDETAILS => crate::products::naval::battles::NavalBattleDetails,
    NAVAL_SUNK_TAIL => crate::products::naval::sunk::tail::ShipsSunkTail,
    NAVAL_LOSSES_NAVALLOSSESAT => crate::products::naval::losses::NavalLossesAt,
}

dashboard_base::register_hourly_products! {
    MINEFIELDS_MINEFIELDS => crate::products::minefields::Minefields,
    NAVAL_YARDS_NAVALREPAIRS => crate::products::naval_yards::NavalRepairs,
    NAVAL_YARDS_NAVALREFITS => crate::products::naval_yards::NavalRefits,
    NAVAL_MANAGEMENT_NAVALMANAGEMENT => crate::products::naval_management::NavalManagement,
}
