dashboard_base::register_products! {
    NAVAL_SUPREMACY_NAVALSUPREMACY => crate::products::naval_supremacy::NavalSupremacy,
    NAVAL_BATTLES_NAVALBATTLESMANIFEST => crate::products::naval::battles::NavalBattlesManifest,
    NAVAL_BATTLES_NAVALBATTLES => crate::products::naval::battles::NavalBattles,
    NAVAL_BATTLES_NAVALBATTLESRANKING => crate::products::naval::battles::NavalBattlesRanking,
    NAVAL_BATTLES_NAVALBATTLEDETAILS => crate::products::naval::battles::NavalBattleDetails,
    NAVAL_SUNK_SHIPSSUNKMANIFEST => crate::products::naval::sunk::ShipsSunkManifest,
    NAVAL_SUNK_SHIPSSUNK => crate::products::naval::sunk::ShipsSunk,
    NAVAL_LOSSES_NAVALLOSSES => crate::products::naval::losses::NavalLosses,
}

dashboard_base::register_hourly_products! {
    MINEFIELDS_MINEFIELDS => crate::products::minefields::Minefields,
    NAVAL_YARDS_NAVALREPAIRS => crate::products::naval_yards::NavalRepairs,
    NAVAL_YARDS_NAVALREFITS => crate::products::naval_yards::NavalRefits,
    NAVAL_MANAGEMENT_NAVALMANAGEMENT => crate::products::naval_management::NavalManagement,
}
