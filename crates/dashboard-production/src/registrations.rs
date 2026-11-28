use dashboard_base::products::at_hour::at_hour;

dashboard_base::register_products! {
    PRODUCTION_PRODUCTIONLINEBREAKDOWN => crate::products::production::ProductionLineBreakdown,
    PRODUCTION_SPENDING_PRODUCTIONSPENDING => crate::products::production_spending::ProductionSpending; interval(at_hour, |r| r.through),
    PRODUCTION_SPENDING_MONTHS_PRODUCTIONSPENDINGMONTHS => crate::products::production_spending_months::ProductionSpendingMonths; panel(crate::products::production_spending_months::panel),
    CONSTRUCTION_TAIL_CONSTRUCTIONLOGTAIL => crate::products::construction::tail::ConstructionLogTail; panel(crate::products::construction::tail::dated_panel),
    CONSTRUCTION_BREAKDOWN_CONSTRUCTIONBREAKDOWN => crate::products::construction::breakdown::ConstructionBreakdown; interval(at_hour, |r| r.through),
    STOCKPILE_EQUIPMENTSTOCKPILETYPES => crate::products::stockpile::EquipmentStockpileTypes; panel(crate::products::stockpile::types::panel),
    STOCKPILE_EQUIPMENTSTOCKPILELINES => crate::products::stockpile::EquipmentStockpileLines; interval(at_hour, |r| r.hour),
    EQUIPMENT_LOSSES_EQUIPMENTLOSSES => crate::products::equipment_losses::EquipmentLosses; interval(at_hour, |r| r.through),
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDTYPES => crate::products::equipment_produced::EquipmentProducedTypes,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDAT => crate::products::equipment_produced::EquipmentProducedAt,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDRACE => crate::products::equipment_produced::EquipmentProducedRace; panel(crate::products::equipment_produced::race_panel),

    @hourly
    PRODUCTION_PRODUCTIONLINESAT => exact<crate::products::production::ProductionLinesAt>,
    PRODUCTION_PRODUCTIONLICENCES => exact<crate::products::production::ProductionLicences>,
    PRODUCTION_INDUSTRIALORGANISATIONS => exact<crate::products::production::IndustrialOrganisations>,
    PRODUCTION_EQUIPMENTDESIGNS => exact<crate::products::production::EquipmentDesigns>,
    STOCKPILE_STOCKPILEARTWORK => exact<crate::products::stockpile::StockpileArtwork>,
}
