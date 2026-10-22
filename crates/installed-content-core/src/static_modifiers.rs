//! Named static modifiers from `common/modifiers`, read as the engine loads them.
//!
//! The loader enumerates `.txt` files without recursion through
//! [`ResolvedLayers::directory_files`]: the merged directory of the content
//! stack, `replace_path` honoured, files in byte order of their names,
//! no nested files.
//!
//! Each file gets its own lexer in mode 0 and its own reader through
//! [`native`], so `@name` definitions are the file's own and a reference
//! resolves through the reader's table. Files are read until their end:
//! every top-level key constructs a new static modifier named by the key's
//! text up to its NUL, pushed to the front of its hash bucket. Repeats
//! are reported. Definitions use [`modifiers::Node::Targeted`]. Lookup
//! compares name bytes exactly and returns the first bucket match, so the
//! last definition wins. An absent name binds the null object, whose
//! accumulator is empty.
//!
//! Inside a definition, [`modifiers`] handles remaining keys: the modifier
//! is found by the key's token without case (the lexer folds a bare key
//! and looks up a quoted one), its destination is zeroed, then fixed5
//! conversion reads the value. A value it cannot convert stores 0; a
//! repeated key replaces the earlier value. `hidden_modifier` entries are
//! stored in the same accumulator. Descriptions read the definition's own
//! accumulator, so an `added_modifier` child is not read, and an absent key
//! is zero.
//!
//! Refused by file, modifier and key, never read as zero or replaced with the
//! shipped value:
//! - a file whose reading the port cannot follow ([`modifiers::Undecided`]: a
//!   block under a modifier name, which the content alone cannot say the game
//!   reads as a value or skips; or [`native::Read::unsettled`]): the rest of
//!   that file could define any name, so a request stays refused unless a
//!   later file defines its modifier;
//! - a definition with a `key` or `tag` field: the field renames the static
//!   modifier after it was bucketed under its first name, so which lookup
//!   finds it depends on the database's bucket layout, which the port does
//!   not model. The requests for either name are refused;
//! - a requested definition holding text the port cannot keep
//!   ([`modifiers::Modifiers::unheld`]).

use crate::{
    ResolvedLayers,
    key_names::{KeyNames, Point},
    modifiers::{self, Undecided},
    parser::{
        self, Item,
        native::{self, Shape},
    },
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, OnceLock},
};

type DescriptionValue = Result<(Option<String>, modifiers::Modifiers), String>;
type DescriptionFiles = Result<(KeyNames, Vec<(String, File)>), String>;

/// Lazy description lookups share the native file walk and bucket precedence.
/// Each stack is read once, and each requested name is resolved once.
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

    /// Returns the last exact-name definition, or the empty null object.
    pub fn get(&self, layers: &ResolvedLayers, name: &str) -> DescriptionValue {
        if let Some(value) = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(name)
        {
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
                    // Static load precedes sub-unit modifier registration.
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

// An unknown token never inserts an accumulator value.
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

/// The directory used for static-modifier loading.
pub const DIRECTORY: &str = "common/modifiers";

/// One requested modifier value of one named static modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub modifier: &'a str,
    pub key: &'a str,
}

/// A requested value the reader cannot convert exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The content path of the effective definition, or of the file whose
    /// reading stops.
    pub file: String,
    pub modifier: String,
    pub key: String,
    pub reason: String,
}

/// A requested value: raw fixed5 (zero when the modifier or key is absent),
/// or the named refusal.
pub type Resolved = Result<i64, Refusal>;

/// What [`read`] gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    /// Each request's value, in request order.
    pub values: Vec<Resolved>,
    /// Each place the game's reading leaves the text ([`native::Read::misreads`]),
    /// as `path: line N: what`; the values follow the game's reading.
    pub notes: Vec<String>,
}

/// Full accumulators of named static modifiers, in request order. An absent
/// name is the null object's empty accumulator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocks {
    pub blocks: Vec<Result<modifiers::Modifiers, Refusal>>,
    pub notes: Vec<String>,
}

/// Script identifiers whose localisations initialise static-modifier names.
/// An absent modifier has the null object's empty display name. Renamed
/// or unresolvable definitions retain named refusals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identifiers {
    pub identifiers: Vec<Result<Option<String>, Refusal>>,
    pub notes: Vec<String>,
}

pub fn read_identifiers(layers: &ResolvedLayers, names: &[&str]) -> io::Result<Identifiers> {
    let files = layer_files(layers)?;
    let key_names = layers.key_names()?.at(Point::StaticModifiers);
    let (identifiers, notes) = resolve_with(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        names,
        Some(&key_names),
        None,
        |name, items| {
            // Preserve the same unsupported-content contract as read_blocks.
            stored_block(items)?;
            Ok(Some(name.to_owned()))
        },
    );
    Ok(Identifiers { identifiers, notes })
}

/// Reads each request, in request order, from the effective definitions of
/// the content stack.
pub fn read(layers: &ResolvedLayers, requests: &[Request<'_>]) -> io::Result<Values> {
    let files = layer_files(layers)?;
    let key_names = layers.key_names()?.at(Point::StaticModifiers);
    Ok(resolve(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        requests,
        Some(&key_names),
    ))
}

/// Reads full static-modifier accumulators for description formatting.
/// Definitions and stopped files follow the same precedence and refusal
/// rules as [`read`].
pub fn read_blocks(layers: &ResolvedLayers, names: &[&str]) -> io::Result<Blocks> {
    let files = layer_files(layers)?;
    let key_names = layers.key_names()?.at(Point::StaticModifiers);
    Ok(resolve_blocks(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        names,
        Some(&key_names),
    ))
}

fn layer_files(layers: &ResolvedLayers) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for path in layers.directory_files(DIRECTORY, ".txt")? {
        let bytes = path.read_uncovered(layers)?;
        files.push((path.to_string(), bytes));
    }
    Ok(files)
}

/// One file: its definitions up to the stop, and the stop.
struct File {
    definitions: Vec<Item>,
    stop: Option<String>,
    notes: Vec<String>,
}

#[derive(Clone, Copy)]
enum Node {
    /// The file loop.
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
    // Each definition's position among those read, when it began.
    let mut entries = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        // valid_relation_trigger reads a conjunction body; trigger dispatch
        // needs the rosters already loaded at static load.
        key_names,
        &mut |node, field| {
            let shape = match node {
                // Every key is a static modifier read by the persistent reader,
                // whatever its value.
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

/// [`read`] over files already in native order.
fn resolve<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    requests: &[Request<'_>],
    key_names: Option<&KeyNames>,
) -> Values {
    // One block lookup establishes the file and bucket precedence;
    // scalar callers select their key from that block.
    let names: Vec<_> = requests.iter().map(|request| request.modifier).collect();
    let Blocks { blocks, notes } = resolve_blocks(files, &names, key_names);
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

/// Each definition is inserted at the front of its hash bucket; lookup
/// finds the first exact name. A later file can replace a stopped file's
/// unknown definition, while a stop shadows every earlier candidate.
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
