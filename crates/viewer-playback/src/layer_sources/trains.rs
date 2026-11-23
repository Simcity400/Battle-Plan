use viewer_layers_infrastructure::trains::{PlanRows, Yard};
use crate::document::Content;
use replay_data::rows::TableId;
use std::io;
use viewer_gpu::province_map::HeightField;
use viewer_rail::{railways::RailNetwork, trains::{Curve, PlanCache, TrainPlan, route_curve}};

impl Yard for Content {
    fn cutoff(&self) -> f32 {
        self.map_defines.railway_cutoff
    }

    fn plan(&self, rows: &PlanRows<'_>, ours: u32, curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>, cache: &mut PlanCache) -> io::Result<TrainPlan> {
        let head = rows.head(TableId::Countries).unwrap_or(0);
        TrainPlan::build_from(rows, 0, head, ours, &self.trains, &self.trains.cultures, curve, Some(cache))
    }

    fn curve(&self, path: &[u32], network: &RailNetwork) -> Curve {
        let heights = HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        route_curve(&self.rail_geometry, path, network, &heights)
    }
}
