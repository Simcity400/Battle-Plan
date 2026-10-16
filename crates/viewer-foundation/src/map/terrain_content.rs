//! Authored province terrain and native combat widths shared by counters and combat.
use std::collections::BTreeMap;

/// A terrain category's `combat_width` and `combat_support_width` in native
/// fixed5 (a value that does not convert keeps the earlier one, and the
/// terrain reader reports it).
pub type TerrainWidths = [i64; 2];

#[derive(Default)]
pub struct TerrainContent {
    /// Authored province terrain, column seven of map/definition.csv.
    pub combat_terrain: BTreeMap<u32, String>,
    /// Each terrain category's name with its `combat_width` and
    /// `combat_support_width`, in load order ([`Self::combat_widths`]).
    pub terrain_widths: Vec<(Vec<u8>, TerrainWidths)>,
}

impl TerrainContent {
    /// A terrain's combat widths: the first category whose name matches without
    /// regard to ASCII case, as `terrain-name lookup` resolves it.
    pub fn combat_widths(&self, terrain: &str) -> Option<&TerrainWidths> {
        self.terrain_widths
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(terrain.as_bytes()))
            .map(|(_, widths)| widths)
    }
}
