//! The research folder types the technology content names (`products::artwork`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
