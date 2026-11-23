use std::io;
use content_reader_layers::ResolvedLayers;
use crate::key_name_products::KeyNamesExt;
use content_reader_script::scripted_trigger_grammar::{DIRECTORY, ScriptedTriggers};

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
    triggers.read_all_names(layers)?;
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
