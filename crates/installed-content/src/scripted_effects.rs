//! Scripted-effect names at their own load point, before any effect body.

use crate::{
    ResolvedLayers,
    parser::{
        c_string,
        native::{self, Shape},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default)]
pub struct ScriptedEffects {
    pub bodies: BTreeMap<Vec<u8>, Result<Vec<crate::parser::Item>, String>>,
}

impl ScriptedEffects {
    /// Body pass: only names registered by the name pass bind templates.
    pub fn read_file(&mut self, bytes: &[u8], names: &crate::key_names::KeyNames) {
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(()),
            Some(names),
            &mut |(), field| {
                if names
                    .scripted_effects
                    .contains(&c_string(&field.key.text).to_ascii_lowercase())
                {
                    Shape::Effect
                } else {
                    Shape::One // A mismatch skips no body.
                }
            },
        );
        for (index, item) in read.items.into_iter().enumerate() {
            let name = c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase();
            if !names.scripted_effects.contains(&name) {
                continue;
            }
            let notes = read.item_misreads.get(index).cloned().unwrap_or_default();
            let body = if notes.is_empty() {
                Ok(item.value.items().unwrap_or_default().to_vec())
            } else {
                Err(notes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "))
            };
            let existing = self.bodies.entry(name).or_insert_with(|| Ok(Vec::new()));
            match (&mut *existing, body) {
                (Ok(existing), Ok(body)) => existing.extend(body), // Effect bodies append.
                (_, Err(why)) => *existing = Err(why),
                (Err(_), Ok(_)) => {}
            }
        }
        if let Some(why) = read.unsettled {
            // An unread suffix can append to any registered template.
            for name in &names.scripted_effects {
                self.bodies.insert(name.clone(), Err(why.to_string()));
            }
        }
    }
}

pub fn read(
    layers: &ResolvedLayers,
    names: &crate::key_names::KeyNames,
) -> io::Result<ScriptedEffects> {
    let mut result = ScriptedEffects {
        bodies: names
            .scripted_effects
            .iter()
            .map(|name| (name.clone(), Ok(Vec::new())))
            .collect(),
    };
    let paths = layers.listed_paths("common/scripted_effects/");
    for path in crate::defines::load_order(
        "common/scripted_effects/",
        ".txt",
        paths.iter().map(String::as_str),
    ) {
        result.read_file(&layers.read(&path)?, names);
    }
    Ok(result)
}

/// Name pass: the directory's own `.txt` files, values skipped without
/// defining variables. Effect lookup finds the name by token hash and
/// requires a template registered by this pass.
pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut names = BTreeSet::new();
    for path in crate::script_files::paths(layers, "common/scripted_effects/", Some(".txt")) {
        names.extend(file_names(&layers.read(&path)?));
    }
    Ok(names)
}

/// The name pass over one file, using the same reader as the body readers.
pub fn file_names(bytes: &[u8]) -> BTreeSet<Vec<u8>> {
    native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(()),
        None,
        &mut |(), _| Shape::SkipIfOpen,
    )
    .items
    .iter()
    .map(|item| c_string(item.key.as_deref().unwrap_or_default()).to_ascii_lowercase())
    .collect()
}

#[cfg(test)]
mod tests;
