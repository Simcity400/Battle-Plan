use crate::document::{Content, Hour};
use viewer_layers_forces::aircraft::Source;
use viewer_layer_core::scene_content::SceneContent;
use replay_frame::frame::DecodedFrame;

impl Source for Content {
    fn scene(&self) -> &SceneContent {
        self
    }

    fn tables(&self) -> Option<engine_executable::executable_tables::Tables> {
        self.tables
    }

    fn hour<'a>(&'a self, frame: &'a DecodedFrame) -> map_products::ProductHour<'a> {
        Hour::new(self, frame, 0.0).product_hour
    }
}
