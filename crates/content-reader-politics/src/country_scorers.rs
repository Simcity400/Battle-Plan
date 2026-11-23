
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_script::{trigger_grammar::{Node as TriggerNode, Shaper}, key_names::KeyNames, native::{self, Shape}, triggers::Triggers};
use content_reader_parser::parser::{Item, c_string};

#[derive(Clone, Debug, Default)]
pub struct ScriptTargets {
    pub arrays: Vec<Vec<u8>>,
    pub targets: Vec<Vec<u8>>,
    pub root_limit: Triggers,
    pub target_limit: Triggers,
    pub dynamic: bool,
    pub non_existing: bool,
    pub unported: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub targets: ScriptTargets,
    pub score: Triggers,
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    entries: BTreeMap<Vec<u8>, Result<Vec<Entry>, String>>,
    pub notes: Vec<String>,
    unread: Vec<String>,
}

#[derive(Clone, Copy)]
enum Node {
    Top,
    Scorer,
    Entry,
    Trigger(TriggerNode),
}

impl Database {
    pub fn get(&self, name: &[u8]) -> Result<Option<&[Entry]>, String> {
        match self.entries.get(&c_string(name).to_ascii_lowercase()) {
            Some(Ok(entries)) => Ok(Some(entries)),
            Some(Err(why)) => Err(why.clone()),
            None if self.unread.is_empty() => Ok(None),
            None => Err(format!(
                "country scorer {} is unknown after {}",
                String::from_utf8_lossy(name),
                self.unread.join("; ")
            )),
        }
    }

    pub fn read_file(&mut self, input: &[u8], names: &KeyNames) {
        let mut seen: BTreeSet<Vec<u8>> = self.entries.keys().cloned().collect();
        let mut shaper = Shaper::new(Some(names));
        let mut current = None;
        let read = native::read(
            input,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(names),
            &mut |node, field| match node {
                Node::Top => {
                    let name = field.key.c_str().to_ascii_lowercase();
                    current = Some(name.clone());
                    if seen.insert(name) {
                        Shape::Object(Node::Scorer)
                    } else {
                        Shape::Skip
                    }
                }
                Node::Scorer => match field.keyword().as_deref() {
                    Some(b"targets") => Shape::Object(Node::Entry),
                    _ => Shape::Unsettled(format!(
                        "country scorer field {} is unported",
                        String::from_utf8_lossy(field.key.c_str())
                    )),
                },
                Node::Entry => match field.keyword().as_deref() {
                    Some(b"targets") => Shape::List,
                    Some(
                        b"target_array"
                        | b"country"
                        | b"state"
                        | b"targets_dynamic"
                        | b"target_non_existing"
                        | b"state_target",
                    ) => Shape::One,
                    Some(b"target_root_trigger" | b"target_trigger") => {
                        Shape::Body(Node::Trigger(TriggerNode::Trigger))
                    }
                    Some(b"score") => Shape::Object(Node::Trigger(TriggerNode::Mtth)),
                    _ => Shape::Unsettled(format!(
                        "country scorer target field {} is unported",
                        String::from_utf8_lossy(field.key.c_str())
                    )),
                },
                Node::Trigger(node) => shaper.shape(node, field).map(Node::Trigger),
            },
        );
        self.notes
            .extend(read.misreads.iter().map(ToString::to_string));
        for (item, notes) in read.items.into_iter().zip(read.item_misreads) {
            let name = c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase();
            let reasons = notes
                .iter()
                .map(ToString::to_string)
                .chain(self.unread.iter().cloned())
                .collect::<Vec<_>>();
            let value = if reasons.is_empty() {
                Ok(item
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|field| field.keyword_is("targets"))
                    .map(entry)
                    .collect())
            } else {
                Err(reasons.join("; "))
            };
            self.entries.entry(name).or_insert(value);
        }
        if let Some(why) = read.unsettled {
            let why = why.to_string();
            self.notes.push(why.clone());
            self.unread.push(why.clone());
            if let Some(current) = current {
                self.entries.insert(current, Err(why));
            }
        }
    }
}

fn entry(item: &Item) -> Entry {
    let mut out = Entry::default();
    read_fields(item.value.items().unwrap_or_default(), &mut out);
    out
}

pub fn script_targets(fields: &[Item]) -> ScriptTargets {
    let mut out = Entry::default();
    read_fields(fields, &mut out);
    out.targets
}

fn read_fields(fields: &[Item], out: &mut Entry) {
    for field in fields {
        let scalar = || field.value.scalar().unwrap_or_default().to_vec();
        match field.keyword().map(|key| key.folded()).as_deref() {
            Some(b"target_array") => out.targets.arrays.push(scalar()),
            Some(b"targets") => out.targets.targets.extend(
                field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .map(|target| target.value.scalar().unwrap_or_default().to_vec()),
            ),
            Some(b"state" | b"country") => out.targets.targets.push(scalar()),
            Some(b"target_root_trigger") => out
                .targets
                .root_limit
                .extend(field.value.items().unwrap_or_default().iter().cloned()),
            Some(b"target_trigger") => out
                .targets
                .target_limit
                .extend(field.value.items().unwrap_or_default().iter().cloned()),
            Some(b"score") => out
                .score
                .extend(field.value.items().unwrap_or_default().iter().cloned()),
            Some(b"targets_dynamic") => {
                if let Some(value) = content_reader_parser::parser::read_bool(&scalar()) {
                    out.targets.dynamic = value;
                }
            }
            Some(b"target_non_existing") => {
                if let Some(value) = content_reader_parser::parser::read_bool(&scalar()) {
                    out.targets.non_existing = value;
                }
            }
            Some(b"state_target") => out
                .targets
                .unported
                .push("state_target selection mode".into()),
            _ => {}
        }
    }
}

pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Database> {
    let mut database = Database::default();
    for path in content_reader_layers::script_files::paths(layers, "common/scorers/country/", Some(".txt"))? {
        database.read_file(&path.read(layers)?, names);
    }
    Ok(database)
}

#[cfg(test)]
mod tests;
