use viewer_layers_political::map_text::Source;
use content_reader_localisation::localisation::Localisation;
use viewer_foundation::visibility::FogContent;
use viewer_map_text::animated_map_text::events;

impl Source for crate::document::Content {
    fn text(&self) -> &events::Content {
        &self.products.map_text
    }

    fn localisation(&self) -> &Localisation {
        &self.products.localisation
    }

    fn place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        self.products.impact_place(province)
    }

    fn fog(&self) -> &Result<FogContent, String> {
        &self.products.fog
    }
}
