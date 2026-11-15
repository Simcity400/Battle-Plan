//! A chart reply: one bucket per bin, with its first, last, least and greatest value and hours, on
//! the wire as columns; hourly views carry their original runs. Exact integers stay in value runs.
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChartBucket {
    /// Sum of absolute raw value times covered hours, at the series decimal scale.
    #[serde(with = "absolute_decimal")]
    #[ts(type = "string")]
    pub absolute_total: u128,
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_value: f64,
    pub last_value: f64,
    pub last_value_hour: i32,
    pub min_hour: i32,
    pub min_value: f64,
    pub max_hour: i32,
    pub max_value: f64,
    pub first_break: bool,
    pub has_break: bool,
}

#[cfg(test)]
impl ChartBucket {
    pub fn hour(hour: i32, value: f64, break_before: bool) -> Self {
        Self {
            absolute_total: value.abs() as u128,
            first_hour: hour,
            last_hour: hour,
            first_value: value,
            last_value: value,
            last_value_hour: hour,
            min_hour: hour,
            min_value: value,
            max_hour: hour,
            max_value: value,
            first_break: break_before,
            has_break: false,
        }
    }
}

/// Ordered merge; missing children stay missing and cannot establish zero.
pub fn merge(first: Option<ChartBucket>, second: Option<ChartBucket>) -> Option<ChartBucket> {
    match (first, second) {
        (None, None) => None,
        (Some(mut value), None) | (None, Some(mut value)) => {
            value.has_break = true;
            Some(value)
        }
        (Some(a), Some(b)) => Some(ChartBucket {
            absolute_total: a.absolute_total + b.absolute_total,
            first_hour: a.first_hour,
            last_hour: b.last_hour,
            first_value: a.first_value,
            last_value: b.last_value,
            // A break after `b`'s one stretch is a coverage gap: the value still began in `a`.
            last_value_hour: if b.last_value_hour == b.first_hour
                && !b.first_break
                && i64::from(a.last_hour) + 1 == i64::from(b.first_hour)
                && b.min_value == b.max_value
                && a.last_value == b.first_value
            {
                a.last_value_hour
            } else {
                b.last_value_hour
            },
            min_hour: if a.min_value <= b.min_value {
                a.min_hour
            } else {
                b.min_hour
            },
            min_value: a.min_value.min(b.min_value),
            max_hour: if a.max_value >= b.max_value {
                a.max_hour
            } else {
                b.max_hour
            },
            max_value: a.max_value.max(b.max_value),
            first_break: a.first_break,
            has_break: a.has_break
                || b.has_break
                || b.first_break
                || i64::from(a.last_hour) + 1 != i64::from(b.first_hour),
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChartRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: f64,
    pub break_before: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChartSummary {
    pub from: f64,
    pub to: f64,
    pub width: u32,
    #[serde(with = "columns")]
    #[ts(type = "ChartColumns")]
    pub buckets: Vec<Option<ChartBucket>>,
    /// Original supplying runs for hourly bins, including their unclipped bounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runs: Option<Vec<ChartRun>>,
}

/// A summary's buckets on the wire: per present bucket in slot order, a quarter of their named bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChartColumns {
    /// Every slot, present or not.
    pub slots: u32,
    /// Absent slots before each present bucket.
    pub skip: Vec<u32>,
    /// First hour less the previous bucket's last (the first's whole); last, last-value, min, max less first.
    #[ts(type = "Array<number>")]
    pub hours: Vec<i64>,
    /// First, last, minimum and maximum values.
    pub values: Vec<f64>,
    /// 1 when the bucket starts with a break, 2 when it holds one.
    pub flags: Vec<u8>,
    /// Absolute totals as decimals.
    pub totals: Vec<String>,
}

impl ChartColumns {
    /// The buckets these columns hold, refused when the columns disagree.
    pub fn buckets(&self) -> io::Result<Vec<Option<ChartBucket>>> {
        let present = self.skip.len();
        if self.hours.len() != 5 * present
            || self.values.len() != 4 * present
            || self.flags.len() != present
            || self.totals.len() != present
        {
            return Err(invalid("summary columns disagree on their buckets"));
        }
        let mut buckets = Vec::with_capacity(self.slots as usize);
        let mut previous: Option<i64> = None;
        let hour = |value: i64| i32::try_from(value).map_err(|_| invalid("summary hour overflow"));
        for index in 0..present {
            if buckets.len() + self.skip[index] as usize >= self.slots as usize {
                return Err(invalid("summary columns hold more buckets than slots"));
            }
            buckets.resize(buckets.len() + self.skip[index] as usize, None);
            let hours = &self.hours[5 * index..5 * index + 5];
            let first = previous
                .map_or(Some(hours[0]), |last| last.checked_add(hours[0]))
                .ok_or_else(|| invalid("summary hour overflow"))?;
            let at = |offset: i64| {
                first
                    .checked_add(offset)
                    .ok_or_else(|| invalid("summary hour overflow"))
                    .and_then(hour)
            };
            let values = &self.values[4 * index..4 * index + 4];
            let flags = self.flags[index];
            if flags > 3 {
                return Err(invalid("invalid summary flags"));
            }
            let bucket = ChartBucket {
                absolute_total: self.totals[index]
                    .parse()
                    .map_err(|_| invalid("invalid summary total"))?,
                first_hour: hour(first)?,
                last_hour: at(hours[1])?,
                last_value_hour: at(hours[2])?,
                min_hour: at(hours[3])?,
                max_hour: at(hours[4])?,
                first_value: values[0],
                last_value: values[1],
                min_value: values[2],
                max_value: values[3],
                first_break: flags & 1 != 0,
                has_break: flags & 2 != 0,
            };
            previous = Some(i64::from(bucket.last_hour));
            buckets.push(Some(bucket));
        }
        buckets.resize(self.slots as usize, None);
        Ok(buckets)
    }
}

/// [`ChartColumns`] written straight from the buckets, and read back into them.
mod columns {
    use super::{ChartBucket, ChartColumns};
    use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

    /// A sequence serialized as its iterator yields it, never collected.
    struct Each<F>(F);
    impl<F: Fn() -> I, I: Iterator<Item = T>, T: Serialize> Serialize for Each<F> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_seq((self.0)())
        }
    }
    /// A decimal string, written without allocating one.
    struct Decimal(u128);
    impl Serialize for Decimal {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(&self.0)
        }
    }

    pub fn serialize<S: Serializer>(
        buckets: &[Option<ChartBucket>],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let slots = u32::try_from(buckets.len()).map_err(serde::ser::Error::custom)?;
        let present = || buckets.iter().flatten();
        let mut columns = serializer.serialize_struct("ChartColumns", 6)?;
        columns.serialize_field("slots", &slots)?;
        columns.serialize_field(
            "skip",
            &Each(|| {
                buckets
                    .iter()
                    .scan(0_u32, |gap, bucket| {
                        Some(match bucket {
                            None => {
                                *gap += 1;
                                None
                            }
                            Some(_) => Some(std::mem::take(gap)),
                        })
                    })
                    .flatten()
            }),
        )?;
        columns.serialize_field(
            "hours",
            &Each(|| {
                present()
                    .scan(None::<i32>, |previous, bucket| {
                        let first = i64::from(bucket.first_hour);
                        let lead = previous.map_or(first, |last| first - i64::from(last));
                        *previous = Some(bucket.last_hour);
                        Some([
                            lead,
                            i64::from(bucket.last_hour) - first,
                            i64::from(bucket.last_value_hour) - first,
                            i64::from(bucket.min_hour) - first,
                            i64::from(bucket.max_hour) - first,
                        ])
                    })
                    .flatten()
            }),
        )?;
        columns.serialize_field(
            "values",
            &Each(|| {
                present().flat_map(|bucket| {
                    [
                        bucket.first_value,
                        bucket.last_value,
                        bucket.min_value,
                        bucket.max_value,
                    ]
                })
            }),
        )?;
        columns.serialize_field(
            "flags",
            &Each(|| {
                present()
                    .map(|bucket| u8::from(bucket.first_break) | (u8::from(bucket.has_break) << 1))
            }),
        )?;
        columns.serialize_field(
            "totals",
            &Each(|| present().map(|bucket| Decimal(bucket.absolute_total))),
        )?;
        columns.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Option<ChartBucket>>, D::Error> {
        ChartColumns::deserialize(deserializer)?
            .buckets()
            .map_err(serde::de::Error::custom)
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarkCluster {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_ordinal: String,
    pub count: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarkSummary {
    pub from: f64,
    pub to: f64,
    pub width: u32,
    pub clusters: Vec<MarkCluster>,
}


mod absolute_decimal {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
