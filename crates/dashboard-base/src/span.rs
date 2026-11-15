//! A stretch of consecutive game hours, inclusive: the span every reply names hours by.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HourSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}
