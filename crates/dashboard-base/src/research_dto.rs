
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchFolder {
    pub key: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchTechnologyFolder {
    pub token: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub database_index: Option<u32>,
    pub folder: Option<String>,
}
