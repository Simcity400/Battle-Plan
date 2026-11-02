//! The measure replies' shapes every measure family answers in.
use super::MeasureRun;
pub use crate::prepared::series::MeasureKey;
use crate::{
    prepared_chart::{ChartSummary, MarkSummary},
    products::country_measure::BalanceDrop,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasureRead {
    pub tag: String,
    pub hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureState {
    pub tag: String,
    pub hour: i32,
    pub approximate: bool,
    pub sample_hour: Option<i32>,
    pub run: Option<MeasureRun>,
    pub drop: Option<BalanceDrop>,
    pub recorded: Option<bool>,
    pub read_hour: Option<i32>,
    pub experience: Option<crate::products::country_experience::ExperienceRead>,
    pub available: Option<bool>,
    pub segment_first_hour: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureStateReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub states: Vec<MeasureState>,
    pub scale: Option<u32>,
    pub day_hours: Option<i32>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSummaryLine {
    pub tag: String,
    pub summary: ChartSummary,
    pub marks: Option<MarkSummary>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureSummaryReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub series: Vec<MeasureSummaryLine>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureKeyReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub hour: Option<f64>,
    pub approximate: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDropReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub first_ordinal: String,
    pub total: String,
    pub rows: Vec<MeasureEvent>,
    pub warnings: Vec<String>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureEvent {
    pub tag: String,
    pub hour: i32,
    pub ordinal: String,
    #[ts(type = "unknown")]
    pub row: Value,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureMarkReply {
    pub coverage: Option<crate::prepared::coverage::Coverage>,
    pub branch_id: String,
    pub measure: String,
    pub tag: String,
    pub summary: MarkSummary,
    pub warnings: Vec<String>,
}
