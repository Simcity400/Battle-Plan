//! Visibility projection of COrganisationTemplate (ReadKey@140A44C00).
//! Directory literal 1427076A8, loaded by CBuildingDatabase::load@14018B610.
use crate::{
    ResolvedLayers,
    key_names::KeyNames,
    parser::{
        Item, c_string,
        native::{self, Shape},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub mod policies;

#[derive(Default, Clone, Debug)]
pub struct Template {
    pub token: Vec<u8>,
    pub include: Option<Vec<u8>>,
    pub delete_visible: bool,
    pub visible: Vec<Item>,
    pub name: Vec<u8>,
    pub initial_trait_name: Option<Vec<u8>>,
    pub delete_name: bool,
    pub delete_initial_trait: bool,
    pub delete_initial_name: bool,
}

#[derive(Default, Clone, Debug)]
pub struct Database {
    pub templates: BTreeMap<Vec<u8>, Template>,
    pub policies: policies::Catalog,
    pub notes: Vec<String>,
    refusals: BTreeMap<Vec<u8>, String>,
    unread: Vec<String>,
}

#[derive(Clone, Copy)]
enum Reader {
    File,
    Template,
    InitialTrait,
}

impl Database {
    pub fn read_file(&mut self, bytes: &[u8], names: &KeyNames) -> Result<(), String> {
        let mut seen: BTreeSet<_> = self
            .templates
            .keys()
            .chain(self.refusals.keys())
            .cloned()
            .collect();
        let mut current = None;
        let mut duplicate_notes = Vec::new();
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Reader::File),
            Some(names),
            &mut |reader, field| match reader {
                Reader::File => {
                    let name = field.key.c_str().to_ascii_lowercase();
                    // Startup 140167C50 keeps the first token; 1424ACF90 skips duplicates.
                    if seen.insert(name.clone()) {
                        current = Some(name);
                        Shape::Object(Reader::Template)
                    } else {
                        current = None;
                        duplicate_notes.push(format!(
                            "industrial organisation template {} at line {}: duplicate definition skipped (count: 1)",
                            String::from_utf8_lossy(&name),
                            field.key.line
                        ));
                        Shape::Skip
                    }
                }
                Reader::Template => {
                    if field.key.is_keyword("visible") {
                        Shape::Trigger // +0x2C8, 140A44C00, mask 0x200.
                    } else if field.key.is_keyword("include") {
                        Shape::One // Dynamic token +0x10, 140A44C00.
                    } else if field.key.is_keyword("delete_included_values") {
                        Shape::List // Token list +0x18, 1403B7180.
                    } else if field.key.is_keyword("name") {
                        Shape::One // 140A44C00 +0x30.
                    } else if field.key.is_keyword("initial_trait") {
                        Shape::Object(Reader::InitialTrait) // 140A44C00 +0xB0.
                    } else {
                        Shape::SkipIfOpen // Other template fields do not decide visibility.
                    }
                }
                Reader::InitialTrait => {
                    if field.key.is_keyword("name") {
                        Shape::One // 140A41020 +0x18.
                    } else if field.key.is_keyword("delete_included_values") {
                        Shape::List // 140A41020 +0x3B0.
                    } else {
                        Shape::SkipIfOpen
                    }
                }
            },
        );
        self.notes.extend(duplicate_notes);
        for (item, notes) in read.items.into_iter().zip(read.item_misreads) {
            let name = c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase();
            // Skipped entries remain in the tree; keep the first definition (140167C50).
            if self.templates.contains_key(&name) || self.refusals.contains_key(&name) {
                continue;
            }
            let why: Vec<_> = notes
                .iter()
                .map(ToString::to_string)
                .chain(self.unread.iter().cloned())
                .collect();
            if !why.is_empty() {
                self.refusals.insert(name, why.join("; "));
                continue;
            }
            let mut template = Template {
                token: c_string(item.key.as_deref().unwrap_or_default()).to_vec(),
                ..Default::default()
            };
            for field in item.value.items().unwrap_or_default() {
                if field.keyword_is("name") {
                    template.name = field.value.read_string().unwrap_or_default().to_vec();
                } else if field.keyword_is("initial_trait") {
                    template.initial_trait_name = Some(
                        field
                            .value
                            .items()
                            .unwrap_or_default()
                            .iter()
                            .rfind(|field| field.keyword_is("name"))
                            .and_then(|field| field.value.read_string())
                            .unwrap_or_default()
                            .to_vec(),
                    );
                    template.delete_initial_name = field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|field| field.keyword_is("delete_included_values"))
                        .flat_map(|field| field.value.items().unwrap_or_default())
                        .any(|item| {
                            item.value
                                .scalar()
                                .is_some_and(|name| name.eq_ignore_ascii_case(b"name"))
                        });
                }
                if field.keyword_is("include") {
                    template.include = Some(
                        c_string(field.value.scalar().unwrap_or_default()).to_ascii_lowercase(),
                    );
                } else if field.keyword_is("visible") {
                    template
                        .visible
                        .extend(field.value.items().unwrap_or_default().iter().cloned());
                } else if field.keyword_is("delete_included_values") {
                    template.delete_name |=
                        field.value.items().unwrap_or_default().iter().any(|item| {
                            item.value
                                .scalar()
                                .is_some_and(|name| name.eq_ignore_ascii_case(b"name"))
                        });
                    template.delete_initial_trait |=
                        field.value.items().unwrap_or_default().iter().any(|item| {
                            item.value
                                .scalar()
                                .is_some_and(|name| name.eq_ignore_ascii_case(b"initial_trait"))
                        });
                    template.delete_visible |=
                        field.value.items().unwrap_or_default().iter().any(|value| {
                            value
                                .value
                                .scalar()
                                .is_some_and(|value| value.eq_ignore_ascii_case(b"visible"))
                        });
                }
            }
            self.templates.insert(name, template);
        }
        self.notes
            .extend(read.unattributed_misreads.iter().map(ToString::to_string));
        if let Some(why) = read.unsettled {
            let why = why.to_string();
            if let Some(name) = current {
                self.templates.remove(&name);
                self.refusals.insert(name, why.clone());
            }
            self.unread.push(why.clone());
            return Err(why);
        }
        Ok(())
    }

    pub fn template(&self, token: &[u8]) -> Result<&Template, String> {
        let name = token.to_ascii_lowercase();
        if let Some(why) = self.refusals.get(&name) {
            return Err(format!(
                "industrial organisation template {}: {why}",
                String::from_utf8_lossy(token)
            ));
        }
        self.templates.get(&name).ok_or_else(|| {
            format!(
                "industrial organisation template {} is not defined{}",
                String::from_utf8_lossy(token),
                if self.unread.is_empty() {
                    String::new()
                } else {
                    format!(": {}", self.unread.join("; "))
                }
            )
        })
    }

    /// Include passes 140A45170/140A417D0 preserve nonempty local values and
    /// reparent the copied initial trait to the child organisation.
    pub fn display_fields(&self, token: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
        let mut token = token.to_ascii_lowercase();
        let mut visited = BTreeSet::new();
        let mut name = Vec::new();
        let mut initial = None;
        let mut deleted_name = false;
        let mut deleted_initial = false;
        loop {
            if !visited.insert(token.clone()) {
                return Err("MIO include cycle (140A417D0)".into());
            }
            let template = self.template(&token)?;
            if name.is_empty() && !deleted_name {
                name.clone_from(&template.name);
            }
            if initial.as_ref().is_none_or(Vec::is_empty)
                && !deleted_initial
                && let Some(value) = &template.initial_trait_name
            {
                initial = Some(value.clone());
            }
            deleted_name |= template.delete_name;
            deleted_initial |= template.delete_initial_trait || template.delete_initial_name;
            let Some(parent) = &template.include else {
                break;
            };
            token.clone_from(parent);
        }
        Ok((name, initial))
    }
}

pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Database> {
    let mut database = Database::default();
    for path in crate::script_files::paths(
        layers,
        "common/military_industrial_organization/organizations/",
        Some(".txt"),
    ) {
        let before = database.notes.len();
        if let Err(why) = database.read_file(&layers.read(&path)?, names) {
            database.notes.push(why);
        }
        for note in &mut database.notes[before..] {
            *note = format!("{path}: {note}");
        }
    }
    // CBuildingDatabase::load 14018B610, directory literal 142707790.
    for path in crate::script_files::paths(
        layers,
        "common/military_industrial_organization/policies/",
        Some(".txt"),
    ) {
        let before = database.policies.notes.len();
        if let Err(why) = database.policies.read_file(&layers.read(&path)?, names) {
            database.notes.push(format!("{path}: {why}"));
        }
        database.notes.extend(
            database.policies.notes[before..]
                .iter()
                .map(|why| format!("{path}: {why}")),
        );
    }
    Ok(database)
}

#[cfg(test)]
mod tests;
