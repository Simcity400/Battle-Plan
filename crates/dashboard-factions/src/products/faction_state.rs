//! Observed faction account, selection and goal histories; never a voting or command ledger.
use dashboard_base::{
    prepared::coverage::Coverage,
    products::country_groups::HourSpan,
    products::faction_details::FactionGoalDefinition,
    products::{LoadContext, Product},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod block;
pub mod columnar;
pub mod prepared;
pub mod served;
#[cfg(test)]
mod tests;
pub struct FactionState;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "view", rename_all = "camelCase", deny_unknown_fields)]
pub enum FactionStateParams {
    Influence {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
    },
    Rules {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
    },
    Manpower {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
    },
    Goals {
        #[serde(rename = "branchId")]
        branch_id: String,
        faction: String,
        /// The first and last hours asked for; null leaves that end open.
        #[serde(rename = "firstHour")]
        first_hour: Option<i32>,
        #[serde(rename = "lastHour")]
        last_hour: Option<i32>,
    },
}
impl FactionStateParams {
    pub fn identity(&self) -> (&str, &str) {
        match self {
            Self::Influence { branch_id, faction }
            | Self::Rules { branch_id, faction }
            | Self::Manpower { branch_id, faction }
            | Self::Goals {
                branch_id, faction, ..
            } => (branch_id, faction),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionShareRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub share_raw: Option<String>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionInfluenceAccount {
    pub key: String,
    pub tag: String,
    pub slot: Option<u32>,
    pub country_id: Option<u32>,
    pub runs: Vec<FactionShareRun>,
    pub member: Vec<HourSpan>,
    /// Proven association without current membership, never assigned to every faction.
    pub retained: Vec<HourSpan>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionSelectionRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionSelectionLane {
    pub slot: u32,
    pub runs: Vec<FactionSelectionRun>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionMinimumLane {
    pub group: u32,
    pub runs: Vec<FactionSelectionRun>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionMemberUpgradeLane {
    pub key: String,
    pub tag: String,
    pub slot: Option<u32>,
    pub country_id: Option<u32>,
    pub group: u32,
    pub runs: Vec<FactionSelectionRun>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionUpgradeGroup {
    pub group: u32,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: Option<i32>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerMember {
    pub key: String,
    pub tag: String,
    pub slot: Option<u32>,
    pub country_id: Option<u32>,
    pub runs: Vec<FactionManpowerRun>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionChannelCoverage {
    pub channel: String,
    pub known: Vec<HourSpan>,
    pub read: Vec<HourSpan>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionInfluenceReply {
    pub accounts: Vec<FactionInfluenceAccount>,
    pub unobserved: Vec<HourSpan>,
    pub pending: Vec<HourSpan>,
    pub coverage: Coverage,
    pub channels: Vec<FactionChannelCoverage>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionRulesReply {
    pub rules: Vec<FactionSelectionLane>,
    pub programmes: Vec<FactionSelectionLane>,
    pub minimums: Vec<FactionMinimumLane>,
    pub members: Vec<FactionMemberUpgradeLane>,
    pub doctrines: Vec<FactionSelectionLane>,
    pub groups: Vec<FactionUpgradeGroup>,
    pub unobserved: Vec<HourSpan>,
    pub pending: Vec<HourSpan>,
    pub coverage: Coverage,
    pub channels: Vec<FactionChannelCoverage>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerReply {
    pub members: Vec<FactionManpowerMember>,
    pub total: Vec<FactionManpowerRun>,
    pub unobserved: Vec<HourSpan>,
    pub pending: Vec<HourSpan>,
    pub coverage: Coverage,
    pub channels: Vec<FactionChannelCoverage>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalListRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The held lists, an index into the reply's `lists`.
    pub list: u32,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionHeldGoal {
    pub ordinal: u32,
    /// An index into the reply's `definitions`.
    pub definition: u32,
    pub status: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalList {
    /// The hour's complete faction roster does not list the faction.
    pub absent: bool,
    /// Why the hour's goal lists are unavailable; they are then empty.
    pub reason: Option<String>,
    pub goals: Vec<FactionHeldGoal>,
    pub completed: Vec<u32>,
    pub cancelled: Vec<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalProgressRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Exact retained fixed-point cache as decimal text.
    pub progress_raw: String,
    pub progress_percent: Option<f64>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalProgressLane {
    /// The held goal's ordinal in the runtime list.
    pub ordinal: u32,
    pub runs: Vec<FactionGoalProgressRun>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGoalsReply {
    pub runs: Vec<FactionGoalListRun>,
    pub lists: Vec<FactionGoalList>,
    /// Goals as faction details name them, without their authored checks.
    pub definitions: Vec<FactionGoalDefinition>,
    pub progress: Vec<FactionGoalProgressLane>,
    pub unobserved: Vec<HourSpan>,
    pub pending: Vec<HourSpan>,
    pub coverage: Coverage,
    pub channels: Vec<FactionChannelCoverage>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum FactionStateReply {
    Influence(FactionInfluenceReply),
    Rules(FactionRulesReply),
    Manpower(FactionManpowerReply),
    Goals(FactionGoalsReply),
}
impl dashboard_base::products::ReplyWarnings for FactionStateReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Influence(r) => &mut r.warnings,
            Self::Rules(r) => &mut r.warnings,
            Self::Manpower(r) => &mut r.warnings,
            Self::Goals(r) => &mut r.warnings,
        }
    }
}
impl Product for FactionState {
    const KEY: &'static str = "faction-state";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = FactionStateParams;
    type Reply = FactionStateReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        let (branch, faction) = params.identity();
        if branch.parse::<u64>().is_err() || !faction.parse::<u64>().is_ok_and(|id| id != 0) {
            return Err("Invalid faction state identity");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            FactionChannelCoverage::decl(),
            FactionStateParams::decl(),
            FactionShareRun::decl(),
            FactionInfluenceAccount::decl(),
            FactionSelectionRun::decl(),
            FactionSelectionLane::decl(),
            FactionMinimumLane::decl(),
            FactionMemberUpgradeLane::decl(),
            FactionUpgradeGroup::decl(),
            FactionManpowerRun::decl(),
            FactionManpowerMember::decl(),
            FactionInfluenceReply::decl(),
            FactionRulesReply::decl(),
            FactionManpowerReply::decl(),
            FactionGoalListRun::decl(),
            FactionHeldGoal::decl(),
            FactionGoalList::decl(),
            FactionGoalProgressRun::decl(),
            FactionGoalProgressLane::decl(),
            FactionGoalsReply::decl(),
            FactionStateReply::decl(),
        ]
    }
}
