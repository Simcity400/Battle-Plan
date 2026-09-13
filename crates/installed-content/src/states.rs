//! State definitions from `history/states/*.txt`.
//!
//! Only the authored start state is read. Dated blocks such as `1939.1.1 = {}`
//! and conditional `if` blocks are campaign history the replay already carries,
//! so they are listed by key and never applied.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{
        all, block, boolean, integer, invalid, key_str, last, number, require, scalar_list, text,
    },
    parser::{self, Value},
};

/// One building entry, either `name = level` or `name = { level = N ... }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingLevel {
    pub level: u32,
    /// True when the file guards the building with conditions such as
    /// `allowed = { has_dlc = ... }`, which this reader does not evaluate.
    pub conditional: bool,
}

/// One state as `history/states` authors it at the campaign start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateDefinition {
    pub id: u32,
    /// Localisation key, normally `STATE_<id>`.
    pub name_key: String,
    pub category: Option<String>,
    pub impassable: bool,
    pub manpower: Option<u64>,
    pub provinces: Vec<u32>,
    /// Authored as decimals, which f64 holds exactly at these magnitudes.
    pub resources: BTreeMap<String, f64>,
    /// `(province, value)` pairs in file order; a state may list several.
    /// Three shipped states write the value as `30.0`, so the decimal the file
    /// holds is kept and rounding is left to display.
    pub victory_points: Vec<(u32, f64)>,
    /// State-wide levels from `history.buildings`.
    pub buildings: BTreeMap<String, BuildingLevel>,
    /// Province-scoped levels from the numeric keys of the same block.
    pub province_buildings: BTreeMap<u32, BTreeMap<String, BuildingLevel>>,
    /// Keys of the dated and conditional history blocks this reader skipped.
    pub skipped_history: Vec<String>,
}

fn building_level(value: &Value, what: &str) -> io::Result<BuildingLevel> {
    match value {
        Value::Scalar(_) => Ok(BuildingLevel {
            level: integer(value, what)?,
            conditional: false,
        }),
        _ => Ok(BuildingLevel {
            level: integer(require(block(value, what)?, "level", what)?, what)?,
            conditional: true,
        }),
    }
}

fn read_buildings(value: &Value, state: &mut StateDefinition) -> io::Result<()> {
    for item in block(value, "state buildings")? {
        let key = key_str(item, "state buildings")?;
        if let Ok(province) = key.parse::<u32>() {
            let entry = state.province_buildings.entry(province).or_default();
            for inner in block(&item.value, "province buildings")? {
                let name = key_str(inner, "province buildings")?.to_owned();
                entry.insert(name, building_level(&inner.value, "province building")?);
            }
        } else {
            state.buildings.insert(
                key.to_owned(),
                building_level(&item.value, "state building")?,
            );
        }
    }
    Ok(())
}

/// True for the `1939.1.1` style keys the engine applies on a later date.
fn is_dated(key: &str) -> bool {
    let parts = key.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Parses one `history/states` file into its start-of-campaign definition.
pub fn parse_state(input: &[u8]) -> io::Result<StateDefinition> {
    let document = parser::parse(input)?;
    let root = require(&document.items, "state", "state file")?;
    let items = block(root, "state")?;
    let mut state = StateDefinition {
        id: integer(require(items, "id", "state")?, "state id")?,
        ..StateDefinition::default()
    };
    state.name_key = text(require(items, "name", "state")?, "state name")?.to_owned();
    state.category = match last(items, b"state_category") {
        Some(value) => Some(text(value, "state_category")?.to_owned()),
        None => None,
    };
    if let Some(value) = last(items, b"impassable") {
        state.impassable = boolean(value, "impassable")?;
    }
    if let Some(value) = last(items, b"manpower") {
        state.manpower = Some(integer(value, "state manpower")?);
    }
    if let Some(value) = last(items, b"provinces") {
        state.provinces = scalar_list(value, "state provinces")?;
    }
    if let Some(value) = last(items, b"resources") {
        for item in block(value, "state resources")? {
            let name = key_str(item, "state resources")?.to_owned();
            state
                .resources
                .insert(name, number(&item.value, "state resource")?);
        }
    }
    for history in all(items, b"history") {
        for item in block(history, "state history")? {
            let key = key_str(item, "state history")?;
            match key {
                "victory_points" => {
                    let pair = block(&item.value, "victory_points")?;
                    let [province, value] = pair else {
                        return Err(invalid(format!(
                            "victory_points needs a province and a value, not {} entries",
                            pair.len()
                        )));
                    };
                    if province.key.is_some() || value.key.is_some() {
                        return Err(invalid("victory_points holds a keyed entry"));
                    }
                    state.victory_points.push((
                        integer(&province.value, "victory point province")?,
                        number(&value.value, "victory point value")?,
                    ));
                }
                "buildings" => read_buildings(&item.value, &mut state)?,
                _ if is_dated(key) || key.eq_ignore_ascii_case("if") => {
                    state.skipped_history.push(key.to_owned());
                }
                _ => {}
            }
        }
    }
    Ok(state)
}

/// Reads every `history/states/*.txt` file from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<BTreeMap<u32, StateDefinition>> {
    let mut states: BTreeMap<u32, StateDefinition> = BTreeMap::new();
    let mut sources: BTreeMap<u32, String> = BTreeMap::new();
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("history/states/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let state = parse_state(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        let id = state.id;
        if let Some(previous) = sources.insert(id, path.clone()) {
            return Err(invalid(format!(
                "state {id} is defined by both {previous} and {path}"
            )));
        }
        states.insert(id, state);
    }
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"
state = {
    id = 1027
    name = "STATE_1027" # Bataan
    manpower = 2371662
    resources = { chromium = 23.000 }
    state_category = rural
    state_category = town
    history = {
        owner = PHI
        buildings = {
            infrastructure = 2
            air_base = 3
            7351 = {
                naval_base = 1
                naval_headquarters = { level = 2 allowed = { has_dlc = "x" } }
            }
        }
        1939.1.1 = { buildings = { industrial_complex = 4 } }
        if = { limit = { } }
        victory_points = { 7351 2 }
        victory_points = { 1415 1 }
    }
    provinces = { 1415 4423 7351 }
    local_supplies = 0.0
}
"#;

    #[test]
    fn a_state_keeps_its_provinces_points_buildings_and_repeated_category() {
        let state = parse_state(SAMPLE).unwrap();
        assert_eq!(state.id, 1027);
        assert_eq!(state.name_key, "STATE_1027");
        // The engine's own files assign state_category twice; the later wins.
        assert_eq!(state.category.as_deref(), Some("town"));
        assert_eq!(state.manpower, Some(2_371_662));
        assert_eq!(state.provinces, vec![1415, 4423, 7351]);
        assert_eq!(state.resources["chromium"], 23.0);
        assert_eq!(state.victory_points, vec![(7351, 2.0), (1415, 1.0)]);
        assert_eq!(
            state.buildings["infrastructure"],
            BuildingLevel {
                level: 2,
                conditional: false
            }
        );
        assert_eq!(state.buildings["air_base"].level, 3);
        let province = &state.province_buildings[&7351];
        assert_eq!(province["naval_base"].level, 1);
        assert_eq!(
            province["naval_headquarters"],
            BuildingLevel {
                level: 2,
                conditional: true
            }
        );
        assert!(!state.impassable);
        // The dated and conditional blocks are listed, never applied.
        assert_eq!(state.skipped_history, vec!["1939.1.1", "if"]);
        assert!(!state.buildings.contains_key("industrial_complex"));
    }

    #[test]
    fn an_impassable_state_without_history_buildings_still_parses() {
        let state = parse_state(
            br#"state = { id = 1022 name = "STATE_1022" impassable = yes
                 resources = { } history = { owner = INS } provinces = { 1230 } }"#,
        )
        .unwrap();
        assert!(state.impassable);
        assert_eq!(state.category, None);
        assert_eq!(state.manpower, None);
        assert!(state.buildings.is_empty());
        assert!(state.victory_points.is_empty());
        assert!(state.skipped_history.is_empty());
    }

    #[test]
    fn dated_keys_are_recognised_only_in_the_engine_s_own_shape() {
        for key in ["1939.1.1", "1936.6.1", "0.0.0"] {
            assert!(is_dated(key), "{key}");
        }
        for key in ["buildings", "1939.1", "1939.1.1.1", "a.b.c", "1939..1", ""] {
            assert!(!is_dated(key), "{key}");
        }
    }

    #[test]
    fn malformed_states_are_named_rather_than_guessed() {
        for (input, message) in [
            (br#"id = 1"#.as_slice(), "state file has no state"),
            (br#"state = { name = "x" }"#, "state has no id"),
            (br#"state = { id = 1 }"#, "state has no name"),
            (br#"state = { id = x name = "y" }"#, "state id"),
            (
                br#"state = { id = 1 name = "y" history = { victory_points = { 1 } } }"#,
                "victory_points needs a province and a value",
            ),
            (
                br#"state = { id = 1 name = "y" history = { victory_points = { a = 1 b = 2 } } }"#,
                "victory_points holds a keyed entry",
            ),
            (
                br#"state = { id = 1 name = "y" history = { victory_points = { 1 x } } }"#,
                "victory point value",
            ),
            (
                br#"state = { id = 1 name = "y" impassable = maybe }"#,
                "not yes or no",
            ),
            (
                br#"state = { id = 1 name = "y" provinces = { a } }"#,
                "state provinces",
            ),
            (
                br#"state = { id = 1 name = "y" history = { buildings = { x = { } } } }"#,
                "has no level",
            ),
        ] {
            let error = parse_state(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }
}
