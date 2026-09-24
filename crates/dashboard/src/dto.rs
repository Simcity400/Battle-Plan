//! Types shared by several products and the frontend. Engine identifiers and
//! native fixed-point values are decimal strings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetadata {
    pub path: String,
    pub name: String,
    pub format: u16,
    pub branches: Vec<BranchMetadata>,
    /// Observations from each branch's first readable country snapshot, one
    /// per (slot, tag) at its first observation in branch order. Later
    /// products add their catalogue; this is never a census of all hours.
    pub countries: Vec<CountryMetadata>,
    /// The recorded player's canonical slot, from the first readable snapshot.
    pub player_country_id: Option<String>,
    /// The recorder's identity witness, not a new local asset verification.
    pub content_verified: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchMetadata {
    pub id: String,
    pub session_id: String,
    /// Includes missing leading/trailing hours. This range is not a count of frames.
    pub first_hour: i32,
    pub last_hour: i32,
    /// A non-null parent is probable, never asserted.
    pub parent_id: Option<String>,
    pub parent_hour: Option<i32>,
    pub sample_boundary: String,
    /// Reported capture losses, not a claim that recovered frames are absent.
    pub gaps: Vec<RecordingGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordingGap {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryMetadata {
    /// Canonical country slot, resolved through that hour's CountryLinks.
    pub id: String,
    pub tag: String,
    pub name: String,
    pub observed_branch_id: String,
    pub observed_hour: i32,
}

/// Localised name, description and thumbnail for one game token (a focus or a
/// technology). Missing pieces stay null rather than guessed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    pub token: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub base_duration_days: Option<f64>,
    /// A PNG data URL of the first authored sprite frame.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandOwners {
    pub countries: Vec<CountryMetadata>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusEntry {
    pub ordinal: u32,
    pub token: String,
    /// First observed completion after an earlier readable focus list. This is
    /// an upper bound, not an invented timestamp for pre-recording completions.
    pub completed_by_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusOrder {
    pub country_name: String,
    pub completed: Vec<FocusEntry>,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusSegment {
    pub first_hour: i32,
    pub last_hour: i32,
    pub order: FocusOrder,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusHistory {
    pub segments: Vec<FocusSegment>,
    /// Recorded focus progress, compressed across equal consecutive hours.
    pub progress: Vec<FocusProgress>,
}

/// `CNationalFocusProgress` progress: fixed5 days of work toward the current
/// focus (the daily update adds the day's focus speed; idle days bank it).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusProgress {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Exact signed fixed5 value as a decimal string.
    pub progress_raw: String,
}

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
    pub entry_first_observed_hour: i32,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifiers {
    pub components: Vec<ResearchModifier>,
    pub total_bonus_raw: String,
    pub year: i32,
    pub ahead_factor_raw: String,
    pub minimum_speed_raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifier {
    pub kind: String,
    pub raw: String,
    pub sources: Vec<ResearchModifierSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchModifierSource {
    pub token: Option<String>,
    pub name: Option<String>,
    pub raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchHistory {
    pub coverage: Vec<ResearchCoverage>,
    pub slots: Vec<ResearchSlotHistory>,
}
