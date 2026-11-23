use viewer_layers_counters::naval_markers::Shore;
use content_reader_map::map::ProvinceKind;

impl Shore for crate::document::Content {
    fn province_kind(&self, province: u32) -> Option<ProvinceKind> {
        self.counters.province_kind(province)
    }

    fn anchor(&self, province: u32, slot: u32) -> Option<[f32; 3]> {
        self.counters.anchor(province, slot)
    }
}
