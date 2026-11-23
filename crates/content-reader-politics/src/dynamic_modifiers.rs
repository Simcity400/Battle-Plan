use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::ResolvedLayers;
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}};
use content_reader_parser::parser::{self, Item};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dynamic {
    pub name: String,
    pub modifiers: modifiers::Modifiers,
    pub scoped_modifiers: Vec<(String, Vec<u8>)>,
    pub icon: Vec<u8>,
}

#[derive(Default)]
pub struct Database {
    pub definitions: BTreeMap<String, Dynamic>,
    pub notes: Vec<String>,
    refused: BTreeMap<String, String>,
    gap: Option<String>,
}

impl Database {
    pub fn find(&self, name: &str) -> Result<Option<&Dynamic>, String> {
        let name = parser::text(parser::c_string(name.as_bytes())).map_err(|e| e.to_string())?;
        if let Some(reason) = self.refused.get(name) {
            return Err(reason.clone());
        }
        if let Some(found) = self.definitions.get(name) {
            return Ok(Some(found));
        }
        match &self.gap {
            Some(reason) => Err(format!("dynamic modifier {name}: {reason}")),
            None => Ok(None),
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Dynamic,
    Modifier(modifiers::Node),
}

fn constant(text: &[u8]) -> bool {
    parser::c_string(text)
        .iter()
        .all(|byte| b"-+0123456789.".contains(byte))
}

pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Vec<Dynamic>> {
    parse_with_notes(input, names).map(|(definitions, _)| definitions)
}

fn parse_with_notes(input: &[u8], names: &KeyNames) -> io::Result<(Vec<Dynamic>, Vec<String>)> {
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Dynamic),
            Node::Dynamic => match field.keyword().as_deref() {
                Some(b"icon" | b"attacker_modifier") => Shape::One,
                Some(b"enable" | b"remove_trigger") => Shape::Trigger,
                Some(b"custom_modifier_tooltip" | b"hidden_modifier" | b"added_modifier") => {
                    modifiers::shape(modifiers::Node::Modifier, field, &registry, &mut undecided)
                        .map(Node::Modifier)
                }
                _ if constant(field.value.c_str()) => {
                    modifiers::shape(modifiers::Node::Modifier, field, &registry, &mut undecided)
                        .map(Node::Modifier)
                }
                _ if field.keyword().is_none() => Shape::Skip,
                _ => match registry(&field.key.text) {
                    Some(true) => Shape::One,
                    Some(false) => Shape::Skip,
                    None => Shape::Unsettled(format!(
                        "dynamic modifier {}: modifier token registry unavailable",
                        String::from_utf8_lossy(&field.key.text)
                    )),
                },
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, &registry, &mut undecided).map(Node::Modifier)
            }
        },
    );
    if let Some(error) = read.unsettled {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            error.to_string(),
        ));
    }
    if let Some(error) = undecided.first() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            error.to_string(),
        ));
    }
    let notes = read.misreads.iter().map(ToString::to_string).collect();
    let definitions = read
        .items
        .iter()
        .map(|item| definition(item, names))
        .collect::<io::Result<_>>()?;
    Ok((definitions, notes))
}

fn definition(item: &Item, names: &KeyNames) -> io::Result<Dynamic> {
    let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidData, error);
    let name = parser::text(parser::c_string(item.key.as_deref().unwrap_or_default()))
        .map_err(|e| invalid(e.to_string()))?
        .to_owned();
    let mut result = Dynamic {
        name,
        ..Dynamic::default()
    };
    for field in item.value.items().unwrap_or_default() {
        match field.keyword().map(|key| key.folded()).as_deref() {
            Some(b"icon") => {
                result.icon = parser::c_string(field.value.scalar().unwrap_or_default()).to_vec();
            }
            Some(b"attacker_modifier" | b"enable" | b"remove_trigger") => {}
            Some(b"custom_modifier_tooltip" | b"hidden_modifier" | b"added_modifier") => {
                result.modifiers.read(field)
            }
            _ => {
                let value = field.value.scalar().unwrap_or(b"{");
                if constant(value) {
                    result.modifiers.read(field);
                } else if field.keyword().is_some() {
                    let key = field.key_str().map_err(|e| invalid(e.to_string()))?;
                    if modifiers::registered(key.as_bytes(), Some(names)) == Some(true) {
                        result
                            .scoped_modifiers
                            .push((key.to_ascii_lowercase(), parser::c_string(value).to_vec()));
                    }
                }
            }
        }
    }
    filter_unknown(&mut result.modifiers, names);
    if let Some(error) = result.modifiers.unheld.cause() {
        return Err(invalid(format!(
            "dynamic modifier {}: {error}",
            result.name
        )));
    }
    Ok(result)
}

fn filter_unknown(block: &mut modifiers::Modifiers, names: &KeyNames) {
    block
        .values
        .retain(|value| modifiers::registered(value.name.as_bytes(), Some(names)) != Some(false));
    for child in &mut block.children {
        filter_unknown(child, names);
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in layers.directory_files("common/dynamic_modifiers", ".txt")? {
        match parse_with_notes(&path.read(layers)?, &names) {
            Ok((definitions, notes)) => {
                for definition in definitions {
                    database.refused.remove(&definition.name);
                    database
                        .definitions
                        .insert(definition.name.clone(), definition);
                }
                database
                    .notes
                    .extend(notes.into_iter().map(|note| format!("{path}: {note}")));
            }
            Err(error) => {
                let reason =
                    format!("{path}: {error}; unread definitions may replace earlier exact names");
                for name in database.definitions.keys() {
                    database.refused.insert(name.clone(), reason.clone());
                }
                database.notes.push(reason.clone());
                database.gap = Some(reason);
            }
        }
    }
    Ok(database)
}

#[cfg(test)]
mod tests;
