//! Wire values shared by land and naval battle products.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BattleWindow {
    pub first_hour: i32,
    pub last_hour: i32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattleWarning {
    pub hour: i32,
    pub reason: String,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BattleBound {
    Exact,
    By,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattleMoment {
    pub hour: i32,
    pub bound: BattleBound,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum BattleEnd {
    Moment(BattleMoment),
    Unknown { unknown: String },
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattlePlace {
    pub name: Option<String>,
    pub hour: i32,
    #[serde(rename = "namedAt", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub named_at: Option<i32>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattleCountry {
    pub tag: String,
    pub hour: i32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum BattleValue {
    Known {
        label: String,
        #[ts(type = "number")]
        value: f64,
        #[serde(rename = "atLeast", skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        at_least: Option<bool>,
    },
    Unknown {
        label: String,
        value: (),
        unknown: String,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattleSide {
    pub label: String,
    pub countries: Vec<BattleCountry>,
    #[serde(rename = "countriesUnknown", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub countries_unknown: Option<String>,
    pub counts: Vec<BattleValue>,
    pub losses: Vec<BattleValue>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum BattleWinner {
    Side { side: usize },
    Unknown { unknown: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct BattleRow {
    pub id: String,
    pub place: BattlePlace,
    pub start: Option<BattleMoment>,
    pub end: Option<BattleEnd>,
    pub sides: Vec<BattleSide>,
    pub winner: BattleWinner,
}

pub fn types() -> Vec<String> {
    vec![
        BattleWindow::decl(),
        BattleWarning::decl(),
        BattleBound::decl(),
        BattleMoment::decl(),
        BattleEnd::decl(),
        BattlePlace::decl(),
        BattleCountry::decl(),
        BattleValue::decl(),
        BattleSide::decl(),
        BattleWinner::decl(),
        BattleRow::decl(),
    ]
}
