//! Hourly live refit work and independently dated daily target, cost and allocation: an owner's
//! refit lines, the ships on them and its totals, at an hour ([`super::yards`]).
use super::yards::{
    self,
    at::{YardsAtParams, YardsAtReply},
};
use dashboard_base::products::Product;
use replay_data::rows::TableId;

pub struct NavalRefits;
impl Product for NavalRefits {
    const KEY: &'static str = "naval-refits";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = YardsAtParams;
    type Reply = YardsAtReply;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&yards::served::REFITS);
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        yards::types()
    }
}
pub(super) const SOURCES: &[TableId] = &[
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::ProductionLines,
    TableId::VariantDefinitions,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
];
