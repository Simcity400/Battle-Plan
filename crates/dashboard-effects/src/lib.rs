#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

#[cfg(test)]
use dashboard_text::test_country_links;
use dashboard_text::{
    catalogue, character_scope, idea_names, identity, modifier_text, scope, scripted_text,
    triggers, unit_names, variant_names,
};

pub mod effect_description;
pub use effect_description::{ScriptedEffects, effect_describer};

#[cfg(test)]
mod balance_tests;
#[cfg(test)]
mod fixtures {
    pub use dashboard_text::catalogue::tests::Folder;
    use dashboard_text::catalogue::{Catalogue, tests};

    pub mod game;

    pub fn folder_catalogue(files: &[(&str, &str)]) -> (Catalogue, Folder) {
        let (catalogue, folder) = tests::folder_catalogue(files);
        (
            tests::with_effect_describer(catalogue, super::effect_describer),
            folder,
        )
    }

    pub fn layers_catalogue(layers: content_reader_layers::ResolvedLayers) -> Catalogue {
        tests::with_effect_describer(tests::layers_catalogue(layers), super::effect_describer)
    }
}
