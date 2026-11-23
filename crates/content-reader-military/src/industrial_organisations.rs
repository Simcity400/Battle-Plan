use content_reader_layers::ResolvedLayers;
use content_reader_script::{key_names::KeyNames, native::{self, Shape}};
use content_reader_parser::parser::{Item, c_string};
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
    created: BTreeMap<Vec<u8>, usize>,
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
            native::Top::Persistent(Reader::File),
            Some(names),
            &mut |reader, field| match reader {
                Reader::File => {
                    let name = field.key.c_str().to_ascii_lowercase();
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
                        Shape::Trigger
                    } else if field.key.is_keyword("include") {
                        Shape::One
                    } else if field.key.is_keyword("delete_included_values") {
                        Shape::List
                    } else if field.key.is_keyword("name") {
                        Shape::One
                    } else if field.key.is_keyword("initial_trait") {
                        Shape::Object(Reader::InitialTrait)
                    } else {
                        Shape::SkipIfOpen
                    }
                }
                Reader::InitialTrait => {
                    if field.key.is_keyword("name") {
                        Shape::One
                    } else if field.key.is_keyword("delete_included_values") {
                        Shape::List
                    } else {
                        Shape::SkipIfOpen
                    }
                }
            },
        );
        self.notes.extend(duplicate_notes);
        for (item, notes) in read.items.into_iter().zip(read.item_misreads) {
            let name = c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase();
            if self.templates.contains_key(&name) || self.refusals.contains_key(&name) {
                continue;
            }
            let next = self.created.len();
            self.created.entry(name.clone()).or_insert(next);
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
                                .is_some_and(|name| c_string(name).eq_ignore_ascii_case(b"name"))
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
                                .is_some_and(|name| c_string(name).eq_ignore_ascii_case(b"name"))
                        });
                    template.delete_initial_trait |=
                        field.value.items().unwrap_or_default().iter().any(|item| {
                            item.value.scalar().is_some_and(|name| {
                                c_string(name).eq_ignore_ascii_case(b"initial_trait")
                            })
                        });
                    template.delete_visible |=
                        field.value.items().unwrap_or_default().iter().any(|value| {
                            value.value.scalar().is_some_and(|value| {
                                c_string(value).eq_ignore_ascii_case(b"visible")
                            })
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
                let next = self.created.len();
                self.created.entry(name.clone()).or_insert(next);
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

    pub fn display_fields(&self, token: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
        let name = token.to_ascii_lowercase();
        let template = self.template(&name)?;
        let local = (template.name.clone(), template.initial_trait_name.clone());
        let Some(include) = &template.include else {
            return Ok(local);
        };
        if *include == name {
            return Err(format!(
                "MIO {} includes itself (count: 1)",
                String::from_utf8_lossy(token)
            ));
        }
        let parent = match self.template(include) {
            Ok(_) if self.created.get(include) < self.created.get(&name) => {
                self.display_fields(include)?
            }
            Ok(parent) => (parent.name.clone(), parent.initial_trait_name.clone()),
            Err(_) if self.unread.is_empty() && !self.refusals.contains_key(include) => {
                return Ok(local);
            }
            Err(why) => return Err(why),
        };
        let (mut name, mut initial) = local;
        if name.is_empty() && !template.delete_name {
            name = parent.0;
        }
        if !template.delete_initial_trait
            && let Some(inherited) = parent.1
            && initial
                .as_ref()
                .is_none_or(|own| own.is_empty() && !template.delete_initial_name)
        {
            initial = Some(inherited);
        }
        Ok((name, initial))
    }
}

pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Database> {
    let mut database = Database::default();
    for path in content_reader_layers::script_files::paths(
        layers,
        "common/military_industrial_organization/organizations/",
        Some(".txt"),
    )? {
        let before = database.notes.len();
        if let Err(why) = database.read_file(&path.read(layers)?, names) {
            database.notes.push(why);
        }
        for note in &mut database.notes[before..] {
            *note = format!("{path}: {note}");
        }
    }
    database.policies = read_policies(layers, names, &mut database.notes)?;
    Ok(database)
}

pub fn read_policies(
    layers: &ResolvedLayers,
    names: &KeyNames,
    notes: &mut Vec<String>,
) -> io::Result<policies::Catalog> {
    let mut catalog = policies::Catalog::default();
    for path in content_reader_layers::script_files::paths(
        layers,
        "common/military_industrial_organization/policies/",
        Some(".txt"),
    )? {
        let before = catalog.notes.len();
        if let Err(why) = catalog.read_file(&path.read(layers)?, names) {
            notes.push(format!("{path}: {why}"));
        }
        notes.extend(
            catalog.notes[before..]
                .iter()
                .map(|why| format!("{path}: {why}")),
        );
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests;
