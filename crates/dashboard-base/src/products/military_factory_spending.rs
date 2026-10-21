//! Where a country's military factory IC goes, for air or for land equipment:
//! factory IC applied to new production and to conversions, each per target
//! design.
//!
//! Both are Production spending's exact applied input, split by its recorded
//! purpose ([`Purposes::Split`]) and kept to the chosen domain's designs and
//! designs whose type content cannot resolve (which could be in it). A line
//! converting stockpiled equipment applies its input to the target design.
//! Input of a production tick whose purpose was not recorded is its own part,
//! with its dated `unproven` evidence; input of a purpose the domain never has
//! is counted in the whole as another part.
//!
//! [`Purposes::Split`]: super::production_spending::Purposes::Split

use super::{
    Product,
    dockyard_spending::{DockyardAmount, DockyardView},
    equipment::Domain,
    production_spending::ProductionSpendingReply,
    validate_tag,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod prepared;

pub struct MilitaryFactorySpending;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryFactorySpendingParams {
    pub branch_id: String,
    pub tag: String,
    pub view: DockyardView,
    pub equipment: FactoryEquipment,
}

/// Which military factory equipment a reply shows.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactoryEquipment {
    Air,
    Land,
}

impl FactoryEquipment {
    pub fn domain(self) -> Domain {
        match self {
            Self::Air => Domain::Air,
            Self::Land => Domain::Land,
        }
    }
}

/// One calendar month's military factory IC, through the reply's hour.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryMonth {
    pub first_hour: i32,
    pub last_hour: i32,
    pub new: DockyardAmount,
    pub converted: DockyardAmount,
    /// The domain's input of production ticks whose purpose was not
    /// recorded: it could be new or converted.
    pub unrecorded: DockyardAmount,
    /// The domain's input recorded with a purpose military production never
    /// has, counted in the whole and warned about.
    pub other: DockyardAmount,
    /// Input whose purpose was not recorded joined the month, however it
    /// nets: new and converted input are not wholly attributed.
    pub purpose_unproven: bool,
    /// A design whose type content cannot resolve spent IC this month, so
    /// the domain's amounts may lack it.
    pub unresolved: bool,
}

#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MilitaryFactorySpendingReply {
    /// Production spending's reply split by purpose, holding only the
    /// domain's and unresolved designs' new and converted leaves (none in the
    /// months view).
    #[serde(flatten)]
    pub spending: ProductionSpendingReply,
    /// The months view's columns, in order; empty in the circle view.
    pub months: Vec<FactoryMonth>,
}

impl super::ReplyWarnings for MilitaryFactorySpendingReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.spending.warnings
    }
}

impl Product for MilitaryFactorySpending {
    const KEY: &'static str = "military-factory-spending";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MilitaryFactorySpendingParams;
    type Reply = MilitaryFactorySpendingReply;

    fn types() -> Vec<String> {
        vec![
            MilitaryFactorySpendingParams::decl(),
            FactoryEquipment::decl(),
            FactoryMonth::decl(),
            MilitaryFactorySpendingReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        validate_tag(&params.tag)
    }
}

#[cfg(test)]
mod tests;
