//! The shared flag test of the flag triggers (`has_country_flag`,
//! `has_mio_flag` and the others on the same reader): its operand as the
//! shared flag reader loads it, and its comparison with a flag record.
use crate::triggers::{Failure, refusal::refused};

use installed_content_core::parser::{Item, Operator};

/// A flag trigger's operand: a scalar flag name, or a block of `flag`
/// (lexical `none` when absent), `value`, `date` and `days`.
pub(super) struct FlagTest<'i> {
    /// The flag's name, quoted or bare, which the lexical registry interns
    /// without case either way.
    pub name: &'i [u8],
    value: Option<(Operator, i32)>,
    /// The hour of the date's day start.
    date: Option<(Operator, i32)>,
    days: Option<(Operator, i32)>,
}

impl<'i> FlagTest<'i> {
    /// Read the operand of `trigger`. A repeated or unknown field takes the
    /// reader's paths that are not reproduced, and a name built through a
    /// country or state (`flag@…`) is not reproduced: each is refused.
    pub fn read(item: &'i Item, trigger: &str) -> Result<Self, Failure> {
        let mut test = Self {
            name: b"none",
            value: None,
            date: None,
            days: None,
        };
        match item.value.items() {
            None => test.name = name(item, trigger)?,
            Some(block) => {
                let mut seen = Vec::new();
                for field in block {
                    let key = field
                        .keyword()
                        .map(|keyword| keyword.folded())
                        .unwrap_or_default();
                    if seen.contains(&key) {
                        return Err(refused(format!(
                            "{trigger} assigns {} more than once, and what the reader keeps is not reproduced",
                            String::from_utf8_lossy(&key)
                        )));
                    }
                    match key.as_slice() {
                        b"flag" => test.name = name(field, trigger)?,
                        b"value" => test.value = Some((field.operator, integer(field, trigger)?)),
                        b"date" => test.date = Some((field.operator, date(field, trigger)?)),
                        b"days" => test.days = Some((field.operator, integer(field, trigger)?)),
                        _ => {
                            return Err(refused(format!(
                                "{trigger} field {} takes the reader's error path, whose recovery is not reproduced",
                                String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                            )));
                        }
                    }
                    seen.push(key);
                }
            }
        }
        if test.name.contains(&b'@') {
            return Err(refused(format!(
                "{trigger} {} names its flag through a country or state, which is not reproduced",
                String::from_utf8_lossy(test.name)
            )));
        }
        Ok(test)
    }

    /// Whether the scope's first record of the flag, of this signed value
    /// and setting hour, passes: every given test must hold, and without
    /// `value` the value must be above zero. `value` compares by its
    /// operator; `date` compares the setting hour strictly; `days` compares
    /// the start of today's day (`now`, read only for it) with the start of
    /// the setting day plus that many days, strictly. `date =` and `days =`
    /// add no test. Expiry takes no part.
    pub fn holds(&self, value: i16, set_hour: i32, now: impl FnOnce() -> i32) -> bool {
        let value = i32::from(value);
        let mut holds = match self.value {
            None => value > 0,
            Some((Operator::Greater, wanted)) => value > wanted,
            Some((Operator::Less, wanted)) => value < wanted,
            Some((Operator::Equals, wanted)) => value == wanted,
        };
        holds &= match self.date {
            Some((Operator::Greater, at)) => set_hour > at,
            Some((Operator::Less, at)) => set_hour < at,
            Some((Operator::Equals, _)) | None => true,
        };
        if let Some((comparison @ (Operator::Greater | Operator::Less), count)) = self.days {
            let now = day_start(now());
            let threshold = day_start(set_hour).wrapping_add(count.wrapping_mul(24));
            holds &= match comparison {
                Operator::Greater => now > threshold,
                _ => now < threshold,
            };
        }
        holds
    }
}

/// A flag's name: its text to the first NUL.
fn name<'i>(item: &'i Item, trigger: &str) -> Result<&'i [u8], Failure> {
    item.value
        .scalar()
        .map(installed_content_core::parser::c_string)
        .ok_or_else(|| format!("{trigger} needs a scalar flag name").into())
}

/// Signed decimal conversion of a flag's `value` or `days`: decimal digits
/// after white space and a sign, any other text zero. Past the `i32` range
/// the conversion is not reproduced.
fn integer(field: &Item, trigger: &str) -> Result<i32, Failure> {
    let text = field.value.scalar().unwrap_or_default();
    let value = installed_content_core::fixed5::decimal_i32(text);
    if value == i32::MIN || value == i32::MAX {
        return Err(refused(format!(
            "{trigger} {} = {} reaches the end of the 32-bit range, whose conversion is not reproduced",
            String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()),
            String::from_utf8_lossy(text)
        )));
    }
    Ok(value)
}

/// A flag's `date`: year, month and day as decimal numbers, as the hour of
/// that day's start in the game's 365-day calendar. Any other form's
/// recovery is not reproduced.
fn date(field: &Item, trigger: &str) -> Result<i32, Failure> {
    let text = field.value.scalar().unwrap_or_default();
    let unported = || -> Failure {
        refused(format!(
            "{trigger} date {} is not a year.month.day date, and how the date reader recovers is not reproduced",
            String::from_utf8_lossy(text)
        ))
    };
    let parts: Vec<&[u8]> = installed_content_core::parser::c_string(text)
        .split(|byte| *byte == b'.')
        .collect();
    let [year, month, day] = parts.as_slice() else {
        return Err(unported());
    };
    let number = |part: &[u8]| -> Option<i64> {
        let digits = part.strip_prefix(b"-").unwrap_or(part);
        if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let value: i64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
        Some(if digits.len() < part.len() { -value } else { value })
    };
    let (Some(year), Some(month), Some(day)) = (number(year), number(month), number(day)) else {
        return Err(unported());
    };
    let lengths = engine_native::cdate::MONTH_LENGTHS;
    let Some(&length) = usize::try_from(month - 1).ok().and_then(|at| lengths.get(at)) else {
        return Err(unported());
    };
    if day < 1 || day > i64::from(length) {
        return Err(unported());
    }
    let before: i64 = lengths[..(month - 1) as usize]
        .iter()
        .map(|&days| i64::from(days))
        .sum();
    let hour = i64::from(engine_native::cdate::HOUR_EPOCH)
        + year * 365 * 24
        + (before + day - 1) * 24;
    i32::try_from(hour).map_err(|_| unported())
}

/// The start of an hour's day in the game's signed calendar arithmetic.
fn day_start(hour: i32) -> i32 {
    hour.wrapping_sub(hour.wrapping_sub(engine_native::cdate::HOUR_EPOCH) % 24)
}
