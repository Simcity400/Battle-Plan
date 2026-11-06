//! Reply types shared by the research products and the frontend. Engine
//! identifiers and native fixed-point values are decimal strings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCoverage {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ResearchOutcome {
    Active,
    Completed,
    Switched,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchEntry {
    pub token: String,
    pub technology_handle: u32,
    /// Repeated technology IDs have separate template indices; older histories omit this.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub database_index: Option<u32>,
    /// Breaks attribution across missing hours or a changed country identity.
    pub continuity_start_hour: i32,
    /// Completed level when this assignment was first observed.
    pub level: i32,
    pub first_observed_hour: i32,
    /// Earlier differing observation; null for initial assignments and gaps.
    pub started_after_hour: Option<i32>,
    pub last_observed_hour: i32,
    /// First observation of the outcome. Never expose it before this hour.
    pub ended_by_hour: Option<i32>,
    /// Completion classification may arrive on a later inventory refresh.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub outcome_observed_hour: Option<i32>,
    pub outcome: ResearchOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotHistory {
    pub index: u32,
    pub availability: Vec<ResearchCoverage>,
    pub entries: Vec<ResearchEntry>,
    /// Recorded slot effects and assignment, compressed only across equal consecutive hours.
    #[serde(default)]
    pub states: Vec<ResearchSlotState>,
    /// Progress/cost observations are compressed separately so daily progress
    /// does not duplicate the complete modifier/source tree.
    pub progress: Vec<ResearchProgress>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchProgress {
    pub first_hour: i32,
    pub last_hour: i32,
    // The assignment this progress belongs to; none for a technology the slot
    // held without its recorded occurrence (older recordings), whose slot state
    // at the same hours names it.
    pub entry_first_observed_hour: Option<i32>,
    /// Base duration in fixed5 days, calculated from recorded cost operands.
    pub base_days_raw: Option<String>,
    pub progress_raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSlotState {
    pub first_hour: i32,
    pub last_hour: i32,
    pub token: Option<String>,
    pub entry_first_observed_hour: Option<i32>,
    /// Native fixed5 operands. Strings preserve the complete signed i64 range.
    pub speed_multiplier_raw: String,
    pub banked_days_raw: String,
    pub applied_days_raw: String,
    pub ahead_reduction_years_raw: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub details: Option<ResearchDetails>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDetails {
    /// Ceiling at the segment's first hour. Subtract complete days elapsed within
    /// this segment. Never inferred from future completion.
    pub remaining_days: Option<u32>,
    /// None means attribution unavailable; empty means no applied IDs.
    pub bonuses: Option<Vec<ResearchBonus>>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub modifiers: Option<ResearchModifiers>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS, replay_spool::Spooled)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifiers {
    pub components: Vec<ResearchModifier>,
    pub total_bonus_raw: String,
    pub year: i32,
    pub ahead_factor_raw: String,
    pub minimum_speed_raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS, replay_spool::Spooled)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifier {
    pub kind: String,
    pub raw: String,
    pub sources: Vec<ResearchModifierSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS, replay_spool::Spooled)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifierSource {
    pub token: Option<String>,
    pub name: Option<String>,
    pub raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS, replay_spool::Spooled)]
#[serde(rename_all = "camelCase")]
pub struct ResearchBonus {
    pub id: i32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub speed_bonus_raw: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub cost_reduction_raw: Option<String>,
    pub ahead_reduction_years_raw: Option<String>,
}

// A research folder (a tab of the technology screen), in content order, with
// its localized name when the content defines one.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchFolder {
    pub key: String,
    pub name: Option<String>,
}

// The folder a technology is shown in: the first folder it is placed in whose
// tab is available. None when it is in no available folder or when a folder it
// is placed in before any available one cannot be decided from content.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchTechnologyFolder {
    pub token: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub database_index: Option<u32>,
    pub folder: Option<String>,
}

// Recorded definitions have separate text, but artwork is keyed by the technology token.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDefinitionText {
    pub token: String,
    pub database_index: u32,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchHistory {
    pub coverage: Vec<ResearchCoverage>,
    pub slots: Vec<ResearchSlotHistory>,
}
