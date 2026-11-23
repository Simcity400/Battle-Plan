use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_script::{trigger_grammar::{Node, Shaper}, key_names::KeyNames, native::{self, Shape}};
use content_reader_parser::parser::{Item, c_string};

pub(crate) const COLLECTION_KEY_READER: u64 = 0x140A6A4C0;

#[derive(Clone, Debug, Default)]
pub struct Database {
    entries: BTreeMap<Vec<u8>, Result<Item, String>>,
    pub notes: Vec<String>,
    unread: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Empty,
    Countries { possible: bool },
    States,
    Scope,
    Constant(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    FactionMembers,
    OwnedStates,
    ControlledStates,
    CountryAndSubjects,
    Limit(Vec<Item>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Header {
    Empty,
    Key(&'static [u8]),
    Bound(Item),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub input: Input,
    pub operators: Vec<Operator>,
    pub header: Header,
}

impl Database {
    pub fn read_file(&mut self, input: &[u8], names: &KeyNames) {
        let mut shaper = Shaper::new(Some(names));
        let mut current = None;
        let read = native::read(
            input,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(names),
            &mut |node, field| {
                if matches!(node, Node::Top) {
                    current = Some(field.key.c_str().to_ascii_lowercase());
                    Shape::Object(Node::KeyReader(COLLECTION_KEY_READER))
                } else {
                    shaper.shape(node, field)
                }
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
            let parsed = if reasons.is_empty() {
                Ok(item)
            } else {
                Err(reasons.join("; "))
            };
            match self.entries.entry(name.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(parsed);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let _ = entry.insert(Err(format!(
                        "duplicate collection {}: native merge is unverified",
                        String::from_utf8_lossy(&name)
                    )));
                }
            }
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

pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Database> {
    let mut database = Database::default();
    for path in content_reader_layers::script_files::paths(layers, "common/collections/", Some(".txt"))? {
        database.read_file(&path.read(layers)?, names);
    }
    Ok(database)
}

pub fn compile(item: &Item, database: Option<&Database>) -> Result<Collection, String> {
    fn inner(
        item: &Item,
        database: Option<&Database>,
        active: &mut BTreeSet<Vec<u8>>,
    ) -> Result<Collection, String> {
        if active.len() >= 64 {
            return Err("collection expansion exceeds nesting guard of 64".into());
        }
        let mut input = item;
        let mut operators = Vec::new();
        let mut authored_name = None;
        if let Some(fields) = item.value.items() {
            for field in fields {
                if field.keyword_is("input") {
                    input = field;
                } else if field.keyword_is("name") {
                    authored_name = Some(field.clone());
                } else if field.keyword_is("operators") {
                    for op in field.value.items().unwrap_or_default() {
                        if op.keyword_is("limit") {
                            operators.push(Operator::Limit(
                                op.value.items().unwrap_or_default().to_vec(),
                            ));
                        } else if op.key.is_none() {
                            match op
                                .value
                                .scalar()
                                .map(|value| c_string(value).to_ascii_lowercase())
                                .as_deref()
                            {
                                Some(b"faction_members") => {
                                    operators.push(Operator::FactionMembers)
                                }
                                Some(b"owned_states") => operators.push(Operator::OwnedStates),
                                Some(b"controlled_states") => {
                                    operators.push(Operator::ControlledStates)
                                }
                                Some(b"country_and_all_subjects") => {
                                    operators.push(Operator::CountryAndSubjects)
                                }
                                _ => {
                                    return Err(format!(
                                        "collection operator {} is reported and skipped",
                                        String::from_utf8_lossy(
                                            op.value.scalar().unwrap_or_default()
                                        )
                                    ));
                                }
                            }
                        } else {
                            return Err(format!(
                                "collection operator {} has an unsupported assignment",
                                String::from_utf8_lossy(op.key.as_deref().unwrap_or_default())
                            ));
                        }
                    }
                }
            }
        }
        let text = input.value.scalar().unwrap_or_default();
        let folded = c_string(text).to_ascii_lowercase();
        let mut inherited = match folded.as_slice() {
            b"game:all_countries" => Collection {
                input: Input::Countries { possible: false },
                operators: Vec::new(),
                header: Header::Key(b"COLLECTION_ALL_COUNTRIES"),
            },
            b"game:all_possible_countries" => Collection {
                input: Input::Countries { possible: true },
                operators: Vec::new(),
                header: Header::Key(b"COLLECTION_ALL_POSSIBLE_COUNTRIES"),
            },
            b"game:all_states" => Collection {
                input: Input::States,
                operators: Vec::new(),
                header: Header::Key(b"COLLECTION_ALL_STATES"),
            },
            b"game:scope" => Collection {
                input: Input::Scope,
                operators: Vec::new(),
                header: Header::Empty,
            },
            _ if folded.starts_with(b"constant:") => Collection {
                input: Input::Constant(folded[9..].to_vec()),
                operators: Vec::new(),
                header: Header::Empty,
            },
            _ if folded.starts_with(b"collection:") => {
                let name = folded[11..].to_vec();
                if !active.insert(name.clone()) {
                    return Err(format!(
                        "collection {} recursion",
                        String::from_utf8_lossy(&name)
                    ));
                }
                let database = database.ok_or("named collection content is unavailable")?;
                let result = match database.entries.get(&name) {
                    Some(Ok(item)) => inner(item, Some(database), active)?,
                    Some(Err(why)) => return Err(why.clone()),
                    None if database.unread.is_empty() => Collection {
                        input: Input::Empty,
                        operators: Vec::new(),
                        header: Header::Empty,
                    },
                    None => {
                        return Err(format!(
                            "collection {} is unknown after {}",
                            String::from_utf8_lossy(&name),
                            database.unread.join("; ")
                        ));
                    }
                };
                active.remove(&name);
                result
            }
            b"" => Collection {
                input: Input::Empty,
                operators: Vec::new(),
                header: Header::Empty,
            },
            _ => {
                return Err(format!(
                    "collection input {} is unsupported",
                    String::from_utf8_lossy(&folded)
                ));
            }
        };
        let named = folded.starts_with(b"collection:");
        if !matches!(inherited.input, Input::Empty) || !named {
            inherited.operators.extend(operators);
        }
        if let Some(name) = authored_name {
            inherited.header = Header::Bound(name);
        } else if !named && matches!(inherited.header, Header::Empty) {
            inherited.header = match inherited.operators.first() {
                Some(Operator::FactionMembers) => Header::Key(b"COLLECTION_OP_FACTION_MEMBERS"),
                Some(Operator::OwnedStates) => Header::Key(b"COLLECTION_OP_OWNED_STATES"),
                Some(Operator::ControlledStates) => Header::Key(b"COLLECTION_OP_CONTROLLED_STATES"),
                Some(Operator::CountryAndSubjects) => {
                    Header::Key(b"COLLECTION_OP_COUNTRY_AND_ALL_SUBJECTS")
                }
                _ => Header::Empty,
            };
        }
        Ok(inherited)
    }
    inner(item, database, &mut BTreeSet::new())
}

#[cfg(test)]
mod tests;
