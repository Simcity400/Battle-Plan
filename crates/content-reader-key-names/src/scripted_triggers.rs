use crate::key_name_products::KeyNamesExt;
use content_reader_layers::ResolvedLayers;
use content_reader_script::scripted_trigger_grammar::ScriptedTriggers;
use std::{collections::BTreeSet, io};

const DIRECTORY: &str = "common/scripted_triggers/";

pub fn read(layers: &ResolvedLayers) -> io::Result<ScriptedTriggers> {
    read_with_bindings(layers, false)
}

pub fn read_descriptions(layers: &ResolvedLayers) -> io::Result<ScriptedTriggers> {
    read_with_bindings(layers, true)
}

fn read_with_bindings(
    layers: &ResolvedLayers,
    tooltip_bindings: bool,
) -> io::Result<ScriptedTriggers> {
    let names = layers.key_names()?;
    let mut triggers = ScriptedTriggers::default();
    let listed = layers.listed_layer_paths(DIRECTORY.as_bytes())?;
    read_all_names(&mut triggers, layers)?;
    for path in content_reader_defines::defines::load_order_paths(
        DIRECTORY.as_bytes(),
        b".txt",
        listed.iter().map(Vec::as_slice),
    ) {
        let path = content_reader_layers::script_files::ScriptPath::new(path);
        triggers.read_bodies(&path.read(layers)?, &path.text(), &names, tooltip_bindings);
    }
    Ok(triggers)
}

fn read_all_names(triggers: &mut ScriptedTriggers, layers: &ResolvedLayers) -> io::Result<()> {
    for path in content_reader_layers::script_files::paths(layers, DIRECTORY, Some(".txt"))? {
        triggers.read_names(&path.read(layers)?, &path.text());
    }
    Ok(())
}

pub fn names(layers: &ResolvedLayers) -> io::Result<BTreeSet<Vec<u8>>> {
    let mut triggers = ScriptedTriggers::default();
    read_all_names(&mut triggers, layers)?;
    Ok(triggers.templates.into_keys().collect())
}
