use viewer_layers_political::occupation_marks::Source;
use crate::document::Content;
use viewer_map_icons::occupation_icons as icons;

impl Source for Content {
    fn occupation(&self) -> Option<&icons::Content> {
        self.icons.occupation.as_ref().ok()
    }
}
