use viewer_layers_forces::units::Ground;
use std::collections::BTreeMap;
use viewer_gpu::{camera::{MapDefines, MapSize}, province_map::HeightField};
use viewer_unit_reads::counter_move::RouteWorld;

impl Ground for crate::document::Content {
    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]> {
        self.counters.anchor(province, slot)
    }

    fn world(&self) -> Option<RouteWorld<'_>> {
        let heights = HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        self.movement.as_deref().map(|movement| RouteWorld { map: &movement.map, heights, width: self.map.width as f32 })
    }

    fn transport_scale(&self) -> f32 {
        self.unit_models.gates.transport_scale
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        &self.unit_models.bounds
    }

    fn units_cutoff(&self) -> f32 {
        self.unit_models.gates.units
    }

    fn ships_cutoff(&self) -> f32 {
        self.unit_models.gates.ships
    }

    fn unit_models(&self) -> Option<&viewer_unit_models::unit_models::UnitModelAssets> {
        Some(&self.unit_models)
    }

    fn map(&self) -> (MapSize, MapDefines) {
        (MapSize::of(&self.map), self.map_defines)
    }
}
