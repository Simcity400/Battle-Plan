use viewer_layers_political::mode_fills::{Fill, Paint};
use engine_executable::executable_tables::Tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_foundation::perspective::Choice;
use viewer_borders::{border_pass::ClassPlan, demilitarized::ZonePlan};
use viewer_map_modes::map_mode::MapMode;

impl Paint for crate::document::Content {
    fn tables(&self) -> Option<Tables> {
        self.tables
    }

    fn paint(&self, frame: &DecodedFrame, choice: &Choice, mode: &MapMode, selected: Option<u32>) -> io::Result<Fill> {
        let product = crate::document::Hour::new(self, frame, 0.0).political_product(mode, choice, selected, None)?;
        let regions = product.layers.regions;
        let classes = ClassPlan::new(&self.border_meshes, &regions, &product.zone);
        let highlighted = ZonePlan::new(&self.border_meshes, &regions, &product.zone);
        Ok(Fill { mode: mode.clone(), regions, zone: product.zone, classes, highlighted })
    }
}
