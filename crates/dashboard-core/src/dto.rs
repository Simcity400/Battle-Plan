//! Recording metadata and token artwork shared by several products and the
//! frontend. Engine identifiers and native fixed-point values are decimal strings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetadata {
    pub path: String,
    pub name: String,
    pub format: u16,
    /// The recorded campaign identity as hex, the same key the map settings use.
    /// Every recording of one campaign shares it; app settings are stored under it.
    pub campaign_key: String,
    pub branches: Vec<BranchMetadata>,
    /// Observations from each branch's first readable country snapshot, one
    /// per (slot, tag) at its first observation in branch order. Later
    /// products add their catalogue; this is never a census of all hours.
    pub countries: Vec<CountryMetadata>,
    /// The recorded player's canonical slot, from the first readable snapshot.
    pub player_country_id: Option<String>,
    /// The recorder's identity witness, not a new local asset verification.
    pub content_verified: bool,
    /// The capture stopped after the verified prefix; independent of gap coverage.
    pub partial: Option<PartialRecording>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub damaged: Option<DamagedRecording>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DamagedRecording {
    pub parts: usize,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PartialRecording {
    pub session_id: String,
    pub reason: String,
    /// No hour is inferred for an idle or status failure.
    pub failed_hour: Option<i32>,
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
    /// Recorded event losses: intervals between two reads whose recorded
    /// events are incomplete because the recorder was detached for part of
    /// them. They are not gaps; a lost frame at either read is its own gap.
    /// Absent when the branch has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub event_losses: Option<Vec<RecordingEventLoss>>,
}

/// An interval between the reads of `after_hour` and `hour` whose recorded
/// events are incomplete (`ControlRecord::EventLoss`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEventLoss {
    pub after_hour: i32,
    pub hour: i32,
    pub reason: String,
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
    /// The display name in the user's game language, or the tag when the game
    /// folders are not chosen or the name cannot be resolved.
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
