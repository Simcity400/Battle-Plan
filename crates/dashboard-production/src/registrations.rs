dashboard_base::register_products! {
    PRODUCTION_PRODUCTIONLINES => crate::products::production::ProductionLines,
    PRODUCTION_PRODUCTIONLINEDETAIL => crate::products::production::ProductionLineDetail,
    PRODUCTION_PRODUCTIONLINEBREAKDOWN => crate::products::production::ProductionLineBreakdown,
    PRODUCTION_SPENDING_PRODUCTIONSPENDING => crate::products::production_spending::ProductionSpending,
    CONSTRUCTION_LOG_CONSTRUCTIONLOGMANIFEST => crate::products::construction::log::ConstructionLogManifest,
    CONSTRUCTION_LOG_CONSTRUCTIONLOG => crate::products::construction::log::ConstructionLog,
    CONSTRUCTION_SPENDING_CONSTRUCTIONSPENDING => crate::products::construction::spending::ConstructionSpending,
    CONSTRUCTION_BREAKDOWN_CONSTRUCTIONBREAKDOWN => crate::products::construction::breakdown::ConstructionBreakdown,
    STOCKPILE_EQUIPMENTSTOCKPILECATALOGUE => crate::products::stockpile::EquipmentStockpileCatalogue,
    STOCKPILE_EQUIPMENTSTOCKPILECHART => crate::products::stockpile::EquipmentStockpileChart,
    EQUIPMENT_LOSSES_EQUIPMENTLOSSES => crate::products::equipment_losses::EquipmentLosses,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCED => crate::products::equipment_produced::EquipmentProduced,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDTYPES => crate::products::equipment_produced::EquipmentProducedTypes,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDCOUNTRY => crate::products::equipment_produced::EquipmentProducedCountry,
    EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDAT => crate::products::equipment_produced::EquipmentProducedAt,
}

dashboard_base::register_hourly_products! {
    PRODUCTION_PRODUCTIONLINESAT => crate::products::production::ProductionLinesAt,
    PRODUCTION_PRODUCTIONLICENCES => crate::products::production::ProductionLicences,
    PRODUCTION_INDUSTRIALORGANISATIONS => crate::products::production::IndustrialOrganisations,
    PRODUCTION_EQUIPMENTDESIGNS => crate::products::production::EquipmentDesigns,
    STOCKPILE_EQUIPMENTSTOCKPILEAT => crate::products::stockpile::EquipmentStockpileAt,
    STOCKPILE_STOCKPILEARTWORK => crate::products::stockpile::StockpileArtwork,
    EQUIPMENT_LOSSES_EQUIPMENTLOSSHEAD => crate::products::equipment_losses::EquipmentLossHead,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_PRODUCTIONLINESAT: ExactPanel = exact_panel::<crate::products::production::ProductionLinesAt>;
