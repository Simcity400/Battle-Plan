use dashboard_query::Query;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const HOURS: i32 = 720;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

pub fn windows(query: &Query<'_>) -> io::Result<Vec<ArmySpan>> {
    Ok(coalesce(query.timeline.first, query.timeline.stretches.iter().map(|stretch| (stretch.first, stretch.last))))
}

pub fn coalesce(origin: i32, stretches: impl IntoIterator<Item = (i32, i32)>) -> Vec<ArmySpan> {
    let mut out: Vec<ArmySpan> = Vec::new();
    let step = i64::from(HOURS);
    let base = i64::from(origin);
    for (first, stretch_last) in stretches {
        let mut hour = first;
        while hour <= stretch_last {
            let bucket = (i64::from(hour) - base).div_euclid(step);
            let bucket_last = (base + (bucket + 1) * step - 1).min(i64::from(i32::MAX)) as i32;
            let last = bucket_last.min(stretch_last);
            match out.last_mut() {
                Some(held) if (i64::from(held.first_hour) - base).div_euclid(step) == bucket => {
                    held.last_hour = held.last_hour.max(last);
                }
                _ => out.push(ArmySpan { first_hour: hour, last_hour: last }),
            }
            let Some(next) = last.checked_add(1) else { break };
            hour = next;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_stretches_coalesce_by_time_not_stretch_count() {
        let origin = 61_320_011;
        let sparse: Vec<(i32, i32)> = (0..285).map(|i| (origin + i * 10, origin + i * 10)).collect();
        let dense = vec![(origin, origin + 2_893)];
        let sparse_windows = coalesce(origin, sparse);
        let dense_windows = coalesce(origin, dense);
        assert_eq!(sparse_windows.len(), 4);
        assert_eq!(dense_windows.len(), 5);
        for window in &sparse_windows {
            assert!(window.last_hour - window.first_hour < HOURS);
        }
        let ten_years_sparse: Vec<(i32, i32)> = (0..8_736).map(|i| (origin + i * 10, origin + i * 10)).collect();
        assert_eq!(coalesce(origin, ten_years_sparse).len(), 122);
    }

    #[test]
    fn empty_gaps_between_stretches_do_not_emit_empty_windows() {
        let out = coalesce(100, [(100, 100), (100 + 10 * HOURS, 100 + 10 * HOURS + 5)]);
        assert_eq!(out, vec![ArmySpan { first_hour: 100, last_hour: 100 }, ArmySpan { first_hour: 100 + 10 * HOURS, last_hour: 100 + 10 * HOURS + 5 }]);
    }
}
