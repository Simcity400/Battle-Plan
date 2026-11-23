use viewer_layers_forces::orders::Plans;
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_counter_products::map::orders;
use viewer_runtime::memo::project;
use viewer_gpu::ribbon_content::OrderField;

impl Plans for crate::document::Content {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        engine_executable::executable_tables::scope(self.tables, || {
            let at = replay_frame::cpu::scope("plan.links", Some("plan"));
            let links = viewer_layer_core::scene_content::links_of(frame)?;
            drop(at);
            let at = replay_frame::cpu::scope("plan.requires", Some("plan"));
            let whole = orders::requires_full_frame(frame, &links, ours, show_allied, self.icons.fonts.arrow().is_some())?;
            drop(at);
            if whole {
                self.battle_plan(frame, &links, ours, show_allied)
            } else {
                let at = replay_frame::cpu::scope("plan.project", Some("plan"));
                let only = project(frame, &orders::memo_tables(show_allied));
                drop(at);
                self.battle_plan(&only, &links, ours, show_allied)
            }
        })
    }
}
