//! Constant-size time envelopes: no complete campaign history is retained.

use crate::{
    CountryMetricCounts, CountrySample, CountrySeriesPoint, CountryValues, PointStatus,
    SeriesCoverage,
};
use replay::rows::CountryRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Values(pub [Option<i64>; 5]);

impl Values {
    pub fn from_row(row: &CountryRow) -> Self {
        Self([
            row.political_power_raw,
            Some(row.command_power_raw),
            Some(row.stability_raw),
            Some(row.war_support_raw),
            row.convoys_raw,
        ])
    }

    fn dto(self) -> CountryValues {
        let [
            political_power,
            command_power,
            stability,
            war_support,
            convoys,
        ] = self.0.map(|value| value.map(|value| value.to_string()));
        CountryValues {
            political_power,
            command_power,
            stability,
            war_support,
            convoys,
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    hour: i32,
    values: Values,
}

impl Sample {
    fn dto(self) -> CountrySample {
        CountrySample {
            hour: self.hour,
            values: self.values.dto(),
        }
    }
}

struct Bucket {
    first_hour: i32,
    last_hour: i32,
    coverage: SeriesCoverage,
    first: Option<Sample>,
    last: Option<Sample>,
    min: Values,
    max: Values,
    missing: [u64; 5],
    reason: Option<String>,
}

impl Bucket {
    fn new(first_hour: i32, last_hour: i32) -> Self {
        Self {
            first_hour,
            last_hour,
            coverage: SeriesCoverage::default(),
            first: None,
            last: None,
            min: Values([None; 5]),
            max: Values([None; 5]),
            missing: [0; 5],
            reason: None,
        }
    }

    fn record(&mut self, hour: i32, values: Values) {
        self.coverage.recorded_hours += 1;
        let sample = Sample { hour, values };
        self.first.get_or_insert(sample);
        self.last = Some(sample);
        for (index, value) in values.0.into_iter().enumerate() {
            if let Some(value) = value {
                self.min.0[index] = Some(self.min.0[index].map_or(value, |old| old.min(value)));
                self.max.0[index] = Some(self.max.0[index].map_or(value, |old| old.max(value)));
            } else {
                self.missing[index] += 1;
            }
        }
    }

    fn finish(self) -> CountrySeriesPoint {
        let candidates = [
            (self.coverage.recorded_hours, PointStatus::Recorded),
            (self.coverage.gap_hours, PointStatus::Gap),
            (self.coverage.unavailable_hours, PointStatus::Unavailable),
            (self.coverage.absent_hours, PointStatus::Absent),
        ];
        let mut active = candidates.into_iter().filter(|(count, _)| *count > 0);
        let status = match (active.next(), active.next()) {
            (Some((_, status)), None) => status,
            _ => PointStatus::Mixed,
        };
        let [
            political_power,
            command_power,
            stability,
            war_support,
            convoys,
        ] = self.missing;
        CountrySeriesPoint {
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            status,
            recorded_hours: self.coverage.recorded_hours,
            gap_hours: self.coverage.gap_hours,
            unavailable_hours: self.coverage.unavailable_hours,
            absent_hours: self.coverage.absent_hours,
            first: self.first.map(Sample::dto),
            last: self.last.map(Sample::dto),
            min: self.min.dto(),
            max: self.max.dto(),
            missing_metric_hours: CountryMetricCounts {
                political_power,
                command_power,
                stability,
                war_support,
                convoys,
            },
            reason: self.reason,
        }
    }
}

pub(crate) struct Reducer {
    first_hour: i32,
    width: u64,
    buckets: Vec<Bucket>,
}

impl Reducer {
    pub fn new(first_hour: i32, last_hour: i32, max_points: u32) -> Self {
        let span = (i64::from(last_hour) - i64::from(first_hour) + 1) as u64;
        let width = span.div_ceil(u64::from(max_points));
        let mut buckets = Vec::with_capacity(span.div_ceil(width) as usize);
        let mut first = i64::from(first_hour);
        while first <= i64::from(last_hour) {
            let last = (first + width as i64 - 1).min(i64::from(last_hour));
            buckets.push(Bucket::new(first as i32, last as i32));
            first = last + 1;
        }
        Self {
            first_hour,
            width,
            buckets,
        }
    }

    fn index(&self, hour: i32) -> usize {
        ((i64::from(hour) - i64::from(self.first_hour)) as u64 / self.width) as usize
    }

    pub fn record(&mut self, hour: i32, values: Values) {
        let index = self.index(hour);
        self.buckets[index].record(hour, values);
    }

    /// Call only for a disjoint, inclusive interval inside the requested branch.
    /// Even a multi-year loss visits at most max_points buckets, never every hour.
    pub fn missing(&mut self, first: i32, last: i32, status: PointStatus, reason: &str) {
        if first > last {
            return;
        }
        let start = self.index(first);
        let end = self.index(last);
        for bucket in &mut self.buckets[start..=end] {
            let count = (i64::from(last.min(bucket.last_hour))
                - i64::from(first.max(bucket.first_hour))
                + 1) as u64;
            match status {
                PointStatus::Gap => bucket.coverage.gap_hours += count,
                PointStatus::Unavailable => bucket.coverage.unavailable_hours += count,
                PointStatus::Absent => bucket.coverage.absent_hours += count,
                PointStatus::Recorded | PointStatus::Mixed => unreachable!("not a loss state"),
            }
            if bucket.reason.is_none() {
                bucket.reason = Some(short_text(reason, 512));
            }
        }
    }

    pub fn finish(self) -> (Vec<CountrySeriesPoint>, SeriesCoverage) {
        let mut coverage = SeriesCoverage::default();
        let points = self
            .buckets
            .into_iter()
            .map(|bucket| {
                coverage.recorded_hours += bucket.coverage.recorded_hours;
                coverage.gap_hours += bucket.coverage.gap_hours;
                coverage.unavailable_hours += bucket.coverage.unavailable_hours;
                coverage.absent_hours += bucket.coverage.absent_hours;
                debug_assert_eq!(
                    bucket.coverage.recorded_hours
                        + bucket.coverage.gap_hours
                        + bucket.coverage.unavailable_hours
                        + bucket.coverage.absent_hours,
                    (i64::from(bucket.last_hour) - i64::from(bucket.first_hour) + 1) as u64,
                );
                bucket.finish()
            })
            .collect();
        (points, coverage)
    }
}

pub(crate) fn short_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes.saturating_sub(3);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}
