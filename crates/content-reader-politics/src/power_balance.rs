use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::ResolvedLayers;
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}};
use content_reader_parser::parser::{self, Item};
use std::{collections::BTreeMap, io};

pub fn rules() -> &'static [(&'static str, &'static str, bool, bool)] {
    use engine_executable::executable_tables::{auxiliary_part, required};
    let rules = required(auxiliary_part(|tables| &tables.rules));
    rules
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub id: Vec<u8>,
    pub min: i64,
    pub max: i64,
    pub modifiers: modifiers::Modifiers,
    pub rules: Vec<bool>,
    pub activation: Vec<Item>,
    pub deactivation: Vec<Item>,
}

impl Default for Range {
    fn default() -> Self {
        Self {
            id: Vec::new(),
            min: 0,
            max: 100_000,
            modifiers: modifiers::Modifiers::default(),
            rules: Vec::new(),
            activation: Vec::new(),
            deactivation: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub id: Vec<u8>,
    pub initial_value: i64,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub sides: Vec<Vec<u8>>,
}

#[derive(Default)]
pub struct Database {
    balances: BTreeMap<Vec<u8>, Vec<Range>>,
    definitions: BTreeMap<Vec<u8>, Definition>,
    pub notes: Vec<String>,
}

impl Database {
    pub fn definition(&self, id: &[u8]) -> Option<&Definition> {
        self.definitions
            .get(&parser::c_string(id).to_ascii_lowercase())
    }

    pub fn ranges(&self, balance: &[u8]) -> Option<&[Range]> {
        self.balances
            .get(&parser::c_string(balance).to_ascii_lowercase())
            .map(Vec::as_slice)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_parsed(entries: impl IntoIterator<Item = (Definition, Vec<Range>)>) -> Self {
        let mut database = Self::default();
        for (definition, ranges) in entries {
            let key = definition.id.to_ascii_lowercase();
            database.balances.insert(key.clone(), ranges);
            database.definitions.insert(key, definition);
        }
        database
    }
    pub fn range(&self, balance: &[u8], range: &[u8]) -> Result<Option<&Range>, String> {
        let balance = parser::c_string(balance);
        let range = parser::c_string(range);
        let matches: Vec<_> = self
            .balances
            .get(&balance.to_ascii_lowercase())
            .into_iter()
            .flatten()
            .filter(|r| r.id == range)
            .collect();
        match matches.as_slice() {
            [] if range.is_empty() => Ok(None),
            _ if range.is_empty() => Err(format!(
                "balance {}: {} unnamed authored ranges cannot be distinguished from the null range; the recording lacks the active range's pointer or ordinal",
                String::from_utf8_lossy(balance),
                matches.len()
            )),
            [found] => Ok(Some(found)),
            [] => Err(format!(
                "balance {}: 1 missing authored range {}",
                String::from_utf8_lossy(balance),
                String::from_utf8_lossy(range)
            )),
            _ => Err(format!(
                "balance {}: {} ranges named {}; the recording lacks the active range's pointer or ordinal",
                String::from_utf8_lossy(balance),
                matches.len(),
                String::from_utf8_lossy(range)
            )),
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    Top,
    Balance,
    Side,
    Range,
    Modifier(modifiers::Node),
    Rule(content_reader_script::idea_fields::RuleNode),
}

fn read_file(
    input: &[u8],
    registry: modifiers::Registry<'_>,
    names: &KeyNames,
) -> Result<native::Read, String> {
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        Some(names),
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Balance),
            Node::Balance => match field.keyword().as_deref() {
                Some(b"range") => Shape::Object(Node::Range),
                Some(b"side") => Shape::Object(Node::Side),
                Some(b"left_side" | b"right_side" | b"decision_category") => Shape::Current,
                Some(b"initial_value") => Shape::One,
                Some(b"on_activate" | b"on_deactivate") => Shape::ByKey,
                _ => Shape::Skip,
            },
            Node::Side => match field.keyword().as_deref() {
                Some(b"range") => Shape::Object(Node::Range),
                Some(b"id") => Shape::Current,
                Some(b"icon") => Shape::One,
                Some(b"on_activate" | b"on_deactivate") => Shape::ByKey,
                _ => Shape::Skip,
            },
            Node::Range => match field.keyword().as_deref() {
                Some(b"id") => Shape::Current,
                Some(b"min" | b"max") => Shape::One,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"rule") => Shape::Object(Node::Rule(content_reader_script::idea_fields::RuleNode::Rule)),
                Some(b"on_activate" | b"on_deactivate") => Shape::Effect,
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, registry, &mut undecided).map(Node::Modifier)
            }
            Node::Rule(node) => content_reader_script::idea_fields::rule_field(node, field).map(Node::Rule),
        },
    );
    if let Some(reason) = undecided
        .first()
        .map(ToString::to_string)
        .or_else(|| read.unsettled.as_ref().map(ToString::to_string))
    {
        return Err(reason);
    }
    Ok(read)
}

fn range(items: &[Item], notes: &mut Vec<String>, registry: modifiers::Registry<'_>) -> Range {
    let mut range = Range {
        rules: rules().iter().map(|r| r.2).collect(),
        ..Default::default()
    };
    for item in items {
        match item.keyword().map(|k| k.folded()).as_deref() {
            Some(b"id") => {
                range.id = parser::c_string(item.value.scalar().unwrap_or_default()).to_vec()
            }
            Some(b"min") => {
                if let Some(value) = modifiers::read_fixed5(item) {
                    range.min = value;
                }
            }
            Some(b"max") => {
                if let Some(value) = modifiers::read_fixed5(item) {
                    range.max = value;
                }
            }
            Some(b"modifier") => range
                .modifiers
                .read_all(item.value.items().unwrap_or_default()),
            Some(b"on_activate") => range
                .activation
                .extend_from_slice(item.value.items().unwrap_or_default()),
            Some(b"on_deactivate") => range
                .deactivation
                .extend_from_slice(item.value.items().unwrap_or_default()),
            Some(b"rule") => {
                for rule in item.value.items().unwrap_or_default() {
                    if let Some(at) = rules().iter().position(|r| rule.keyword_is(r.0)) {
                        let value = rule.value.scalar().and_then(parser::read_bool);
                        if value.is_none() {
                            notes.push(format!(
                                "range {}: rule {} failed boolean conversion; stored false",
                                String::from_utf8_lossy(&range.id),
                                rules()[at].0
                            ));
                        }
                        range.rules[at] = value.unwrap_or(false);
                    } else if !rule.keyword_is("icon")
                        && !rule.keyword_is("desc")
                        && !rule.keyword_is("override")
                    {
                        notes.push(format!(
                            "range {}: unknown rule {} ignored",
                            String::from_utf8_lossy(&range.id),
                            String::from_utf8_lossy(rule.key.as_deref().unwrap_or_default())
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    range.modifiers.values.retain(|m| {
        if registry(m.name.as_bytes()) == Some(false) {
            notes.push(format!(
                "range {}: unknown modifier {} ignored",
                String::from_utf8_lossy(&range.id),
                m.key
            ));
            false
        } else {
            true
        }
    });
    let mut scans = modifiers::ScanFailures::default();
    scans.modifiers(&String::from_utf8_lossy(&range.id), &range.modifiers);
    notes.extend(scans.warning());
    range
}

pub fn read(layers: &ResolvedLayers, registry: modifiers::Registry<'_>) -> io::Result<Database> {
    let mut database = Database::default();
    let names = layers.key_names()?;
    for path in layers.directory_files("common/bop", ".txt")? {
        let read = read_file(&path.read_uncovered(layers)?, registry, &names).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{path}: 1 unread balance file: {e}"),
            )
        })?;
        database
            .notes
            .extend(read.misreads.iter().map(|n| format!("{path}: {n}")));
        for balance in read.items {
            let key = parser::c_string(balance.key.as_deref().unwrap_or_default()).to_vec();
            let items = balance.value.items().unwrap_or_default();
            let mut definition = Definition {
                id: key.clone(),
                ..Default::default()
            };
            for item in items {
                if item.keyword_is("initial_value") {
                    if let Some(value) = modifiers::read_fixed5(item) {
                        definition.initial_value = value;
                    }
                } else if item.keyword_is("left_side") {
                    definition.left =
                        parser::c_string(item.value.scalar().unwrap_or_default()).to_vec();
                } else if item.keyword_is("right_side") {
                    definition.right =
                        parser::c_string(item.value.scalar().unwrap_or_default()).to_vec();
                } else if item.keyword_is("side") {
                    let mut id = Vec::new();
                    for own in item
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|own| own.keyword_is("id"))
                    {
                        id = parser::c_string(own.value.scalar().unwrap_or_default()).to_vec();
                    }
                    definition.sides.push(id);
                }
            }
            database
                .definitions
                .insert(key.to_ascii_lowercase(), definition);
            let mut ranges = Vec::new();
            for item in items.iter().filter(|i| i.keyword_is("range")) {
                ranges.push(range(
                    item.value.items().unwrap_or_default(),
                    &mut database.notes,
                    registry,
                ));
            }
            for side in items.iter().filter(|i| i.keyword_is("side")) {
                for item in side
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|i| i.keyword_is("range"))
                {
                    ranges.push(range(
                        item.value.items().unwrap_or_default(),
                        &mut database.notes,
                        registry,
                    ));
                }
            }
            if let Some(cause) = ranges.iter().find_map(|r| r.modifiers.unheld.cause()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{path}: 1 unread balance file: {cause}"),
                ));
            }
            if database
                .balances
                .insert(key.to_ascii_lowercase(), ranges)
                .is_some()
            {
                database.notes.push(format!(
                    "{path}: duplicate template {} replaced",
                    String::from_utf8_lossy(&key)
                ));
            }
        }
    }
    Ok(database)
}

pub fn read_descriptions(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    read(layers, &|key| modifiers::registered(key, Some(&names)))
}

#[cfg(test)]
mod tests;
