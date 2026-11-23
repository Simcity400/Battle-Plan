use viewer_layers_counters::{land_markers::Front, naval_counters::Admits};
use content_reader_core::movement::MovementMap;
use viewer_counter_kit::combat_markers::LandCombatDefines;
use viewer_gpu::camera::Placement;

impl Front for crate::document::Content {
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]> {
        self.counters.anchor(province, slot)
    }

    fn movement(&self) -> Option<&MovementMap> {
        self.movement.as_deref().map(|movement| &movement.map)
    }

    fn defines(&self) -> LandCombatDefines {
        self.counters.defines.land_combat
    }

    fn cutoff(&self) -> f32 {
        self.counters.defines.land_combat_cutoff
    }

    fn admits(&self, placement: Placement, viewport: [u32; 2]) -> Option<Admits> {
        viewer_layers_counters::naval_counters::Waters::admits(self, placement, viewport)
    }
}
