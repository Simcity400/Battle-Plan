//! Exact hourly fixed5 histories. Only changes occupy sparse delta pairs;
//! equal rereads extend the observed span without additional payload.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

const SCALE: i64 = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct NativeFixed5Run {
    pub first_hour: i32,
    pub last_hour: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw: Option<String>,
    /// Space-separated `hours_since_change:delta` pairs, first from first_hour.
    /// Decimal deltas multiply by delta_scale (fixed5 scale when omitted);
    /// `r` prefixes an unscaled raw delta. Gaps are positive, deltas nonzero.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deltas: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub delta_scale: Option<String>,
    /// Product-owned shared reason index, never an implicit zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub missing: Option<usize>,
    pub read_each_hour: bool,
}

impl NativeFixed5Run {
    #[cfg(any(test, feature = "test-support"))]
    pub fn value_at(&self, hour: i32) -> Option<i64> {
        if hour < self.first_hour || hour > self.last_hour {
            return None;
        }
        let mut raw = self.raw.as_ref()?.parse::<i64>().ok()?;
        if let Some(deltas) = &self.deltas {
            let mut changed_at = i64::from(self.first_hour);
            for pair in deltas.split_whitespace() {
                let (gap, token) = pair.split_once(':')?;
                changed_at = changed_at.checked_add(gap.parse::<i64>().ok()?)?;
                if changed_at > i64::from(hour) {
                    break;
                }
                let delta = match token.strip_prefix('r') {
                    Some(raw) => raw.parse::<i64>().ok()?,
                    None => token.parse::<i64>().ok()?.wrapping_mul(
                        self.delta_scale
                            .as_ref()
                            .map_or(Some(SCALE), |scale| scale.parse().ok())?,
                    ),
                };
                raw = raw.wrapping_add(delta);
            }
        }
        Some(raw)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fixed5State {
    raw: Option<i64>,
    changed_at: i32,
}

/// Append an observed read without rescanning the existing changes.
pub fn push_fixed5(
    runs: &mut Vec<NativeFixed5Run>,
    state: &mut Fixed5State,
    hour: i32,
    raw: Option<i64>,
    missing: Option<usize>,
    read_each_hour: bool,
    break_before: bool,
) -> usize {
    if let Some(last) = runs.last_mut()
        && !break_before
        && last.last_hour.checked_add(1) == Some(hour)
        && last.read_each_hour == read_each_hour
        && last.missing == missing
        && last.raw.is_some() == raw.is_some()
    {
        let mut added = 0;
        if let (Some(before), Some(now)) = (state.raw, raw) {
            let delta = now.wrapping_sub(before);
            if delta != 0 {
                if last.deltas.is_none() {
                    let scale = delta.checked_abs().unwrap_or(1);
                    if scale != SCALE {
                        last.delta_scale = Some(scale.to_string());
                    }
                }
                let scale = last
                    .delta_scale
                    .as_ref()
                    .and_then(|scale| scale.parse::<i64>().ok())
                    .unwrap_or(SCALE);
                let deltas = last.deltas.get_or_insert_with(String::new);
                if !deltas.is_empty() {
                    deltas.push(' ');
                    added += 1;
                }
                let token = if delta % scale == 0 {
                    (delta / scale).to_string()
                } else {
                    format!("r{delta}")
                };
                let pair = format!("{}:{token}", i64::from(hour) - i64::from(state.changed_at));
                added += pair.len();
                deltas.push_str(&pair);
                state.changed_at = hour;
            }
        }
        last.last_hour = hour;
        state.raw = raw;
        return added;
    }
    let encoded = raw.map(|value| value.to_string());
    let added = encoded.as_ref().map_or(0, String::len);
    runs.push(NativeFixed5Run {
        first_hour: hour,
        last_hour: hour,
        raw: encoded,
        deltas: None,
        delta_scale: None,
        missing,
        read_each_hour,
    });
    *state = Fixed5State {
        raw,
        changed_at: hour,
    };
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_score_fixed5_deltas_preserve_wrap_fraction_and_equal_prefix() {
        let mut runs = Vec::new();
        let mut last = Fixed5State::default();
        for (hour, raw) in [
            (1, i64::MAX),
            (2, i64::MAX),
            (3, i64::MIN),
            (4, i64::MIN + SCALE),
        ] {
            push_fixed5(&mut runs, &mut last, hour, Some(raw), None, true, false);
        }
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].deltas.as_deref(), Some("2:1 1:100000"));
        assert_eq!(runs[0].delta_scale.as_deref(), Some("1"));
        assert_eq!(runs[0].value_at(2), Some(i64::MAX));
        assert_eq!(runs[0].value_at(3), Some(i64::MIN));
        assert_eq!(runs[0].value_at(4), Some(i64::MIN + SCALE));
        assert_eq!(runs[0].value_at(5), None);
    }

    #[test]
    fn game_score_fixed5_missing_and_gaps_split_without_null_payloads() {
        let mut runs = Vec::new();
        let mut last = Fixed5State::default();
        for (hour, raw, missing) in [
            (1, Some(0), None),
            (2, None, Some(0)),
            (3, None, Some(0)),
            (5, Some(0), None),
        ] {
            push_fixed5(&mut runs, &mut last, hour, raw, missing, true, false);
        }
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].last_hour, 3);
        assert_eq!(runs[1].value_at(2), None);
        let json = serde_json::to_string(&runs[1]).unwrap();
        assert!(json.contains("\"missing\":0"));
        assert!(!json.contains("null"));
    }

    #[test]
    fn game_score_fixed5_equal_rereads_add_no_payload_after_a_change() {
        let mut runs = Vec::new();
        let mut state = Fixed5State::default();
        push_fixed5(&mut runs, &mut state, 0, Some(0), None, true, false);
        push_fixed5(&mut runs, &mut state, 1, Some(SCALE), None, true, false);
        assert_eq!(
            push_fixed5(&mut runs, &mut state, 2, Some(SCALE), None, true, false),
            0
        );
        assert_eq!(runs[0].deltas.as_deref(), Some("1:1"));
        assert_eq!(runs[0].last_hour, 2);
        push_fixed5(&mut runs, &mut state, 3, Some(SCALE + 7), None, true, false);
        assert_eq!(runs[0].deltas.as_deref(), Some("1:1 2:r7"));
        assert_eq!(runs[0].value_at(2), Some(SCALE));
        assert_eq!(runs[0].value_at(3), Some(SCALE + 7));
    }
}
