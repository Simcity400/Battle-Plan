pub fn political_lines(side: &viewer_layer_core::playhead_host::MapSide<impl viewer_layer_core::playhead_host::PlaybackProducts>) -> Vec<String> {
    match side.parts().filter(|_| side.draws("political")) {
        Some((_, layers)) => {
            let political = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::political::Political>()).flat_map(|layer| layer.warnings());
            let naval = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::naval::Naval>()).flat_map(|layer| layer.warnings());
            let modes = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::mode_fills::ModeFills>()).flat_map(crate::mode_fills::lines);
            political.chain(naval).chain(modes).collect()
        }
        None => Vec::new(),
    }
}
