use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_layers::content::diagnostics::Once as OnceLock;
use content_reader_layers::ResolvedLayers;
use content_reader_script::{key_names::{KeyNames, Point}, modifiers::{self, Undecided}, native::{self, Shape}};
use content_reader_parser::parser::{self, Item};
use std::{collections::BTreeMap, io, sync::Mutex};

type DescriptionValue = Result<(Option<String>, modifiers::Modifiers), String>;
type DescriptionFiles = Result<(KeyNames, Vec<(String, File)>), String>;

#[derive(Default)]
pub struct Descriptions {
    files: OnceLock<DescriptionFiles>,
    values: Mutex<BTreeMap<String, DescriptionValue>>,
}

impl Descriptions {
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_blocks(blocks: impl IntoIterator<Item = (String, modifiers::Modifiers)>) -> Self {
        Self {
            values: Mutex::new(
                blocks
                    .into_iter()
                    .map(|(name, block)| (name.clone(), Ok((Some(name), block))))
                    .collect(),
            ),
            ..Self::default()
        }
    }

    pub fn get(&self, layers: &ResolvedLayers, name: &str) -> DescriptionValue {
        if let Some(value) = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
        {
            let _ = self.files.get();
            return value.clone();
        }
        let value = (|| {
            let (names, files) = self
                .files
                .get_or_init(|| {
                    let mut names = layers
                        .key_names()
                        .map_err(|why| why.to_string())?
                        .at(Point::StaticModifiers);
                    names.effects.modifiers = names.effects.early_modifiers.clone();
                    names.effects.modifiers_gap = names.effects.early_modifiers_gap.clone();
                    let registry = |key: &[u8]| modifiers::registered(key, Some(&names));
                    let files = layer_files(layers)
                        .map_err(|why| why.to_string())?
                        .into_iter()
                        .map(|(path, bytes)| {
                            (
                                path,
                                read_file_with_registry(&bytes, Some(&names), &registry),
                            )
                        })
                        .collect();
                    Ok((names, files))
                })
                .as_ref()
                .map_err(Clone::clone)?;
            let (mut values, _) = resolve_parsed(
                files.iter().map(|(path, file)| (path.as_str(), file)),
                &[name],
                (None, modifiers::Modifiers::default()),
                |name, items| {
                    let mut block = stored_block(items)?;
                    description_values(&mut block, names)?;
                    Ok((Some(name.to_owned()), block))
                },
            );
            values
                .pop()
                .expect("one requested modifier")
                .map_err(|why| format!("{}: static modifier {}: {}", why.file, name, why.reason))
        })();
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name.to_owned(), value.clone());
        value
    }

    pub fn notes(&self) -> Vec<String> {
        self.files
            .get()
            .and_then(|files| files.as_ref().ok())
            .into_iter()
            .flat_map(|(_, files)| files)
            .flat_map(|(path, file)| file.notes.iter().map(move |note| format!("{path}: {note}")))
            .collect()
    }
}

fn description_values(block: &mut modifiers::Modifiers, names: &KeyNames) -> Result<(), String> {
    for value in &block.values {
        if modifiers::registered(value.name.as_bytes(), Some(names)).is_none() {
            return Err(format!(
                "modifier {} registry is unavailable at static load",
                value.key
            ));
        }
    }
    block
        .values
        .retain(|value| modifiers::registered(value.name.as_bytes(), Some(names)) == Some(true));
    for child in &mut block.children {
        description_values(child, names)?;
    }
    Ok(())
}

pub const DIRECTORY: &str = "common/modifiers";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub modifier: &'a str,
    pub key: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub file: String,
    pub modifier: String,
    pub key: String,
    pub reason: String,
}

pub type Resolved = Result<i64, Refusal>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    pub values: Vec<Resolved>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocks {
    pub blocks: Vec<Result<modifiers::Modifiers, Refusal>>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identifiers {
    pub identifiers: Vec<Result<Option<String>, Refusal>>,
    pub notes: Vec<String>,
}

pub fn read_identifiers(layers: &ResolvedLayers, names: &[&str]) -> io::Result<Identifiers> {
    let files = parsed_files(layers)?;
    let (identifiers, notes) = resolve_parsed(
        files
            .iter()
            .map(|(path, file)| (path.as_str(), file)),
        names,
        None,
        |name, items| {
            stored_block(items)?;
            Ok(Some(name.to_owned()))
        },
    );
    Ok(Identifiers { identifiers, notes })
}

pub fn read(layers: &ResolvedLayers, requests: &[Request<'_>]) -> io::Result<Values> {
    let names: Vec<_> = requests.iter().map(|request| request.modifier).collect();
    Ok(values(read_blocks(layers, &names)?, requests))
}

pub fn read_blocks(layers: &ResolvedLayers, names: &[&str]) -> io::Result<Blocks> {
    let files = parsed_files(layers)?;
    let (blocks, notes) = resolve_parsed(
        files
            .iter()
            .map(|(path, file)| (path.as_str(), file)),
        names,
        modifiers::Modifiers::default(),
        |_, items| stored_block(items),
    );
    Ok(Blocks { blocks, notes })
}

fn parsed_files(layers: &ResolvedLayers) -> io::Result<std::sync::Arc<Vec<(String, File)>>> {
    layers.recorded_product("static modifier definitions", 1, &[], |layers| {
        let key_names = layers.key_names()?.at(Point::StaticModifiers);
        layers.directory_files(DIRECTORY, ".txt")?.into_iter().map(|path| {
            let bytes = path.read_uncovered(layers)?;
            Ok((path.to_string(), read_file(&bytes, Some(&key_names))))
        }).collect()
    })
}

fn layer_files(layers: &ResolvedLayers) -> io::Result<Vec<(String, content_reader_layers::content::Bytes)>> {
    layers.directory_files(DIRECTORY, ".txt")?.into_iter().map(|path| {
        path.read_uncovered(layers).map(|bytes| (path.to_string(), bytes))
    }).collect()
}

struct File {
    definitions: Vec<Item>,
    stop: Option<String>,
    notes: Vec<String>,
}

#[derive(Clone, Copy)]
enum Node {
    Top,
    Modifier(modifiers::Node),
}

fn read_file(bytes: &[u8], key_names: Option<&KeyNames>) -> File {
    read_file_with_registry(bytes, key_names, modifiers::UNKNOWN)
}

fn read_file_with_registry(
    bytes: &[u8],
    key_names: Option<&KeyNames>,
    registry: modifiers::Registry<'_>,
) -> File {
    let mut undecided: Vec<Undecided> = Vec::new();
    let mut entries = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        key_names,
        &mut |node, field| {
            let shape = match node {
                Node::Top => {
                    entries += 1;
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                Node::Modifier(modifier) => {
                    modifiers::shape(modifier, field, registry, &mut undecided).map(Node::Modifier)
                }
            };
            if stop.is_none()
                && let Some(first) = undecided.first()
            {
                stop = Some((entries.saturating_sub(1), first.to_string()));
            }
            shape
        },
    );
    if stop.is_none()
        && let Some(unsettled) = &read.unsettled
    {
        stop = Some((entries.saturating_sub(1), unsettled.to_string()));
    }
    let known = stop.as_ref().map_or(read.items.len(), |(at, _)| *at);
    File {
        definitions: read.items.into_iter().take(known).collect(),
        stop: stop.map(|(_, reason)| reason),
        notes: read.misreads.iter().map(ToString::to_string).collect(),
    }
}

#[cfg(test)]
fn resolve<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    requests: &[Request<'_>],
    key_names: Option<&KeyNames>,
) -> Values {
    let names: Vec<_> = requests.iter().map(|request| request.modifier).collect();
    values(resolve_blocks(files, &names, key_names), requests)
}

fn values(Blocks { blocks, notes }: Blocks, requests: &[Request<'_>]) -> Values {
    let values = blocks
        .into_iter()
        .zip(requests)
        .map(|(block, request)| {
            block
                .map(|block| {
                    let name = request.key.to_ascii_lowercase();
                    block
                        .values
                        .iter()
                        .find(|value| value.name == name)
                        .map_or(0, |value| value.value)
                })
                .map_err(|mut refusal| {
                    refusal.key = request.key.to_owned();
                    refusal
                })
        })
        .collect();
    Values { values, notes }
}

#[cfg(test)]
fn resolve_blocks<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    names: &[&str],
    key_names: Option<&KeyNames>,
) -> Blocks {
    let (blocks, notes) = resolve_with(
        files,
        names,
        key_names,
        modifiers::Modifiers::default(),
        |_, items| stored_block(items),
    );
    Blocks { blocks, notes }
}

#[cfg(test)]
fn resolve_with<'a, T: Clone>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    names: &[&str],
    key_names: Option<&KeyNames>,
    absent: T,
    stored: impl FnMut(&str, &[Item]) -> Result<T, String>,
) -> (Vec<Result<T, Refusal>>, Vec<String>) {
    let files: Vec<_> = files
        .into_iter()
        .map(|(path, bytes)| (path, read_file(bytes, key_names)))
        .collect();
    resolve_parsed(
        files.iter().map(|(path, file)| (*path, file)),
        names,
        absent,
        stored,
    )
}

fn resolve_parsed<'a, T: Clone>(
    files: impl IntoIterator<Item = (&'a str, &'a File)>,
    names: &[&str],
    absent: T,
    mut stored: impl FnMut(&str, &[Item]) -> Result<T, String>,
) -> (Vec<Result<T, Refusal>>, Vec<String>) {
    let mut blocks = vec![Ok(absent); names.len()];
    let mut notes = Vec::new();
    for (path, file) in files {
        let refuse = |name: &str, reason: String| Refusal {
            file: path.to_owned(),
            modifier: name.to_owned(),
            key: "<whole block>".to_owned(),
            reason,
        };
        for definition in &file.definitions {
            let name = parser::c_string(definition.key.as_deref().unwrap_or_default());
            let items = definition.value.items().unwrap_or_default();
            let renamed: Vec<&[u8]> = items
                .iter()
                .filter(|item| item.keyword_is("key") || item.keyword_is("tag"))
                .map(|item| item.value.scalar().unwrap_or_default())
                .collect();
            for (block, &requested) in blocks.iter_mut().zip(names) {
                let modifier = requested.as_bytes();
                if !renamed.is_empty() && (name == modifier || renamed.contains(&modifier)) {
                    *block = Err(refuse(
                        requested,
                        format!(
                            "definition {} has a key or tag field, which renames it after it is bucketed under its first name; the bucket layout that decides the lookup is not modelled",
                            String::from_utf8_lossy(name)
                        ),
                    ));
                } else if name == modifier {
                    *block = stored(requested, items).map_err(|reason| refuse(requested, reason));
                }
            }
        }
        if let Some(stop) = &file.stop {
            for (block, &requested) in blocks.iter_mut().zip(names) {
                *block = Err(refuse(
                    requested,
                    format!("{stop}; any definition the rest of the file makes is unknown"),
                ));
            }
        }
        notes.extend(file.notes.iter().map(|note| format!("{path}: {note}")));
    }
    (blocks, notes)
}

fn stored_block(items: &[Item]) -> Result<modifiers::Modifiers, String> {
    let definition = modifiers::targeted(items).modifiers;
    if let Some(cause) = definition.unheld.cause() {
        return Err(format!(
            "the definition holds text the port cannot keep: {cause}"
        ));
    }
    Ok(definition)
}

#[cfg(test)]
mod tests;
