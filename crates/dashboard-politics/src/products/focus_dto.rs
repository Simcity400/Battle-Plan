use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusEntry {
    pub ordinal: u32,
    pub token: String,
    #[doc = " First observed completion after an earlier readable focus list. This is"]
    #[doc = " an upper bound, not an invented timestamp for pre-recording completions."]
    pub completed_by_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusOrder {
    pub completed: Vec<FocusEntry>,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusSegment {
    pub first_hour: i32,
    pub last_hour: i32,
    pub order: FocusOrder,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusHistory {
    pub segments: Vec<FocusSegment>,
    #[doc = " Recorded focus progress, compressed across equal consecutive hours."]
    pub progress: Vec<FocusProgress>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusProgress {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Exact signed fixed5 value as a decimal string."]
    pub progress_raw: String,
}
