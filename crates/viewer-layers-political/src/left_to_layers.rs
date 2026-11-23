#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftToLayers {
    pub resource_icons: bool,
    pub occupation_marks: bool,
    pub radar: bool,
}

impl LeftToLayers {
    pub fn of(side: Option<viewer_layer_core::playhead_host::LayerParts<'_>>) -> Self {
        Self {
            resource_icons: side.and_then(crate::resource_icons::drawn).is_some(),
            occupation_marks: side.and_then(crate::occupation_marks::drawn).is_some(),
            radar: side.is_some_and(|side| side.draws("radar")),
        }
    }

}
