use dashboard_query::Query;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

pub fn windows(query: &Query<'_>) -> io::Result<Vec<ArmySpan>> {
    let recording = query.replay.recording();
    let mut out = Vec::new();
    for stretch in &query.timeline.stretches {
        let held = recording.run(stretch.run)?;
        let mut hour = stretch.first;
        while hour <= stretch.last {
            let ordinal = (i64::from(hour) - i64::from(held.first_hour)) as u32;
            let block_last = held.first_hour + ((ordinal / 720 + 1) * 720) as i32 - 1;
            let last = block_last.min(stretch.last);
            out.push(ArmySpan { first_hour: hour, last_hour: last });
            hour = last + 1;
        }
    }
    Ok(out)
}
