
use std::{collections::BTreeMap, io, sync::OnceLock};

use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_parser::parser;
use replay_data::{
    rows::ProvinceRow, rows::Recorded, rows::TableId, rows::WeatherProvinceRow, rows::tables,
};
use replay_frame::frame::{DecodedFrame, kept};

use crate::unit_models::ModelStatus;

use viewer_foundation::tables::{Available, core, rows, rows_fields};
use dashboard_text::table_access::{TokenKeys, TokenNames};

const SNOW: &str = "snow";

#[derive(Debug, Default)]
struct Category {
    change: Vec<String>,
    forbid: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Content {
    categories: BTreeMap<String, Category>,
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
            .paths()?
            .find(|path| path.eq_ignore_ascii_case(b"common/acclimatation.txt"))
            .ok_or_else(|| invalid("missing matching common/acclimatation.txt"))?;
        Self::parse(
            &layers.read_path(path)?,
            defines.fixed5("NGraphics", "ACCLIMATIZATION_CAMO_SHOW_AT")?,
            defines.fixed5("NGraphics", "ACCLIMATIZATION_CAMO_SHOW_WHEN_IN_STATE")?,
        )
    }

    pub fn refusals(&self) -> impl Iterator<Item = String> + '_ {
        self.category_tokens.ambiguous().map(|keys| {
            format!(
                "acclimatization categories {} are one token under the lexer's fold",
                keys.join(", ")
            )
        })
    }

    pub fn parse(bytes: &[u8], show_at: i64, show_in_state: i64) -> io::Result<Self> {
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

    fn category(&self, status: ModelStatus<'_>) -> io::Result<Option<u32>> {
        let status = match status {
            ModelStatus::Status(status) => status,
            ModelStatus::Witness(row) => {
                return row
                    .camouflage
                    .map(|camouflage| camouflage.category)
                    .ok_or_else(|| invalid("division camouflage is unavailable this hour"));
            }
        };
        match &status.camouflage {
            Recorded::Value(Some(camouflage)) => Ok(camouflage.category),
            Recorded::Value(None) => Err(invalid("division camouflage is unavailable this hour")),
            Recorded::NotRecorded => {
                let maximum = status
                    .acclimatization_maximum
                    .ok_or_else(|| invalid("division acclimatization is unavailable this hour"))?;
                Ok(game_model::acclimatization::camouflage(
                    status
                        .acclimatization
                        .iter()
                        .map(|entry| (entry.category, entry.weight)),
                    maximum,
                    self.show_at,
                ))
            }
        }
    }

    fn suffix<'a, 's>(
        &'a self,
        status: impl Into<ModelStatus<'s>>,
        tokens: TokenNames<'_>,
        province: u32,
        state_provinces: &[u32],
        mut matches: impl FnMut(u32, &str) -> io::Result<bool>,
    ) -> io::Result<Option<&'a str>> {
        let Some(token) = self.category(status.into())? else {
            return Ok(None);
        };
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

pub fn entity(base: &str, suffix: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    if let Some(suffix) = suffix.filter(|suffix| !suffix.is_empty()) {
        let candidate = format!("{base}_{suffix}");
        if exists(&candidate) {
            return candidate;
        }
    }
    base.to_owned()
}

pub(crate) struct Fields {
    pub state: u64,
    pub snow: u64,
    pub ceiling: u64,
    pub status: u64,
    pub witness: u64,
}

pub(crate) fn fields() -> &'static Fields {
    fn mask(table: TableId, names: &[&str]) -> u64 {
        let mask = (0..table.schema().len())
            .filter(|&field| {
                table
                    .field_name(field)
                    .is_some_and(|name| names.contains(&name))
            })
            .fold(0, |mask, field| mask | (1_u64 << field));
        assert_eq!(
            mask.count_ones() as usize,
            names.len(),
            "missing camouflage operand column of {table:?}"
        );
        mask
    }
    static FIELDS: OnceLock<Fields> = OnceLock::new();
    FIELDS.get_or_init(|| Fields {
        state: mask(TableId::Provinces, &["state"]),
        snow: mask(TableId::WeatherProvinces, &["snow", "permanent_snow"]),
        ceiling: mask(TableId::WeatherRegions, &["max_snow"]),
        status: mask(
            TableId::DivisionStatus,
            &["camouflage", "acclimatization", "acclimatization_maximum"],
        ),
        witness: mask(TableId::DivisionModelInputs, &["camouflage"]),
    })
}

#[derive(Debug, Default)]
pub struct StateMembers(BTreeMap<u32, Vec<u32>>);

impl StateMembers {
    pub fn read(provinces: &[ProvinceRow]) -> Self {
        kept::note_table(TableId::Provinces, fields().state);
        let mut members: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for row in provinces.iter().filter(|row| row.state != 0) {
            members.entry(row.state).or_default().push(row.id);
        }
        Self(members)
    }

    fn of(&self, state: u32) -> &[u32] {
        self.0.get(&state).map_or(&[], Vec::as_slice)
    }
}

pub struct Context<'a> {
    provinces: &'a [ProvinceRow],
    weather: &'a [WeatherProvinceRow],
    ceiling: Option<i64>,
    weather_count: Option<usize>,
    terrain: &'a BTreeMap<u32, String>,
}

impl<'a> Context<'a> {
    pub fn none(terrain: &'a BTreeMap<u32, String>) -> Self {
        Self { provinces: &[], weather: &[], ceiling: None, weather_count: None, terrain }
    }

    pub fn read(frame: &'a DecodedFrame, terrain: &'a BTreeMap<u32, String>) -> io::Result<Self> {
        let provinces = core::<tables::Provinces>(frame)?;
        let weather = rows_fields::<tables::WeatherProvinces>(frame, fields().snow)?;
        let weather_count = match &weather {
            Available::Complete(rows) => Some(rows.len()),
            Available::Truncated { rows, missing } => Some(
                rows.len()
                    .checked_add(*missing as usize)
                    .ok_or_else(|| invalid("weather province count overflows"))?,
            ),
            Available::Unavailable { .. } | Available::NotRecorded { .. } => None,
        };
        let weather = weather.rows();
        if let Some(slot) = weather
            .iter()
            .enumerate()
            .position(|(slot, row)| usize::try_from(row.id).ok() != Some(slot))
        {
            return Err(invalid(format!(
                "weather province {slot} differs from its dense native array slot"
            )));
        }
        Ok(Self {
            provinces,
            weather,
            ceiling: rows::<tables::WeatherRegions>(frame)?
                .rows()
                .first()
                .map(|row| row.max_snow),
            weather_count,
            terrain,
        })
    }

    pub fn provinces(&self) -> &'a [ProvinceRow] {
        self.provinces
    }

    fn province(&self, province: u32) -> Option<&'a ProvinceRow> {
        self.provinces
            .binary_search_by_key(&province, |row| row.id)
            .ok()
            .map(|at| &self.provinces[at])
    }

    pub fn suffix<'b>(
        &self,
        content: &'b Content,
        tokens: TokenNames<'_>,
        status: ModelStatus<'_>,
        province: u32,
        members: &StateMembers,
    ) -> io::Result<Option<&'b str>> {
        kept::note_table(TableId::Provinces, fields().state);
        let state = self.province(province).map(|row| row.state).ok_or_else(|| {
            invalid(format!(
                "division {} has unknown province {province}",
                status.division()
            ))
        })?;
        content.suffix(status, tokens, province, members.of(state), |id, condition| {
            self.matches(id, condition)
        })
    }

    fn matches(&self, province: u32, condition: &str) -> io::Result<bool> {
        if self.province(province).is_none() {
            return Err(invalid(format!(
                "camouflage references unknown province {province}"
            )));
        }
        if condition == SNOW {
            let Some(row) = self.weather.get(province as usize) else {
                kept::note_table(TableId::WeatherProvinces, u64::MAX);
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
            kept::note_row(TableId::WeatherProvinces, u128::from(province), fields().snow);
            kept::note_table(TableId::WeatherRegions, fields().ceiling);
            let ceiling = self
                .ceiling
                .ok_or_else(|| invalid("the snow ceiling is not recorded this hour"))?;
            return Ok(row.permanent_snow.wrapping_add(row.snow).min(ceiling) > 0);
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
    use replay_data::{
        rows::DivisionAcclimatization, rows::DivisionCamouflage, rows::DivisionStatusRow,
    };

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
            camouflage: Recorded::NotRecorded,
            ..Default::default()
        }
    }

    fn recorded(category: Option<u32>) -> DivisionStatusRow {
        DivisionStatusRow {
            camouflage: Recorded::Value(Some(DivisionCamouflage { category })),
            ..Default::default()
        }
    }

    fn tokens() -> TokenNames<'static> {
        TokenNames::from_rows(
            Box::leak(Box::new([
                replay_data::rows::LexicalTokenRow {
                    token: 1,
                    literal: b"cold".to_vec(),
                },
                replay_data::rows::LexicalTokenRow {
                    token: 2,
                    literal: b"hot".to_vec(),
                },
                replay_data::rows::LexicalTokenRow {
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
                .suffix(&recorded(Some(3)), tokens(), 1, &[], snowy)
                .unwrap(),
            Some(SNOW)
        );
        assert_eq!(
            content
                .suffix(&status(&[(9, 90)], 100), tokens(), 1, &[], snowy)
                .unwrap_err()
                .to_string(),
            "acclimatization category: token 9 is not in lexical tokens"
        );
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
        for unread in [
            DivisionStatusRow {
                acclimatization_maximum: None,
                ..status(&[(1, 90)], 100)
            },
            DivisionStatusRow {
                acclimatization_maximum: None,
                ..status(&[], 100)
            },
            DivisionStatusRow {
                camouflage: Recorded::Value(None),
                ..Default::default()
            },
        ] {
            assert!(content.suffix(&unread, tokens(), 1, &[], snowy).is_err());
        }
        assert_eq!(
            content
                .suffix(&recorded(None), tokens(), 1, &[], snowy)
                .unwrap(),
            None
        );
        let dry = |_: u32, _: &str| Ok(false);
        assert_eq!(
            content
                .suffix(&recorded(Some(1)), tokens(), 1, &[], dry)
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_recorded_camouflage_draws_what_the_entries_drew() {
        let content = content();
        let terrain = |id: u32, condition: &str| {
            Ok(match id {
                1 => condition == SNOW,
                2 => condition == "desert",
                _ => condition == "jungle",
            })
        };
        let weights = [0_i64, 1, -1, 74, 75, 76, 90, 100, i64::MAX];
        let mut compared = 0;
        for maximum in [0_i64, 100, -100, i64::MAX] {
            for first in weights {
                for second in weights {
                    for (categories, province) in [((1, 2), 1), ((2, 1), 2), ((2, 2), 3)] {
                        let entries = [(categories.0, first), (categories.1, second)];
                        let old = status(&entries, maximum);
                        let current = recorded(game_model::acclimatization::camouflage(
                            entries,
                            maximum,
                            content.show_at,
                        ));
                        assert_eq!(
                            content
                                .suffix(&old, tokens(), province, &[], terrain)
                                .unwrap(),
                            content
                                .suffix(&current, tokens(), province, &[], terrain)
                                .unwrap(),
                            "{entries:?} maximum {maximum}"
                        );
                        compared += 1;
                    }
                }
            }
        }
        assert_eq!(compared, 4 * 9 * 9 * 3);
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
