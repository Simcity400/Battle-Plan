//! Recorded diplomacy, with sparse zero proof separate from numeric values.
use dashboard_base::prepared::coverage::Coverage;
use dashboard_base::products::country_groups::HourSpan;
use dashboard_base::products::{Product, ReplyWarnings, validate_tag};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod block;
mod evidence;
pub mod prepared;
pub mod query;

pub struct DiplomacyState;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(
    tag = "view",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DiplomacyParams {
    Opinions {
        branch_id: String,
        hour: i32,
        tags: Vec<String>,
    },
    /// Both countries are the holders of their tags at `hour`, the hour the pair was chosen.
    Pair {
        branch_id: String,
        first: String,
        second: String,
        hour: i32,
    },
    Pacts {
        branch_id: String,
        tags: Option<Vec<String>>,
    },
    Proposals {
        branch_id: String,
        tags: Option<Vec<String>>,
    },
}
impl DiplomacyParams {
    pub fn branch_id(&self) -> &str {
        match self {
            Self::Opinions { branch_id, .. }
            | Self::Pair { branch_id, .. }
            | Self::Pacts { branch_id, .. }
            | Self::Proposals { branch_id, .. } => branch_id,
        }
    }
    pub fn branch(&self) -> u64 {
        self.branch_id().parse().expect("validated branch")
    }
    pub fn hour(&self) -> Option<i32> {
        if let Self::Opinions { hour, .. } = self {
            Some(*hour)
        } else {
            None
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyCountry {
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct DiplomacyOpinion {
    pub target: String,
    pub value: i32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyOwner {
    pub tag: String,
    pub identity: Option<DiplomacyCountry>,
    pub read_hour: Option<i32>,
    /// A complete opinion read made while the owner held this identity is recorded.
    pub known: bool,
    /// Selected tags a known owner's read cannot prove; any other one it lists no opinion of is zero.
    pub unknown_targets: Vec<String>,
    pub opinions: Vec<DiplomacyOpinion>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyOpinionsReply {
    pub hour: i32,
    pub owners: Vec<DiplomacyOwner>,
    pub unresolved: usize,
    /// True only for an exact, cadence-bounded recorded snapshot beyond the prepared prefix.
    pub snapshot: bool,
    pub coverage: Coverage,
    pub pending: Vec<HourSpan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: Option<i32>,
    pub starts_segment: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyPairReply {
    pub forward: Vec<DiplomacyRun>,
    pub reverse: Vec<DiplomacyRun>,
    pub unobserved: Vec<HourSpan>,
    pub first: Option<DiplomacyCountry>,
    pub second: Option<DiplomacyCountry>,
    pub coverage: Coverage,
    pub pending: Vec<HourSpan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct DiplomacyKind {
    pub kind: String,
    pub name: Option<String>,
    pub reason: Option<String>,
    pub count: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyPact {
    pub id: String,
    pub kind: String,
    pub owner: Option<String>,
    pub target: Option<String>,
    pub first: Option<String>,
    pub second: Option<String>,
    pub identities: Vec<Option<DiplomacyCountry>>,
    pub raw_countries: Vec<u32>,
    pub ordinal: u16,
    pub start_hour: Option<i32>,
    pub end_hour: Option<i32>,
    pub cancelled: Option<bool>,
    pub retired: Option<bool>,
    pub cancel_raw: u8,
    pub retired_raw: u8,
    pub first_seen: i32,
    pub last_seen: i32,
    pub present_at_end: bool,
    pub ended_by: bool,
    pub read_hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyReadRun {
    pub owner: Option<DiplomacyCountry>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyPactsReply {
    pub kinds: Vec<DiplomacyKind>,
    pub pacts: Vec<DiplomacyPact>,
    pub reads: Vec<DiplomacyReadRun>,
    pub unobserved: Vec<HourSpan>,
    pub coverage: Coverage,
    pub pending: Vec<HourSpan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyProposal {
    pub id: String,
    pub handle: String,
    pub owner: Option<String>,
    pub sender: Option<String>,
    pub recipient: Option<String>,
    pub original_sender: Option<String>,
    pub original_recipient: Option<String>,
    pub identities: Vec<Option<DiplomacyCountry>>,
    pub raw_countries: Vec<u32>,
    pub stored_hour: i32,
    pub last_command_hour: i32,
    pub first_seen: i32,
    pub last_seen: i32,
    pub gone_by: Option<i32>,
    pub gone_by_bound: bool,
    pub present_at_end: bool,
    pub read_hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiplomacyProposalsReply {
    pub proposals: Vec<DiplomacyProposal>,
    pub reads: Vec<DiplomacyReadRun>,
    pub unobserved: Vec<HourSpan>,
    pub coverage: Coverage,
    pub pending: Vec<HourSpan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum DiplomacyReply {
    Opinions(DiplomacyOpinionsReply),
    Pair(DiplomacyPairReply),
    Pacts(DiplomacyPactsReply),
    Proposals(DiplomacyProposalsReply),
}
impl ReplyWarnings for DiplomacyReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Opinions(r) => &mut r.warnings,
            Self::Pair(r) => &mut r.warnings,
            Self::Pacts(r) => &mut r.warnings,
            Self::Proposals(r) => &mut r.warnings,
        }
    }
}
impl Product for DiplomacyState {
    const KEY: &'static str = "diplomacy-state";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DiplomacyParams;
    type Reply = DiplomacyReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id()
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        match params {
            DiplomacyParams::Opinions { tags, .. } => {
                for tag in tags {
                    validate_tag(tag)?;
                }
            }
            DiplomacyParams::Pair { first, second, .. } => {
                validate_tag(first)?;
                validate_tag(second)?;
            }
            DiplomacyParams::Pacts { tags, .. } | DiplomacyParams::Proposals { tags, .. } => {
                if let Some(tags) = tags {
                    for tag in tags {
                        validate_tag(tag)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            DiplomacyParams::decl(),
            DiplomacyReply::decl(),
            DiplomacyCountry::decl(),
            DiplomacyOpinion::decl(),
            DiplomacyOwner::decl(),
            DiplomacyOpinionsReply::decl(),
            DiplomacyRun::decl(),
            DiplomacyPairReply::decl(),
            DiplomacyKind::decl(),
            DiplomacyPact::decl(),
            DiplomacyReadRun::decl(),
            DiplomacyPactsReply::decl(),
            DiplomacyProposal::decl(),
            DiplomacyProposalsReply::decl(),
        ]
    }
}
#[cfg(test)]
mod tests;
