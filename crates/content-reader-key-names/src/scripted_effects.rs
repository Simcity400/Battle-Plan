
use content_reader_layers::ResolvedLayers;
use content_reader_parser::parser::c_string;
use content_reader_script::native::{self, Shape};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Clone, Debug, Default)]
pub struct ScriptedEffects {
    pub bodies: BTreeMap<Vec<u8>, Result<Vec<content_reader_parser::parser::Item>, String>>,
}

impl ScriptedEffects {
    pub fn read_file(&mut self, bytes: &[u8], names: &content_reader_script::key_names::KeyNames) {
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
                    Shape::One
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
                (Ok(existing), Ok(body)) => existing.extend(body),
                (_, Err(why)) => *existing = Err(why),
                (Err(_), Ok(_)) => {}
            }
        }
        if let Some(why) = read.unsettled {
            for name in &names.scripted_effects {
                self.bodies.insert(name.clone(), Err(why.to_string()));
            }
        }
    }
}

pub fn read(
    layers: &ResolvedLayers,
    names: &content_reader_script::key_names::KeyNames,
) -> io::Result<ScriptedEffects> {
    let mut result = ScriptedEffects {
        bodies: names
            .scripted_effects
            .iter()
            .map(|name| (name.clone(), Ok(Vec::new())))
            .collect(),
    };
    let paths = layers.listed_layer_paths(b"common/scripted_effects/")?;
    for path in content_reader_defines::defines::load_order_paths(
        b"common/scripted_effects/",
        b".txt",
        paths.iter().map(Vec::as_slice),
    ) {
        result.read_file(&layers.read_path(&path)?, names);
    }
    Ok(result)
}

pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut names = BTreeSet::new();
    for path in content_reader_layers::script_files::paths(layers, "common/scripted_effects/", Some(".txt"))? {
        names.extend(file_names(&path.read(layers)?));
    }
    Ok(names)
}

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
