use viewer_layers_political::names::Atlas;
use crate::document::Content;
use engine_executable::executable_tables::Tables as Executable;
use replay_frame::frame::DecodedFrame;
use content_reader_localisation::localisation::Localisation;
use map_display_data::landscape::{Placements, WorldName};
use std::{collections::BTreeSet, io, sync::Arc};
use viewer_map_modes::map_mode::{MapMode, NameLayers};
use viewer_map_text::name_pass::{self, PlacementInputs};

impl Atlas for Content {
    fn provinces(&self) -> Vec<u32> {
        self.map.definitions.keys().copied().collect()
    }

    fn localisation(&self) -> Arc<Localisation> {
        Arc::clone(&self.names.localisation)
    }

    fn components(&self, members: &BTreeSet<u32>) -> Vec<Vec<u32>> {
        self.names.graph.components(members)
    }

    fn place(&self, tag: &str, text: &str, component: &[u32]) -> Option<WorldName> {
        let inputs = PlacementInputs { map: &self.map, heightmap: &self.assets.heightmap, content: &self.names, font: &self.icons.fonts.map };
        name_pass::place_component(&inputs, tag, text, component, false)
    }

    fn name_layers(&self, mode: &MapMode) -> Option<NameLayers> {
        self.map_modes.knows(mode).then(|| self.map_modes.name_layers(mode))
    }

    fn names_cutoff(&self) -> f32 {
        self.map_defines.country_names_cutoff
    }

    fn mode_names(&self, frame: &DecodedFrame, mode: &MapMode) -> io::Result<Arc<Placements>> {
        crate::document::Hour::new(self, frame, 0.0).shared_names(mode)
    }

    fn executable(&self) -> Option<Executable> {
        self.tables
    }
}
