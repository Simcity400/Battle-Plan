use viewer_layers_counters::naval_counters::{Waters, Admits};
use std::sync::Arc;
use viewer_gpu::camera::Placement;
use viewer_unit_reads::{counter_move::UnitRoute, task_force_visual};

impl Waters for crate::document::Content {
    fn land(&self, province: u32) -> bool {
        self.ground().land(province)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.counters.region(province)
    }

    fn place(&self, province: u32, route: &UnitRoute<'_>) -> Result<task_force_visual::Placement, &'static str> {
        self.ground().place(province, route)
    }

    fn port(&self, province: u32) -> Option<[f32; 3]> {
        self.counters.port(province)
    }

    fn icons_cutoff(&self) -> f32 {
        self.counters.defines.icons_cutoff
    }

    fn admits(&self, placement: Placement, viewport: [u32; 2]) -> Option<Admits> {
        if viewport == [0, 0] {
            return None;
        }
        let map = viewer_gpu::camera::MapSize::of(&self.map);
        let admission = viewer_gpu::province_admission::Admission::new(placement.on(map, self.map_defines), viewport);
        let counters = Arc::clone(&self.counters);
        Some(Arc::new(move |province| admission.admits_province(&counters.bounds, province)))
    }
}

impl crate::document::Content {
    pub fn ground(&self) -> viewer_counters_naval::naval_members::SeaGround<'_> {
        let heights = viewer_gpu::province_map::HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        viewer_counters_naval::naval_members::SeaGround {
            content: &self.counters,
            map: &self.map,
            movement: self.movement.as_deref(),
            world: self.movement.as_deref().map(|movement| viewer_unit_reads::counter_move::RouteWorld { map: &movement.map, heights, width: viewer_gpu::camera::MapSize::of(&self.map).x() }),
        }
    }
}
