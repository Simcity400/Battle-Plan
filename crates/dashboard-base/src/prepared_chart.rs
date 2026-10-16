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
    pub buckets: Vec<Option<ChartBucket>>,
    /// Original supplying runs for hourly bins, including their unclipped bounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runs: Option<Vec<ChartRun>>,
    /// Display-only keyframe estimates; these hours are not exact values.
    #[serde(default)]
    pub approximate: Vec<crate::products::country_groups::HourSpan>,
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
