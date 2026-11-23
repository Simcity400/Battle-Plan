use viewer_layers_infrastructure::rails::Geography;
use crate::document::Content;
use std::io;
use viewer_rail::railways::{Bridge, RailNetwork, RailRun};

impl Geography for Content {
    fn cutoff(&self) -> f32 {
        self.map_defines.railway_cutoff
    }

    fn run(&self, network: &RailNetwork, province: u32) -> io::Result<Option<RailRun>> {
        self.rail_geometry.run(network, province)
    }

    fn bridge(&self, network: &RailNetwork, key: (u32, u32)) -> Option<Bridge> {
        self.rail_geometry.bridge(network, key)
    }
}
