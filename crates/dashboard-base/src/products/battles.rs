use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum BattleBound {
    Exact,
    By,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
pub struct BattleMoment {
    pub hour: i32,
    pub bound: BattleBound,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum BattleEnd {
    Moment(BattleMoment),
    Unknown { unknown: String },
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum BattleWinner {
    Side { side: usize },
    Unknown { unknown: String },
}
