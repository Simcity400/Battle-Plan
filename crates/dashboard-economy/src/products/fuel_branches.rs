//! Fuel by military branch: each country's fuel consumers as its fuel status holds them.

use super::keyed::{EconomyRun, HolderTag};
use dashboard_base::{
    Source,
    catalogue,
    products::{LoadContext, Product},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
mod served;

pub const FAMILY: &str = "fuel_branches";
pub const QUERY: &str = "fuel-branches";

pub struct FuelBranches;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelBranchesParams {
    pub branch_id: String,
    /// The countries; every country when empty.
    pub tags: Vec<String>,
}

/// One consumer (military branch) of one country over the branch.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranch {
    /// Its native consumer index.
    pub index: u32,
    /// Fixed5 fuel requested at each read.
    pub requested: Vec<EconomyRun>,
    /// Fixed5 fuel received at each read.
    pub received: Vec<EconomyRun>,
    pub priority: Vec<EconomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranchCountry {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    /// The supplying read's hour.
    pub read: Vec<EconomyRun>,
    pub branches: Vec<FuelBranch>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranchesReply {
    pub branch_id: String,
    /// Through the last prepared hour; later hours are still being prepared.
    pub last_hour: Option<i32>,
    /// Each native consumer index's label from the recorded content; null
    /// where content does not name it.
    pub consumer_names: Vec<Option<String>>,
    /// Fixed-point denominator of the amounts.
    pub scale: u32,
    /// Hours the consumer table was read complete with every consumer's
    /// country identified ("1").
    pub known: Vec<EconomyRun>,
    pub countries: Vec<FuelBranchCountry>,
    pub warnings: Vec<String>,
}

impl Product for FuelBranches {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = FuelBranchesParams;
    type Reply = FuelBranchesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            dashboard_base::products::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            super::keyed::EconomyRun::decl(),
            super::keyed::HolderTag::decl(),
            FuelBranchesParams::decl(),
            FuelBranchesReply::decl(),
            FuelBranchCountry::decl(),
            FuelBranch::decl(),
        ]
    }
}

/// The content labels of the consumers, where the recorded stack names them.
pub fn labels(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> [Option<String>; 3] {
    let catalogue = match context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        Ok(catalogue) => Some(catalogue),
        Err(error) => {
            warnings.push(format!("3 fuel branch labels cannot be resolved: {error}"));
            None
        }
    };
    dashboard_country_fuel::products::country_fuel::consumer_names(catalogue.as_deref(), warnings)
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
