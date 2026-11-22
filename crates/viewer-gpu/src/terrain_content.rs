use std::collections::BTreeMap;

pub type TerrainWidths = [i64; 2];

#[derive(Default)]
pub struct TerrainContent {
    pub combat_terrain: BTreeMap<u32, String>,
    pub terrain_widths: Vec<(Vec<u8>, TerrainWidths)>,
}

impl TerrainContent {
    pub fn combat_widths(&self, terrain: &str) -> Option<&TerrainWidths> {
        self.terrain_widths
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(terrain.as_bytes()))
            .map(|(_, widths)| widths)
    }
}
