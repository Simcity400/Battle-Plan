//! Doctrine names and tactic-source inventories, through the content stack.

use crate::{
    ResolvedLayers,
    key_names::KeyNames,
    parser::{
        self,
        native::{self, Shape},
    },
    tokens::Token,
};
use std::{collections::BTreeMap, io};

mod names;
mod sources;
pub use names::Name;
pub use sources::{DefinitionSources, RewardSource, TacticSources};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub name: Vec<u8>,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Database {
    pub entries: BTreeMap<Vec<u8>, Folder>,
    pub grand: BTreeMap<Token, Name>,
    pub sub: BTreeMap<Token, Name>,
    pub tracks: BTreeMap<Token, Name>,
    pub tactic_sources: TacticSources,
    /// Technology descriptions share this already cached family database.
    pub technologies: Option<crate::technology::Catalog>,
    pub notes: Vec<String>,
    unread: Option<String>,
}

impl Database {
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

    /// Folder token lookup used by mastery selectors.
    pub fn folder_exact(&self, token: &[u8]) -> Result<Option<&Folder>, String> {
        self.lookup(parser::c_string(token), false)
    }

    /// The bonus reader first finds the exact folder token, then removes
    /// its native `_doctrine` suffix before the category fallback.
    pub fn folder(&self, token: &[u8]) -> Result<Option<&Folder>, String> {
        self.lookup(parser::c_string(token), true)
    }

    fn lookup(&self, token: &[u8], strip_suffix: bool) -> Result<Option<&Folder>, String> {
        if let Some(why) = &self.unread {
            return Err(why.clone());
        }
        let folder = self.entries.get(&token.to_ascii_lowercase()).or_else(|| {
            // The suffix check tests authored bytes before token lookup.
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
                    crate::effect_keys::bound_localization_shape(field).map(|()| Node::Folder)
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
        // Repeated tokens read into the existing object; omitted fields remain.
        let folder = database.entries.entry(token).or_default();
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("name") {
                if let Some(value) = field.value.scalar() {
                    folder.name = parser::c_string(value).to_vec();
                    folder.refused =
                        match crate::gui::native::FormattedLocalization::parse(&folder.name) {
                            crate::gui::native::FormattedLocalization::Key(_) => None,
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
                    crate::DiagnosticKind::Unsupported
                        | crate::DiagnosticKind::Unsettled
                        | crate::DiagnosticKind::Refused
                )
            })
            .map(ToString::to_string)
            .collect();
        if !blockers.is_empty() {
            folder.refused = Some(format!("{source}: {}", blockers.join("; ")));
        }
    }
}

/// Folder loader enumerates only this directory's files in lexer mode 0.
pub fn read(layers: &ResolvedLayers) -> io::Result<Database> {
    let names = layers.key_names()?;
    let mut database = Database::default();
    for path in layers.directory_files("common/doctrines/folders", ".txt")? {
        append(&mut database, &layers.read(&path)?, &path, &names);
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
