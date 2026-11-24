
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetadata {
    pub path: String,
    pub name: String,
    pub format: u16,
    pub capture_mode: CaptureMode,
    #[doc = " The recorded campaign identity as hex, the same key the map settings use."]
    #[doc = " Every recording of one campaign shares it; app settings are stored under it."]
    pub campaign_key: String,
    pub branches: Vec<BranchMetadata>,
    #[doc = " Observations from each branch's first readable country snapshot, one"]
    #[doc = " per (slot, tag) at its first observation in branch order. Later"]
    #[doc = " products add their catalogue; this is never a census of all hours."]
    pub countries: Vec<CountryMetadata>,
    #[doc = " The recorded player's canonical slot, from the first readable snapshot."]
    pub player_country_id: Option<String>,
    #[doc = " The recorder's identity witness, not a new local asset verification."]
    pub content_verified: bool,
    #[doc = " The capture stopped after the verified prefix; independent of gap coverage."]
    pub partial: Option<PartialRecording>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub damaged: Option<DamagedRecording>,
    pub warnings: Vec<String>,
    #[doc = " The recorded and installed game builds, when either needs a warning; worked out as"]
    #[doc = " metadata is served, since the installed game can change under a cached recording."]
    #[serde(default)]
    pub build_warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum CaptureMode {
    Full,
    Map,
}

impl From<replay_data::scope::CaptureMode> for CaptureMode {
    fn from(mode: replay_data::scope::CaptureMode) -> Self {
        match mode {
            replay_data::scope::CaptureMode::Full => Self::Full,
            replay_data::scope::CaptureMode::Map => Self::Map,
        }
    }
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
    #[doc = " No hour is inferred for an idle or status failure."]
    pub failed_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchMetadata {
    pub id: String,
    pub session_id: String,
    #[doc = " Includes missing leading/trailing hours. This range is not a count of frames."]
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The history holding the latest recorded play: the campaign as it went on. Listed first."]
    pub main: bool,
    #[doc = " For a history other than the main one that shares its early play: the main history, and"]
    #[doc = " the first hour where they part, from which this one's play was left behind."]
    pub fork: Option<BranchFork>,
    #[doc = " Each place it goes on with play recorded after a load or in another session."]
    pub joins: Vec<BranchJoin>,
    pub sample_boundary: String,
    #[doc = " Reported capture losses, not a claim that recovered frames are absent."]
    pub gaps: Vec<RecordingGap>,
    #[doc = " Recorded event losses: intervals between two reads whose recorded"]
    #[doc = " events are incomplete because the recorder was detached for part of"]
    #[doc = " them. They are not gaps; a lost frame at either read is its own gap."]
    #[doc = " Absent when the branch has none."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub event_losses: Option<Vec<RecordingEventLoss>>,
    #[doc = " Without a complete country-table read, catalogue membership is unknown, not empty."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub catalogue_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchFork {
    pub branch_id: String,
    pub hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum JoinEvidence {
    Counters,
    Only,
    Process,
    Unrecorded,
    Unconfirmed,
    Chosen,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchJoin {
    #[doc = " The first hour of the play joined."]
    pub hour: i32,
    #[doc = " The recorded stretch joined, which a choice names."]
    pub segment_id: String,
    #[doc = " The recorded stretch it goes on from."]
    pub parent_id: String,
    pub evidence: JoinEvidence,
    #[doc = " What the viewer may join it to instead, when the recording can't tell: each earlier"]
    #[doc = " stretch the save could have come from. Empty when proven."]
    pub choices: Vec<JoinChoice>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct JoinChoice {
    pub segment_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

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
    #[doc = " Canonical country slot, resolved through that hour's CountryLinks."]
    pub id: String,
    pub tag: String,
    #[doc = " The display name in the user's game language, or the tag when the game"]
    #[doc = " folders are not chosen or the name cannot be resolved."]
    pub name: String,
    pub observed_branch_id: String,
    pub observed_hour: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum FocusDatabase {
    National,
    Continuous,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct Artwork {
    pub token: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub base_duration_days: Option<f64>,
    #[doc = " The artwork URL of the first authored sprite frame."]
    pub icon: Option<String>,
    #[doc = " The sprite the icon was drawn from, for the art resolver."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sprite: Option<String>,
}
