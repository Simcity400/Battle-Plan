//! Land-model camouflage from recorded acclimatization and matching content.
//!
//! `140607DD0` selects the first strictly greatest positive category weight,
//! compares it with maximum * `NGraphics.ACCLIMATIZATION_CAMO_SHOW_AT`, rejects
//! the current province's `forbid_camo_when`, then `140607900` evaluates the
//! ordered `change_camo_when` conditions against the current province and its
//! state's share (`ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE`). `141918710` tries
//! the resulting `<entity>_<condition>` and keeps the base if it is absent.
//!
//! A condition is literal `snow` (`1406087E0`: effective snow positive, the
//! executable's own keyword) or a terrain name (`140608930`, compared with the
//! province's authored terrain). Effective snow is `140F05480`'s
//! `min(permanent + current, snow_gain_max)`, all three recorded.

use std::{collections::BTreeMap, io};

use installed_content::{ResolvedLayers, defines::Defines, parser};
use replay::{
    frame::DecodedFrame,
    rows::{DivisionStatusRow, tables},
};

use super::tables::{Available, core, rows};
use crate::table_access::{TokenKeys, TokenNames};

/// The only condition that is not a terrain name.
const SNOW: &str = "snow";

#[derive(Debug, Default)]
struct Category {
    change: Vec<String>,
    forbid: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Content {
    categories: BTreeMap<String, Category>,
    /// The category keys by the lexer's token identity.
    category_tokens: TokenKeys,
    show_at: i64,
    show_in_state: i64,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

impl Content {
    pub fn load(layers: &ResolvedLayers, defines: &Defines) -> io::Result<Self> {
        let path = layers
            .paths()
            .find(|path| path.eq_ignore_ascii_case("common/acclimatation.txt"))
            .ok_or_else(|| invalid("missing matching common/acclimatation.txt"))?;
        Self::parse(
            &layers.read(path)?,
            defines.fixed5("NGraphics", "ACCLIMATIZATION_CAMO_SHOW_AT")?,
            defines.fixed5("NGraphics", "ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE")?,
        )
    }

    /// Each set of categories equal under the lexer's fold, which a
    /// recorded status names at once and so refuses.
    pub fn refusals(&self) -> impl Iterator<Item = String> + '_ {
        self.category_tokens.ambiguous().map(|keys| {
            format!(
                "acclimatization categories {} are one token under the lexer's fold",
                keys.join(", ")
            )
        })
    }

    pub(crate) fn parse(bytes: &[u8], show_at: i64, show_in_state: i64) -> io::Result<Self> {
        let mut categories = BTreeMap::new();
        for item in parser::parse(bytes)?.items {
            let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items()) else {
                return Err(invalid("invalid authored acclimatization category"));
            };
            let key = std::str::from_utf8(key)
                .map_err(|_| invalid("non-UTF-8 acclimatization category"))?;
            let mut category = Category::default();
            for field in fields {
                let target = match field.key.as_deref() {
                    Some(b"change_camo_when") => &mut category.change,
                    Some(b"forbid_camo_when") => &mut category.forbid,
                    _ => continue,
                };
                for entry in field
                    .value
                    .items()
                    .ok_or_else(|| invalid("acclimatization camouflage condition is not a list"))?
                {
                    let condition = entry
                        .value
                        .scalar_str()
                        .filter(|_| entry.key.is_none())
                        .ok_or_else(|| invalid("invalid acclimatization camouflage condition"))?;
                    target.push(condition.to_owned());
                }
            }
            if categories.insert(key.to_owned(), category).is_some() {
                return Err(invalid(format!("duplicate acclimatization category {key}")));
            }
        }
        Ok(Self {
            category_tokens: TokenKeys::new(categories.keys()),
            categories,
            show_at,
            show_in_state,
        })
    }

    /// The suffix for one division at one province. `state_provinces` holds
    /// every member of the province's state, the province included; empty when
    /// it has no state. `matches` decides one condition at one province.
    fn suffix<'a>(
        &'a self,
        status: &DivisionStatusRow,
        tokens: TokenNames<'_>,
        province: u32,
        state_provinces: &[u32],
        mut matches: impl FnMut(u32, &str) -> io::Result<bool>,
    ) -> io::Result<Option<&'a str>> {
        let mut winner = None;
        let mut highest = 0;
        for entry in &status.acclimatization {
            if entry.weight > highest {
                highest = entry.weight;
                winner = Some(entry.category);
            }
        }
        let Some(token) = winner else {
            return Ok(None);
        };
        let maximum = status
            .acclimatization_maximum
            .ok_or_else(|| invalid("division acclimatization is unavailable this hour"))?;
        if highest < maximum.wrapping_mul(self.show_at) / 100_000 {
            return Ok(None);
        }
        // The division records the category by token: its spelling names the
        // authored category equal to it under the lexer's fold.
        let spelling = tokens
            .name(token)
            .map_err(|reason| invalid(format!("acclimatization category: {reason}")))?;
        let key = self
            .category_tokens
            .resolve(spelling)
            .map_err(invalid)?
            .ok_or_else(|| {
                invalid(format!(
                    "missing authored acclimatization category {spelling}"
                ))
            })?;
        let category = &self.categories[key];
        for condition in &category.forbid {
            if matches(province, condition)? {
                return Ok(None);
            }
        }
        for condition in &category.change {
            if matches(province, condition)? {
                return Ok(Some(condition));
            }
            if !state_provinces.is_empty() {
                let mut matching = 0_i64;
                for member in state_provinces {
                    if matches(*member, condition)? {
                        matching += 1;
                    }
                }
                let count = i32::try_from(state_provinces.len())
                    .map_err(|_| invalid("state province count exceeds native i32"))?;
                let share = matching.wrapping_mul(10_000_000_000) / (i64::from(count) * 100_000);
                if share >= self.show_in_state {
                    return Ok(Some(condition));
                }
            }
        }
        Ok(None)
    }
}

/// `141918710` does not replace a valid base with a nonexistent seasonal model.
pub fn entity(base: &str, suffix: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    if let Some(suffix) = suffix.filter(|suffix| !suffix.is_empty()) {
        let candidate = format!("{base}_{suffix}");
        if exists(&candidate) {
            return candidate;
        }
    }
    base.to_owned()
}

/// One hour's camouflage operands. The division's own status row comes from
/// the caller, which already keys the hour's statuses.
pub struct Context<'a> {
    provinces: BTreeMap<u32, u32>,
    state_members: BTreeMap<u32, Vec<u32>>,
    /// The dense prefix of the native weather array and the native count,
    /// which bounds the getter even when the capture was truncated.
    /// Province id to (current, permanent) snow words.
    weather_snow: BTreeMap<u32, (i64, i64)>,
    /// `snow_gain_max`; `None` without a weather region row.
    ceiling: Option<i64>,
    weather_count: Option<usize>,
    terrain: &'a BTreeMap<u32, String>,
}

impl<'a> Context<'a> {
    /// `terrain` is each province's authored terrain, column seven of the
    /// matching province definitions.
    pub fn read(frame: &'a DecodedFrame, terrain: &'a BTreeMap<u32, String>) -> io::Result<Self> {
        let mut provinces = BTreeMap::new();
        let mut state_members: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for row in core::<tables::Provinces>(frame)? {
            provinces.insert(row.id, row.state);
            if row.state != 0 {
                state_members.entry(row.state).or_default().push(row.id);
            }
        }
        let weather = rows::<tables::WeatherProvinces>(frame)?;
        let weather_count = match &weather {
            Available::Complete(rows) => Some(rows.len()),
            Available::Truncated { rows, missing } => Some(
                rows.len()
                    .checked_add(*missing as usize)
                    .ok_or_else(|| invalid("weather province count overflows"))?,
            ),
            Available::Unavailable { .. } => None,
        };
        let mut weather_snow = BTreeMap::new();
        for (slot, row) in weather.rows().iter().enumerate() {
            if usize::try_from(row.id).ok() != Some(slot) {
                return Err(invalid(format!(
                    "weather province {slot} differs from its dense native array slot"
                )));
            }
            weather_snow.insert(row.id, (row.snow, row.permanent_snow));
        }
        Ok(Self {
            provinces,
            state_members,
            weather_snow,
            ceiling: rows::<tables::WeatherRegions>(frame)?
                .rows()
                .first()
                .map(|row| row.max_snow),
            weather_count,
            terrain,
        })
    }

    /// The suffix for one division at its drawn province. A missing operand
    /// is an error, never a false test.
    pub fn suffix<'b>(
        &self,
        content: &'b Content,
        tokens: TokenNames<'_>,
        status: &DivisionStatusRow,
        province: u32,
    ) -> io::Result<Option<&'b str>> {
        let state = *self.provinces.get(&province).ok_or_else(|| {
            invalid(format!(
                "division {} has unknown province {province}",
                status.division
            ))
        })?;
        let members = self
            .state_members
            .get(&state)
            .map_or(&[][..], Vec::as_slice);
        content.suffix(status, tokens, province, members, |id, condition| {
            self.matches(id, condition)
        })
    }

    fn matches(&self, province: u32, condition: &str) -> io::Result<bool> {
        if !self.provinces.contains_key(&province) {
            return Err(invalid(format!(
                "camouflage references unknown province {province}"
            )));
        }
        if condition == SNOW {
            let Some((current, permanent)) = self.weather_snow.get(&province).copied() else {
                // `140F04D50` returns null before touching either snow word
                // when this id lies beyond CWeatherManager+0x1C.
                if self
                    .weather_count
                    .is_some_and(|count| province as usize >= count)
                {
                    return Ok(false);
                }
                return Err(invalid(format!(
                    "province {province} has no captured weather snow operand"
                )));
            };
            let ceiling = self
                .ceiling
                .ok_or_else(|| invalid("the snow ceiling is not recorded this hour"))?;
            return Ok(permanent.wrapping_add(current).min(ceiling) > 0);
        }
        let terrain = self.terrain.get(&province).ok_or_else(|| {
            invalid(format!(
                "province {province} has no authored terrain definition"
            ))
        })?;
        Ok(terrain == condition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::DivisionAcclimatization;

    fn content() -> Content {
        Content::parse(
            br#"
            cold = { change_camo_when = { snow } }
            hot = { change_camo_when = { desert } forbid_camo_when = { jungle } }
        "#,
            75_000,
            50_000,
        )
        .unwrap()
    }

    fn status(entries: &[(u32, i64)], maximum: i64) -> DivisionStatusRow {
        DivisionStatusRow {
            acclimatization_maximum: Some(maximum),
            acclimatization: entries
                .iter()
                .map(|&(category, weight)| DivisionAcclimatization { category, weight })
                .collect(),
            ..Default::default()
        }
    }

    fn tokens() -> TokenNames<'static> {
        TokenNames::from_rows(
            Box::leak(Box::new([
                replay::rows::LexicalTokenRow {
                    token: 1,
                    literal: b"cold".to_vec(),
                },
                replay::rows::LexicalTokenRow {
                    token: 2,
                    literal: b"hot".to_vec(),
                },
                // A spelling that differs from its authored key in ASCII case.
                replay::rows::LexicalTokenRow {
                    token: 3,
                    literal: b"COLD".to_vec(),
                },
            ])),
            true,
        )
    }

    #[test]
    fn a_category_token_names_its_authored_key_under_the_lexers_fold() {
        let content = content();
        let snowy = |_: u32, condition: &str| Ok(condition == SNOW);
        assert_eq!(
            content
                .suffix(&status(&[(3, 90)], 100), tokens(), 1, &[], snowy)
                .unwrap(),
            Some(SNOW)
        );
        assert_eq!(
            content
                .suffix(&status(&[(9, 90)], 100), tokens(), 1, &[], snowy)
                .unwrap_err()
                .to_string(),
            "acclimatization category: token 9 is not in LexicalTokens"
        );
        // Two authored categories equal under the fold are refused by name.
        let ambiguous = Content::parse(
            br#"
            cold = { change_camo_when = { snow } }
            Cold = { change_camo_when = { snow } }
        "#,
            75_000,
            50_000,
        )
        .unwrap();
        assert_eq!(
            ambiguous
                .suffix(&status(&[(3, 90)], 100), tokens(), 1, &[], snowy)
                .unwrap_err()
                .to_string(),
            "token COLD names several content keys: Cold, cold"
        );
    }

    #[test]
    fn the_first_strict_maximum_must_reach_the_show_threshold() {
        let content = content();
        let snowy = |_: u32, condition: &str| Ok(condition == SNOW);
        // Ties keep the first; 75% of the maximum is the threshold.
        let row = status(&[(1, 75), (2, 75)], 100);
        assert_eq!(
            content.suffix(&row, tokens(), 1, &[], snowy).unwrap(),
            Some(SNOW)
        );
        let low = status(&[(1, 74)], 100);
        assert_eq!(content.suffix(&low, tokens(), 1, &[], snowy).unwrap(), None);
        assert_eq!(
            content
                .suffix(&status(&[], 100), tokens(), 1, &[], snowy)
                .unwrap(),
            None
        );
        let unknown = status(&[(9, 90)], 100);
        assert!(content.suffix(&unknown, tokens(), 1, &[], snowy).is_err());
        // An unreadable acclimatization is never a false test.
        let unread = DivisionStatusRow {
            acclimatization_maximum: None,
            ..status(&[(1, 90)], 100)
        };
        assert!(content.suffix(&unread, tokens(), 1, &[], snowy).is_err());
    }

    #[test]
    fn forbidden_terrain_wins_and_the_state_share_can_switch_the_suffix() {
        let content = content();
        let row = status(&[(2, 90)], 100);
        let terrain = |id: u32, condition: &str| {
            Ok(match id {
                1 => condition == "jungle",
                2 | 3 => condition == "desert",
                _ => false,
            })
        };
        assert_eq!(
            content.suffix(&row, tokens(), 1, &[1, 2], terrain).unwrap(),
            None
        );
        // Province 4 is not desert, but half its state is: 50% >= 50%.
        assert_eq!(
            content.suffix(&row, tokens(), 4, &[4, 2], terrain).unwrap(),
            Some("desert")
        );
        assert_eq!(
            content
                .suffix(&row, tokens(), 4, &[4, 5, 2], terrain)
                .unwrap(),
            None
        );
        assert_eq!(
            entity("GER_infantry_entity", Some("desert"), |_| false),
            "GER_infantry_entity"
        );
        assert_eq!(
            entity("GER_infantry_entity", Some("desert"), |_| true),
            "GER_infantry_entity_desert"
        );
    }
}
