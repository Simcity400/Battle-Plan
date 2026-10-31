//! Numeric display buckets shared by prepared series. Exact integers stay in value runs.
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

#[cfg(any(test, feature = "test-support"))]
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
            last_value_hour: if !b.has_break
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
    /// Display-only keyframe estimates; these hours are not exact values.
    #[serde(default)]
    pub approximate: Vec<crate::products::country_groups::HourSpan>,
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

pub(crate) fn var(bytes: &mut Vec<u8>, mut value: u128) {
    while value >= 128 {
        bytes.push(value as u8 | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}
pub(crate) fn take(bytes: &mut &[u8]) -> io::Result<u128> {
    let mut value = 0;
    for shift in (0..128).step_by(7) {
        let (&byte, rest) = bytes
            .split_first()
            .ok_or_else(|| invalid("truncated series integer"))?;
        *bytes = rest;
        if shift == 126 && byte > 3 {
            return Err(invalid("series integer overflow"));
        }
        value |= u128::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(invalid("series integer overflow"))
}
pub(crate) fn zig(value: i128) -> u128 {
    ((value as u128) << 1) ^ ((value >> 127) as u128)
}
pub(crate) fn signed(bytes: &mut &[u8]) -> io::Result<i128> {
    let value = take(bytes)?;
    Ok((value >> 1) as i128 ^ -((value & 1) as i128))
}
pub(crate) fn encode_bucket(bytes: &mut Vec<u8>, bucket: Option<ChartBucket>, aligned: i64) {
    let Some(bucket) = bucket else {
        var(bytes, 0);
        return;
    };
    var(
        bytes,
        1 | (u128::from(bucket.first_break) << 1) | (u128::from(bucket.has_break) << 2),
    );
    for hour in [
        bucket.first_hour,
        bucket.last_hour,
        bucket.last_value_hour,
        bucket.min_hour,
        bucket.max_hour,
    ] {
        var(bytes, zig(i128::from(hour) - i128::from(aligned)));
    }
    var(bytes, bucket.absolute_total);
    let mut previous = 0;
    for value in [
        bucket.first_value,
        bucket.last_value,
        bucket.min_value,
        bucket.max_value,
    ] {
        var(bytes, u128::from(value.to_bits() ^ previous));
        previous = value.to_bits();
    }
}
pub(crate) fn decode_bucket(bytes: &mut &[u8], aligned: i64) -> io::Result<Option<ChartBucket>> {
    let flags = take(bytes)?;
    if flags == 0 {
        return Ok(None);
    }
    if flags > 7 {
        return Err(invalid("invalid summary flags"));
    }
    let mut hours = [0; 5];
    for hour in &mut hours {
        *hour = i32::try_from(i128::from(aligned) + signed(bytes)?)
            .map_err(|_| invalid("summary hour overflow"))?;
    }
    let absolute_total = take(bytes)?;
    let mut values = [0.0; 4];
    let mut previous = 0_u64;
    for value in &mut values {
        previous ^= u64::try_from(take(bytes)?).map_err(|_| invalid("summary value overflow"))?;
        *value = f64::from_bits(previous);
    }
    Ok(Some(ChartBucket {
        absolute_total,
        first_hour: hours[0],
        last_hour: hours[1],
        last_value_hour: hours[2],
        min_hour: hours[3],
        max_hour: hours[4],
        first_value: values[0],
        last_value: values[1],
        min_value: values[2],
        max_value: values[3],
        first_break: flags & 2 != 0,
        has_break: flags & 4 != 0,
    }))
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

#[cfg(test)]
mod tests {
    use super::{ChartBucket, ChartSummary};
    use serde_json::json;

    #[test]
    fn a_summary_sends_its_buckets_as_columns_and_reads_them_back_whole() {
        let later = ChartBucket {
            absolute_total: u128::MAX,
            last_hour: i32::MAX,
            max_hour: i32::MAX,
            first_break: true,
            has_break: true,
            ..ChartBucket::hour(i32::MAX - 9, -2.5, false)
        };
        let summary = ChartSummary {
            from: 0.0,
            to: 1.0,
            width: 3,
            buckets: vec![
                None,
                Some(ChartBucket::hour(i32::MIN, 7.0, true)),
                None,
                None,
                Some(later),
                None,
            ],
            runs: None,
            approximate: Vec::new(),
        };
        let wire = serde_json::to_value(&summary).unwrap();
        let lead = i64::from(i32::MAX - 9) - i64::from(i32::MIN);
        assert_eq!(
            wire["buckets"],
            json!({
                "slots": 6,
                "skip": [1, 2],
                "hours": [i32::MIN, 0, 0, 0, 0, lead, 9, 0, 0, 9],
                "values": [7.0, 7.0, 7.0, 7.0, -2.5, -2.5, -2.5, -2.5],
                "flags": [1, 3],
                "totals": ["7", u128::MAX.to_string()],
            })
        );
        assert_eq!(
            serde_json::from_value::<ChartSummary>(wire.clone()).unwrap(),
            summary
        );
        for (column, value) in [
            ("slots", json!(4)),
            ("skip", json!([1])),
            ("flags", json!([1, 4])),
            ("totals", json!(["7", "-1"])),
            ("hours", json!([i32::MIN, 0, 0, 0, 0, -1, 9, 0, 0, 9])),
        ] {
            let mut broken = wire.clone();
            broken["buckets"][column] = value;
            assert!(
                serde_json::from_value::<ChartSummary>(broken).is_err(),
                "{column}"
            );
        }
    }
}
