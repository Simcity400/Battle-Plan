use crate::triggers::{Failure, refusal::refused};

use content_reader_parser::parser::{Item, Operator};
use std::borrow::Cow;

use super::{Evaluator, Scope, Scopes, world::NULL_STATE};
use crate::scripted_text::EventObject;
use content_reader_script::scope::Keyword;

pub(super) struct FlagTest<'i> {
    pub name: &'i [u8],
    value: Option<(Operator, i32)>,
    date: Option<(Operator, i32)>,
    days: Option<(Operator, i32)>,
}

impl<'i> FlagTest<'i> {
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
        Ok(test)
    }

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

impl Evaluator<'_, '_> {
    pub(super) fn flag_name<'n>(
        &self,
        name: &'n [u8],
        scopes: Scopes,
    ) -> Result<Cow<'n, [u8]>, Failure> {
        let mut parts = name.split(|byte| *byte == b'@');
        let prefix = parts.next().unwrap();
        let Some(selector) = parts.next().filter(|_| !prefix.is_empty()) else {
            return Ok(Cow::Borrowed(name));
        };
        if parts.next().is_some() {
            return Ok(Cow::Borrowed(name));
        }
        let (country, state) = match selector.to_ascii_lowercase().as_slice() {
            b"this" | b"root" | b"prev" | b"from" => {
                let keyword = Keyword::named(selector).unwrap();
                let object = self.flag_scope(keyword, scopes)?;
                (self.flag_country(object)?, flag_members(object).1)
            }
            b"owner" | b"controller" | b"occupied" | b"capital" => {
                let here = self.flag_scope(Keyword::This, scopes)?;
                let (_, state) = flag_members(here);
                if selector.eq_ignore_ascii_case(b"capital") {
                    let owner = if state == NULL_STATE {
                        here
                    } else {
                        Some(EventObject::State(state))
                    };
                    let capital = match self.flag_country(owner)? {
                        Some(raw) => {
                            let country = self
                                .world()
                                .slot(raw)
                                .ok_or("dynamic flag capital owner has no country links row")?;
                            match self.enter_flat(
                                &content_reader_script::scope::Scope::Keyword(Keyword::CapitalScope),
                                Scopes::of(Scope::Country(country)),
                            )? {
                                Some(Scope::State(state)) => state,
                                None => NULL_STATE,
                                _ => {
                                    return Err(
                                        "dynamic flag capital did not resolve to a state".into()
                                    );
                                }
                            }
                        }
                        None => NULL_STATE,
                    };
                    (None, capital)
                } else if state == NULL_STATE {
                    (None, NULL_STATE)
                } else if selector.eq_ignore_ascii_case(b"occupied") {
                    return Err(refused(
                        "dynamic flag occupied needs the state's unrecorded occupier",
                    ));
                } else {
                    let (owner, controller) = self
                        .world()
                        .state_row(state)
                        .ok_or("dynamic flag selector has no States row")?;
                    let raw = if selector.eq_ignore_ascii_case(b"owner") {
                        owner
                    } else {
                        controller
                    };
                    (((raw as i32) > 0).then_some(raw), NULL_STATE)
                }
            }
            b"random" => (None, NULL_STATE),
            _ => {
                let Some(target) = selector.strip_prefix(b"event_target:") else {
                    return Ok(Cow::Borrowed(name));
                };
                self.saved_flag_scope(target, scopes)?
            }
        };
        let mut resolved = prefix.to_vec();
        if let Some(raw) = country {
            let slot = self
                .world()
                .slot(raw)
                .ok_or("dynamic flag country has no country links row")?;
            if self.world().raw_country(slot) != Some(raw) {
                return Err(refused(format!(
                    "dynamic flag raw country {raw} has no recorded tag spelling"
                )));
            }
            resolved.extend_from_slice(
                self.world()
                    .tag(slot)
                    .ok_or("dynamic flag country has no recorded tag")?,
            );
        } else if state != NULL_STATE {
            resolved.extend_from_slice((state as i32).to_string().as_bytes());
        } else {
            return Ok(Cow::Borrowed(b"none"));
        }
        Ok(Cow::Owned(resolved))
    }

    pub(super) fn flag_raw_members(
        &self,
        object: Option<EventObject>,
    ) -> Result<(Option<u32>, u32), Failure> {
        if matches!(object, Some(EventObject::Province(_))) {
            return Err(refused("dynamic flag province scope is not reproduced"));
        }
        let (country, state) = flag_members(object);
        let raw = country
            .map(|slot| {
                self.world()
                    .raw_country(slot)
                    .ok_or_else(|| Failure::from("dynamic flag country has no Countries row"))
            })
            .transpose()?;
        Ok((raw.filter(|raw| (*raw as i32) > 0), state))
    }

    fn flag_country(&self, object: Option<EventObject>) -> Result<Option<u32>, Failure> {
        let (country, state) = self.flag_raw_members(object)?;
        if country.is_some() {
            return Ok(country);
        }
        if state == NULL_STATE {
            return Ok(None);
        }
        let (owner, _) = self
            .world()
            .state_row(state)
            .ok_or("dynamic flag scope has no States row")?;
        Ok(((owner as i32) > 0).then_some(owner))
    }
}

fn flag_members(object: Option<EventObject>) -> (Option<u32>, u32) {
    match object {
        Some(EventObject::Country(country) | EventObject::CountryUnitLeader { country, .. }) => {
            (Some(country), NULL_STATE)
        }
        Some(EventObject::CountryState { country, state }) => (Some(country), state),
        Some(EventObject::State(state)) => (None, state),
        _ => (None, NULL_STATE),
    }
}

fn name<'i>(item: &'i Item, trigger: &str) -> Result<&'i [u8], Failure> {
    item.value
        .scalar()
        .map(content_reader_parser::parser::c_string)
        .ok_or_else(|| format!("{trigger} needs a scalar flag name").into())
}

fn integer(field: &Item, trigger: &str) -> Result<i32, Failure> {
    let text = field.value.scalar().unwrap_or_default();
    let value = content_reader_parser::fixed5::decimal_i32(text);
    if value == i32::MIN || value == i32::MAX {
        return Err(refused(format!(
            "{trigger} {} = {} reaches the end of the 32-bit range, whose conversion is not reproduced",
            String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()),
            String::from_utf8_lossy(text)
        )));
    }
    Ok(value)
}

fn date(field: &Item, trigger: &str) -> Result<i32, Failure> {
    let text = field.value.scalar().unwrap_or_default();
    let unported = || -> Failure {
        refused(format!(
            "{trigger} date {} is not a year.month.day date, and how the date reader recovers is not reproduced",
            String::from_utf8_lossy(text)
        ))
    };
    let parts: Vec<&[u8]> = content_reader_parser::parser::c_string(text)
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
        Some(if digits.len() < part.len() {
            -value
        } else {
            value
        })
    };
    let (Some(year), Some(month), Some(day)) = (number(year), number(month), number(day)) else {
        return Err(unported());
    };
    let lengths = game_model::calendar::MONTH_LENGTHS;
    let Some(&length) = usize::try_from(month - 1)
        .ok()
        .and_then(|at| lengths.get(at))
    else {
        return Err(unported());
    };
    if day < 1 || day > i64::from(length) {
        return Err(unported());
    }
    let before: i64 = lengths[..(month - 1) as usize]
        .iter()
        .map(|&days| i64::from(days))
        .sum();
    let hour =
        i64::from(game_model::calendar::HOUR_EPOCH) + year * 365 * 24 + (before + day - 1) * 24;
    i32::try_from(hour).map_err(|_| unported())
}

fn day_start(hour: i32) -> i32 {
    hour.wrapping_sub(hour.wrapping_sub(game_model::calendar::HOUR_EPOCH) % 24)
}
