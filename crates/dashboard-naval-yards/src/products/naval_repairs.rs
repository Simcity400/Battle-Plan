//! Dockyard allocation and ordered repair queues: an owner's repair ports, the ships queued at
//! them and its totals, at an hour ([`super::yards`]).
use super::yards::{
    self,
    at::{YardsAtParams, YardsAtReply},
};
use dashboard_base::products::Product;
use replay_data::rows::TableId;

pub struct NavalRepairs;
impl Product for NavalRepairs {
    const KEY: &'static str = "naval-repairs";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = YardsAtParams;
    type Reply = YardsAtReply;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&yards::served::REPAIRS);
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        yards::types()
    }
}
pub(super) const SOURCES: &[TableId] = &[
    TableId::NavalRepairCountries,
    TableId::CountryFactories,
    TableId::NavalBases,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::NavalRepairTaskForces,
    TableId::NavalRepairChildren,
    TableId::VariantDefinitions,
];
