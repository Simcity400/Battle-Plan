//! Autonomy states (`common/autonomous_states`): the subject levels a recording
//! names by id (`Autonomy.state`, `Subjects.state`). Each state's order is its
//! authored `min_freedom_level` (the engine's fixed5 conversion, so equal texts
//! compare equal), then load order; the dashboard names and colours subject
//! types in that order, so a mod's states work without a list of vanilla ids.
//! A level that is missing or not a plain number is left unknown, never
//! assumed, and its state sorts after every state with a known level.

use crate::{
    Diagnostic, DiagnosticKind, ResolvedLayers,
    fields::invalid,
    fixed5, ideas,
    key_names::KeyNames,
    modifiers,
    parser::{
        Item, Value,
        native::{self, Shape},
    },
    script_files,
};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutonomyState {
    /// The state's id, as the recording names it.
    pub id: String,
    /// `min_freedom_level` in fixed5 (five decimals); None when it is missing or
    /// not a plain number.
    pub min_freedom_level: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutonomyStates {
    /// In load order; a state defined again replaces the earlier one in place.
    pub states: Vec<AutonomyState>,
}

impl AutonomyStates {
    pub fn get(&self, id: &str) -> Option<&AutonomyState> {
        self.states.iter().find(|state| state.id == id)
    }

    /// Every state from the least free to the freest: by `min_freedom_level`, then
    /// load order, with states whose level is unknown last (in load order).
    pub fn ordered(&self) -> Vec<&AutonomyState> {
        let mut ordered: Vec<(usize, &AutonomyState)> = self.states.iter().enumerate().collect();
        ordered.sort_by_key(|(index, state)| {
            (
                state.min_freedom_level.is_none(),
                state.min_freedom_level,
                *index,
            )
        });
        ordered.into_iter().map(|(_, state)| state).collect()
    }
}

/// The definitions of one autonomy file, as the database reads it: both
/// top-level spellings by keyword, variables replayed and every field
/// consumed by its reader. A read that cannot be settled refuses the file.
pub(crate) fn read_definitions(input: &[u8], names: &KeyNames) -> io::Result<Vec<Item>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        State,
        Modifier(modifiers::Node),
        Rule(ideas::RuleNode),
        Mtth,
    }
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File
                if field.keyword_is("autonomy_state") || field.keyword_is("autonomous_state") =>
            {
                Shape::Object(Node::State)
            }
            Node::File => Shape::Skip,
            Node::State => match field.keyword().as_deref() {
                Some(
                    b"id"
                    | b"default"
                    | b"cost"
                    | b"is_puppet"
                    | b"min_freedom_level"
                    | b"manpower_influence"
                    | b"peace_conference_initial_freedom"
                    | b"use_overlord_color",
                ) => Shape::One,
                Some(
                    b"allowed"
                    | b"can_take_level"
                    | b"can_lose_level"
                    | b"ai_overlord_wants_garrison",
                ) => Shape::Trigger,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"rule") => Shape::Object(Node::Rule(ideas::RuleNode::Rule)),
                Some(b"allowed_levels_filter") => Shape::List,
                Some(b"ai_subject_wants_higher" | b"ai_overlord_wants_lower") => {
                    Shape::Object(Node::Mtth)
                }
                Some(b"use_for_peace_conference_weight") => Shape::ObjectIfOpen(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
            Node::Rule(node) => ideas::rule_field(node, field).map(Node::Rule),
            Node::Mtth => native::mean_time_to_happen(field),
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("autonomy definitions: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("autonomy definitions: {cause}")));
    }
    Ok(read
        .items
        .into_iter()
        .filter(|item| item.keyword_is("autonomy_state") || item.keyword_is("autonomous_state"))
        .collect())
}

/// The last field that is the keyword `key`.
fn last<'a>(items: &'a [Item], key: &str) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .map(|item| &item.value)
}

/// Require a complete decimal, so a numeric prefix of unsupported content is
/// never mistaken for a known level. Conversion still uses the engine's fixed5.
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

/// Every autonomy state in load order, with notes about files and states left
/// out or unordered. A state defined again replaces the earlier one in place.
pub fn read(layers: &ResolvedLayers) -> io::Result<(AutonomyStates, Vec<Diagnostic>)> {
    let names = layers.key_names()?;
    let mut warnings = Vec::new();
    let mut result = AutonomyStates::default();
    for path in script_files::paths(layers, "common/autonomous_states/", Some(".txt")) {
        let path_text = path.to_string();
        match read_definitions(&path.read(layers)?, &names) {
            Ok(definitions) => {
                for state in from_definitions(&definitions, &path_text, &mut warnings) {
                    merge(&mut result, state, &path_text, &mut warnings);
                }
            }
            Err(error) => warnings.push((
                DiagnosticKind::ParseFailure,
                format!("Autonomy states unavailable in {path}: {error}"),
            )),
        }
    }
    Ok((result, warnings))
}

fn merge(
    result: &mut AutonomyStates,
    state: AutonomyState,
    path: &str,
    warnings: &mut Vec<Diagnostic>,
) {
    match result
        .states
        .iter_mut()
        .find(|existing| existing.id == state.id)
    {
        Some(existing) => {
            warnings.push((
                DiagnosticKind::Warning,
                format!(
                    "{path}: autonomy state {} is defined again; the later definition is used.",
                    state.id
                ),
            ));
            *existing = state;
        }
        None => result.states.push(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(files: &[(&str, &str)]) -> (AutonomyStates, Vec<Diagnostic>) {
        let mut warnings = Vec::new();
        let mut result = AutonomyStates::default();
        for (path, text) in files {
            let definitions = read_definitions(text.as_bytes(), &KeyNames::default()).unwrap();
            for state in from_definitions(&definitions, path, &mut warnings) {
                merge(&mut result, state, path, &mut warnings);
            }
        }
        (result, warnings)
    }

    /// Both top-level spellings and every keyword in any ASCII case are read,
    /// as the map's colour rule reads them; quoted keys are no keywords.
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
        let order: Vec<_> = read
            .ordered()
            .iter()
            .map(|state| state.id.as_str())
            .collect();
        // Equal levels (0.2 and 0.20) keep load order.
        assert_eq!(order, ["tight", "also_tight", "middle", "loose"]);
        assert_eq!(read.get("loose").unwrap().min_freedom_level, Some(80_000));
    }

    #[test]
    fn unknown_levels_sort_last_and_are_reported() {
        let (read, warnings) = states(&[(
            "mod.txt",
            "autonomy_state = { id = scripted min_freedom_level = @level } autonomy_state = { id = bare } \
             autonomy_state = { id = known min_freedom_level = 0.4 } autonomy_state = { min_freedom_level = 0.1 }",
        )]);
        let order: Vec<_> = read
            .ordered()
            .iter()
            .map(|state| state.id.as_str())
            .collect();
        assert_eq!(order, ["known", "scripted", "bare"]);
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
            // The level reader takes `{` as its value: the block's contents join the
            // state, whose own `}` then ends the file's loop.
            (
                "z.txt",
                "autonomy_state = { id = block min_freedom_level = { value = 0.2 } } \
                 autonomy_state = { id = unread min_freedom_level = 0.5 }",
            ),
        ]);
        for id in ["suffix", "exponent", "block"] {
            assert_eq!(read.get(id).unwrap().min_freedom_level, None);
            assert!(warnings.iter().any(|(_, warning)| warning.contains(id)));
        }
        assert_eq!(warnings.len(), 3);
        assert!(read.get("unread").is_none());
        assert_eq!(read.get("signed").unwrap().min_freedom_level, Some(60_000));
        assert_eq!(
            read.get("negative").unwrap().min_freedom_level,
            Some(-123_456)
        );
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
        // Equal levels: the replaced state keeps its first place in load order.
        let order: Vec<_> = read
            .ordered()
            .iter()
            .map(|state| state.id.as_str())
            .collect();
        assert_eq!(order, ["first", "second"]);
        assert_eq!(warnings.len(), 1);
    }
}
