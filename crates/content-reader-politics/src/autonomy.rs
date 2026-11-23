use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_key_names::autonomy_definitions::read_definitions;

use content_reader_parser::{Diagnostic, DiagnosticKind, coverage::{Coverage, Gap}, fixed5, parser::{Item, Value}};
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_script::{key_names::KeyNames, };
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutonomyState {
    pub id: String,
    pub min_freedom_level: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutonomyStates {
    pub states: Vec<AutonomyState>,
    levels: Coverage,
    places: Coverage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Order<'a> {
    pub placed: Vec<&'a AutonomyState>,
    pub unplaced: Vec<&'a AutonomyState>,
}

impl AutonomyStates {
    pub fn new(states: Vec<AutonomyState>) -> Self {
        Self {
            states,
            ..Self::default()
        }
    }

    pub fn get(&self, id: &str) -> Result<Option<&AutonomyState>, &Gap> {
        match self.levels.unknown(id) {
            Some(gap) => Err(gap),
            None => Ok(self.states.iter().find(|state| state.id == id)),
        }
    }

    pub fn gap(&self) -> Option<&Gap> {
        self.levels.any_gap()
    }

    pub fn ordered(&self) -> Order<'_> {
        let mut placed: Vec<(usize, &AutonomyState, i64)> = Vec::new();
        let mut unplaced = vec![false; self.states.len()];
        for (index, state) in self.states.iter().enumerate() {
            match state.min_freedom_level {
                Some(level) if self.levels.unknown(&state.id).is_none() => {
                    placed.push((index, state, level));
                }
                _ => unplaced[index] = true,
            }
        }
        placed.sort_by_key(|&(index, _, level)| (level, index));
        for group in placed.chunk_by(|a, b| a.2 == b.2) {
            if let Some(from) = group
                .iter()
                .position(|(_, state, _)| self.places.before_first(&state.id).is_some())
                && group.len() - from > 1
            {
                for &(index, _, _) in &group[from..] {
                    unplaced[index] = true;
                }
            }
        }
        Order {
            placed: placed
                .into_iter()
                .filter(|&(index, _, _)| !unplaced[index])
                .map(|(_, state, _)| state)
                .collect(),
            unplaced: self
                .states
                .iter()
                .zip(unplaced)
                .filter_map(|(state, unplaced)| unplaced.then_some(state))
                .collect(),
        }
    }
}

fn last<'a>(items: &'a [Item], key: &str) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .map(|item| &item.value)
}

fn plain_number(bytes: &[u8]) -> bool {
    let digits = bytes
        .strip_prefix(b"-")
        .or_else(|| bytes.strip_prefix(b"+"))
        .unwrap_or(bytes);
    let mut parts = digits.splitn(2, |byte| *byte == b'.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.iter().all(u8::is_ascii_digit)
        && fraction.is_none_or(|part| part.iter().all(u8::is_ascii_digit))
}

fn from_definitions(
    definitions: &[Item],
    path: &str,
    warnings: &mut Vec<Diagnostic>,
) -> Vec<AutonomyState> {
    let mut states = Vec::new();
    for definition in definitions {
        let Some(items) = definition.value.items() else {
            warnings.push((
                DiagnosticKind::Refused,
                format!("{path}: an autonomy_state is not a block and is left out."),
            ));
            continue;
        };
        let id = match last(items, "id").map(Value::read_str) {
            Some(Ok(id)) if !id.is_empty() => id,
            Some(Err(why)) => {
                warnings.push((
                    DiagnosticKind::Refused,
                    format!("{path}: an autonomy_state id is {why}; the state is left out."),
                ));
                continue;
            }
            _ => {
                warnings.push((
                    DiagnosticKind::Refused,
                    format!("{path}: an autonomy_state has no id and is left out."),
                ));
                continue;
            }
        };
        let level = last(items, "min_freedom_level");
        let min_freedom_level = level
            .and_then(Value::scalar)
            .filter(|bytes| plain_number(bytes))
            .and_then(fixed5::script_fixed5);
        if min_freedom_level.is_none() {
            warnings.push((DiagnosticKind::Refused, format!(
                "{path}: autonomy state {id} has {}; its place in the level order is not known.",
                if level.is_some() {
                    "a min_freedom_level that is not a plain number"
                } else {
                    "no min_freedom_level"
                }
            )));
        }
        states.push(AutonomyState {
            id: id.to_owned(),
            min_freedom_level,
        });
    }
    states
}

pub fn read(layers: &ResolvedLayers) -> io::Result<(AutonomyStates, Vec<Diagnostic>)> {
    let names = layers.key_names()?;
    let mut reading = Reading::default();
    for path in script_files::paths(layers, "common/autonomous_states/", Some(".txt"))? {
        reading.file(&path.to_string(), &path.read(layers)?, &names);
    }
    Ok((reading.result, reading.warnings))
}

#[derive(Default)]
struct Reading {
    result: AutonomyStates,
    warnings: Vec<Diagnostic>,
}

impl Reading {
    fn file(&mut self, path: &str, input: &[u8], names: &KeyNames) {
        let (definitions, stop) = read_definitions(input, names);
        for state in from_definitions(&definitions, path, &mut self.warnings) {
            self.merge(state, path);
        }
        if let Some(cause) = stop {
            self.warnings.push((
                DiagnosticKind::ParseFailure,
                format!(
                    "Autonomy states in {path} are unknown from {cause}; a level it could define or redefine is not ordered."
                ),
            ));
            self.result.levels.gap(path, cause.as_str());
            self.result.places.gap(path, cause);
        }
        self.result.levels.next_file();
        self.result.places.next_file();
    }

    fn merge(&mut self, state: AutonomyState, path: &str) {
        self.result.levels.replace(&state.id);
        self.result.places.add(&state.id);
        match self
            .result
            .states
            .iter_mut()
            .find(|existing| existing.id == state.id)
        {
            Some(existing) => {
                self.warnings.push((
                    DiagnosticKind::Warning,
                    format!(
                        "{path}: autonomy state {} is defined again; the later definition is used.",
                        state.id
                    ),
                ));
                *existing = state;
            }
            None => self.result.states.push(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(files: &[(&str, &str)]) -> (AutonomyStates, Vec<Diagnostic>) {
        let mut reading = Reading::default();
        for (path, text) in files {
            reading.file(path, text.as_bytes(), &KeyNames::default());
        }
        (reading.result, reading.warnings)
    }

    fn placed<'a>(order: &Order<'a>) -> Vec<&'a str> {
        order.placed.iter().map(|state| state.id.as_str()).collect()
    }

    #[test]
    fn states_are_read_through_the_database_keyword_dispatch() {
        let (read, warnings) = states(&[(
            "mod.txt",
            "autonomous_state = { ID = mod_level MIN_FREEDOM_LEVEL = 0.4 } \
             AUTONOMY_STATE = { id = upper min_freedom_level = 0.6 } \
             \"autonomy_state\" = { id = quoted min_freedom_level = 0.1 } \
             autonomy_state = { id = keyed \"min_freedom_level\" = 0.2 min_freedom_level = 0.3 \"id\" = other } \
             @level = 0.9 autonomy_state = { id = variable min_freedom_level = @level }",
        )]);
        assert!(warnings.is_empty(), "{warnings:?}");
        let levels: Vec<_> = read
            .states
            .iter()
            .map(|state| (state.id.as_str(), state.min_freedom_level))
            .collect();
        assert_eq!(
            levels,
            [
                ("mod_level", Some(40_000)),
                ("upper", Some(60_000)),
                ("keyed", Some(30_000)),
                ("variable", Some(90_000)),
            ]
        );
    }

    #[test]
    fn states_order_by_authored_level_then_load_order() {
        let (read, warnings) = states(&[
            (
                "a.txt",
                "autonomy_state = { id = loose min_freedom_level = 0.80 }",
            ),
            (
                "b.txt",
                "autonomy_state = { id = tight min_freedom_level = 0.2 } autonomy_state = { id = middle min_freedom_level = 0.5 }",
            ),
            (
                "c.txt",
                "autonomy_state = { id = also_tight min_freedom_level = 0.20 rule = { x = yes } }",
            ),
        ]);
        assert!(warnings.is_empty(), "{warnings:?}");
        let order = read.ordered();
        assert_eq!(placed(&order), ["tight", "also_tight", "middle", "loose"]);
        assert!(order.unplaced.is_empty());
        assert_eq!(
            read.get("loose").unwrap().unwrap().min_freedom_level,
            Some(80_000)
        );
    }

    #[test]
    fn unknown_levels_sort_last_and_are_reported() {
        let (read, warnings) = states(&[(
            "mod.txt",
            "autonomy_state = { id = scripted min_freedom_level = @level } autonomy_state = { id = bare } \
             autonomy_state = { id = known min_freedom_level = 0.4 } autonomy_state = { min_freedom_level = 0.1 }",
        )]);
        let order = read.ordered();
        assert_eq!(placed(&order), ["known"]);
        let unplaced: Vec<_> = order
            .unplaced
            .iter()
            .map(|state| state.id.as_str())
            .collect();
        assert_eq!(unplaced, ["scripted", "bare"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .all(|(kind, _)| *kind == DiagnosticKind::Refused)
        );
        assert!(warnings[0].1.contains("not a plain number"));
        assert!(warnings[1].1.contains("no min_freedom_level"));
        assert!(warnings[2].1.contains("no id"));
    }

    #[test]
    fn unsupported_numeric_suffixes_are_unknown_instead_of_prefix_levels() {
        let (read, warnings) = states(&[
            (
                "mod.txt",
                "autonomy_state = { id = suffix min_freedom_level = 0.4a } \
                 autonomy_state = { id = exponent min_freedom_level = 1e3 } \
                 autonomy_state = { id = signed min_freedom_level = +0.60 } \
                 autonomy_state = { id = negative min_freedom_level = -1.234567 }",
            ),
            (
                "z.txt",
                "autonomy_state = { id = block min_freedom_level = { value = 0.2 } } \
                 autonomy_state = { id = unread min_freedom_level = 0.5 }",
            ),
        ]);
        let level = |id| read.get(id).unwrap().unwrap().min_freedom_level;
        for id in ["suffix", "exponent", "block"] {
            assert_eq!(level(id), None);
            assert!(warnings.iter().any(|(_, warning)| warning.contains(id)));
        }
        assert_eq!(warnings.len(), 3);
        assert_eq!(read.get("unread"), Ok(None));
        assert_eq!(level("signed"), Some(60_000));
        assert_eq!(level("negative"), Some(-123_456));
    }

    #[test]
    fn a_state_defined_again_replaces_the_earlier_one_in_place() {
        let (read, warnings) = states(&[
            (
                "a.txt",
                "autonomy_state = { id = first min_freedom_level = 0.9 } autonomy_state = { id = second min_freedom_level = 0.1 }",
            ),
            (
                "z.txt",
                "autonomy_state = { id = first min_freedom_level = 0.1 }",
            ),
        ]);
        assert_eq!(read.states.len(), 2);
        assert_eq!(read.states[0].id, "first");
        assert_eq!(read.states[0].min_freedom_level, Some(10_000));
        assert_eq!(placed(&read.ordered()), ["first", "second"]);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn a_file_that_stops_short_leaves_every_level_it_could_redefine_unknown() {
        let (read, warnings) = states(&[
            (
                "a.txt",
                "autonomy_state = { id = early min_freedom_level = 0.2 } \
                 autonomy_state = { id = kept min_freedom_level = 0.5 } \
                 autonomy_state = { id = untouched min_freedom_level = 0.4 }",
            ),
            (
                "b.txt",
                "autonomy_state = { id = early min_freedom_level = 0.8 } \
                 autonomy_state = { id = stops min_freedom_level = 0.3 modifier = { unknown_modifier = { x = 1 } } } \
                 autonomy_state = { id = after min_freedom_level = 0.1 }",
            ),
            (
                "c.txt",
                "autonomy_state = { id = kept min_freedom_level = 0.6 }",
            ),
        ]);
        assert_eq!(read.states[0].min_freedom_level, Some(80_000));
        for id in ["early", "untouched", "stops", "after"] {
            assert_eq!(read.get(id).unwrap_err().path, "b.txt", "{id}");
        }
        assert!(read.states.iter().all(|state| state.id != "stops"));
        assert_eq!(
            read.get("kept").unwrap().unwrap().min_freedom_level,
            Some(60_000)
        );
        let order = read.ordered();
        assert_eq!(placed(&order), ["kept"]);
        let unplaced: Vec<_> = order
            .unplaced
            .iter()
            .map(|state| state.id.as_str())
            .collect();
        assert_eq!(unplaced, ["early", "untouched"]);
        assert_eq!(read.gap().unwrap().path, "b.txt");
        assert!(
            warnings
                .iter()
                .any(|(kind, warning)| *kind == DiagnosticKind::ParseFailure
                    && warning.contains("b.txt")
                    && warning.contains("unknown_modifier")),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_gap_before_equal_levels_leaves_their_order_unknown() {
        let gap = "autonomy_state = { id = gap modifier = { unknown_modifier = { x = 1 } } }";
        let (read, _) = states(&[
            ("a.txt", gap),
            (
                "b.txt",
                "autonomy_state = { id = first min_freedom_level = 0.5 } \
                 autonomy_state = { id = second min_freedom_level = 0.5 } \
                 autonomy_state = { id = alone min_freedom_level = 0.7 }",
            ),
        ]);
        let order = read.ordered();
        assert_eq!(placed(&order), ["alone"]);
        assert_eq!(order.unplaced.len(), 2);
        let (read, _) = states(&[
            (
                "a.txt",
                "autonomy_state = { id = early_tie min_freedom_level = 0.5 }",
            ),
            ("b.txt", gap),
            (
                "c.txt",
                "autonomy_state = { id = late_tie min_freedom_level = 0.5 } \
                 autonomy_state = { id = early_tie min_freedom_level = 0.5 }",
            ),
        ]);
        let order = read.ordered();
        assert_eq!(placed(&order), ["early_tie", "late_tie"]);
        assert!(order.unplaced.is_empty());
    }
}
