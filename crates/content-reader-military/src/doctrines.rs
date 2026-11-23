use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::ResolvedLayers;
use content_reader_script::{key_names::KeyNames, native::{self, Shape}};
use content_reader_parser::{parser, tokens::Token};
use std::{collections::BTreeMap, io};

mod names;
mod sources;
pub use names::Name;
pub use sources::{Benefits, DefinitionSources, RewardSource, TacticSources};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub name: Vec<u8>,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    pub token: Vec<u8>,
    pub icon: Vec<u8>,
    pub folder: Option<Token>,
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    pub entries: BTreeMap<Vec<u8>, Folder>,
    pub grand: BTreeMap<Token, Name>,
    pub sub: BTreeMap<Token, Name>,
    pub tracks: BTreeMap<Token, Name>,
    pub tactic_sources: TacticSources,
    pub folder_order: Vec<Token>,
    pub folder_presentations: BTreeMap<Token, Presentation>,
    pub grand_presentations: BTreeMap<Token, Presentation>,
    pub sub_presentations: BTreeMap<Token, Presentation>,
    pub track_presentations: BTreeMap<Token, Presentation>,
    pub technologies: Option<crate::technology::Catalog>,
    pub notes: Vec<String>,
    unread: Option<String>,
}

impl Database {
    pub fn presentation(&self, kind: u8, token: &[u8]) -> Result<Option<&Presentation>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        let entries = match kind {
            0 => &self.grand_presentations,
            1 => &self.sub_presentations,
            _ => &self.track_presentations,
        };
        Ok(entries.get(&Token::c_str(token)))
    }

    pub fn folder_presentation(&self, token: &[u8]) -> Result<Option<&Presentation>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        Ok(self.folder_presentations.get(&Token::c_str(token)))
    }

    pub fn grand_in_folder(&self, folder: &[u8]) -> Result<Vec<&Presentation>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        let folder = Token::c_str(folder);
        let mut seen = std::collections::BTreeSet::new();
        Ok(self
            .tactic_sources
            .grand_order
            .iter()
            .filter(|token| seen.insert(*token))
            .filter_map(|token| self.grand_presentations.get(token))
            .filter(|entry| entry.folder.as_ref() == Some(&folder))
            .collect())
    }

    pub fn name(&self, kind: u8, token: &[u8]) -> Result<Option<&Name>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        let entries = match kind {
            0 => &self.grand,
            1 => &self.sub,
            _ => &self.tracks,
        };
        let name = entries.get(&Token::c_str(token));
        if let Some(why) = name.and_then(|name| name.refusal.as_ref()) {
            return Err(why.clone());
        }
        Ok(name)
    }

    pub fn folder_exact(&self, token: &[u8]) -> Result<Option<&Folder>, String> {
        self.lookup(parser::c_string(token), false)
    }

    pub fn folder(&self, token: &[u8]) -> Result<Option<&Folder>, String> {
        self.lookup(parser::c_string(token), true)
    }

    fn lookup(&self, token: &[u8], strip_suffix: bool) -> Result<Option<&Folder>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        let folder = self.entries.get(&token.to_ascii_lowercase()).or_else(|| {
            strip_suffix
                .then_some(token)
                .and_then(|token| token.strip_suffix(b"_doctrine"))
                .and_then(|key| self.entries.get(&key.to_ascii_lowercase()))
        });
        if let Some(why) = folder.and_then(|folder| folder.refused.as_ref()) {
            Err(why.clone())
        } else {
            Ok(folder)
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Folder,
}

fn append(database: &mut Database, bytes: &[u8], source: &str, names: &KeyNames) {
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Folder),
            Node::Folder => match field.keyword().as_deref() {
                Some(b"name") => {
                    content_reader_script::effect_keys::bound_localization_shape(field).map(|()| Node::Folder)
                }
                Some(b"allowed") => Shape::Trigger,
                Some(b"sound" | b"ledger" | b"ledger_gfx" | b"tab_gfx" | b"color_frame") => {
                    Shape::One
                }
                _ => Shape::Skip,
            },
        },
    );
    database
        .notes
        .extend(read.misreads.iter().map(|note| format!("{source}: {note}")));
    if let Some(why) = &read.unsettled {
        database.unread = Some(format!("{source}: unread doctrine folders: {why}"));
    }
    for (index, item) in read.items.iter().enumerate() {
        let Token::Name(token) = Token::c_str(item.key.as_deref().unwrap_or_default()) else {
            database.unread = Some(format!(
                "{source}: doctrine folder token cannot be represented"
            ));
            continue;
        };
        let identity = Token::Name(token.clone());
        let presentation = database
            .folder_presentations
            .entry(identity.clone())
            .or_insert_with(|| {
                database.folder_order.push(identity);
                Presentation {
                    token: parser::c_string(item.key.as_deref().unwrap_or_default()).to_vec(),
                    ..Default::default()
                }
            });
        let folder = database.entries.entry(token).or_default();
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("tab_gfx")
                && let Some(value) = field.value.read_string()
            {
                presentation.icon = value.to_vec();
            }
            if field.keyword_is("name") {
                if let Some(value) = field.value.scalar() {
                    folder.name = parser::c_string(value).to_vec();
                    folder.refused =
                        match content_reader_script::localization_binding::FormattedLocalization::parse(&folder.name) {
                            content_reader_script::localization_binding::FormattedLocalization::Key(_) => None,
                            _ => Some(format!(
                                "{source}: doctrine folder {} formatted name is not ported",
                                String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                            )),
                        };
                } else {
                    folder.refused = Some(format!(
                        "{source}: doctrine folder {} bound name requires a description formatter",
                        String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
                    ));
                }
            }
        }
        let blockers: Vec<_> = read.item_misreads[index]
            .iter()
            .chain(&read.unattributed_misreads)
            .filter(|note| {
                matches!(
                    note.kind,
                    content_reader_parser::DiagnosticKind::Unsupported
                        | content_reader_parser::DiagnosticKind::Unsettled
                        | content_reader_parser::DiagnosticKind::Refused
                )
            })
            .map(ToString::to_string)
            .collect();
        if !blockers.is_empty() {
            folder.refused = Some(format!("{source}: {}", blockers.join("; ")));
        }
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in layers.directory_files("common/doctrines/folders", ".txt")? {
        append(&mut database, &path.read(layers)?, &path.text(), &names);
    }
    names::read(layers, &names, &mut database)?;
    match crate::technology::Catalog::read(layers, &names) {
        Ok(catalog) => {
            database.notes.extend(catalog.notes.iter().cloned());
            database.technologies = Some(catalog);
        }
        Err(why) => database.notes.push(format!(
            "Technology description catalogue unavailable: {why}"
        )),
    }
    Ok(database)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod layered_read_tests {
use content_reader_layers::ResolvedLayers;
use content_reader_layers::test_support::Stack;

#[test]
fn content_stack_uses_effective_files_and_first_definitions() {
    content_reader_script::test_game::enter();
    let fixture = Stack::new("effective-decisions-and-doctrine");
    let base = fixture.base();
    let overlay = fixture.addon();
    for folder in [&base, &overlay] {
        std::fs::create_dir_all(folder.join("common/decisions/categories")).unwrap();
        std::fs::create_dir_all(folder.join("common/doctrines/folders")).unwrap();
    }
    std::fs::write(base.join("common/decisions/categories/a.txt"), b"cat={}").unwrap();
    std::fs::write(base.join("common/decisions/a.txt"), b"cat={x={name=BASE}}").unwrap();
    std::fs::write(
        overlay.join("common/decisions/a.txt"),
        b"cat={x={name=OVERLAY}}",
    )
    .unwrap();
    std::fs::write(
        overlay.join("common/decisions/b.txt"),
        b"cat={x={name=IGNORED}} missing={y={name=NO}}",
    )
    .unwrap();
    std::fs::write(
        base.join("common/doctrines/folders/a.txt"),
        b"land={name=BASE}",
    )
    .unwrap();
    std::fs::write(
        overlay.join("common/doctrines/folders/b.txt"),
        b"land={name=OVERLAY}",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let database = content_reader_politics::decisions::read(&layers).unwrap();
    assert_eq!(database.decision(b"x").unwrap().unwrap().name, b"OVERLAY");
    assert!(database.decision(b"y").unwrap().is_none());
    assert!(
        database
            .notes
            .iter()
            .any(|note| note.contains("not registered"))
    );
    let folders = crate::doctrines::read(&layers).unwrap();
    assert_eq!(
        folders.folder(b"land_doctrine").unwrap().unwrap().name,
        b"OVERLAY"
    );
}
}
