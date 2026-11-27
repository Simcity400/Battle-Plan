#[cfg(test)]
mod tests;

dashboard_base::register_products! {
    NAVAL_SUPREMACY_NAVALSUPREMACY => crate::products::naval_supremacy::NavalSupremacy; panel(crate::products::naval_supremacy::panel),
    NAVAL_BATTLES_TAIL => crate::products::naval::battles::tail::NavalBattlesTail; panel(crate::products::naval::battles::tail::dated_panel),
    NAVAL_BATTLES_COURSE => crate::products::naval::battles::tail::NavalBattleCourse,
    NAVAL_BATTLES_NAVALBATTLEDETAILS => crate::products::naval::battles::NavalBattleDetails,
    NAVAL_SUNK_TAIL => crate::products::naval::sunk::tail::ShipsSunkTail; panel(crate::products::naval::sunk::tail::dated_panel),
    NAVAL_LOSSES_NAVALLOSSESAT => crate::products::naval::losses::NavalLossesAt,

    @hourly
    MINEFIELDS_MINEFIELDS => exact<crate::products::minefields::Minefields>,
    NAVAL_YARDS_NAVALREPAIRS => exact<crate::products::naval_yards::NavalRepairs>,
    NAVAL_YARDS_NAVALREFITS => exact<crate::products::naval_yards::NavalRefits>,
    NAVAL_MANAGEMENT_NAVALMANAGEMENT => exact<crate::products::naval_management::NavalManagement>,
}
