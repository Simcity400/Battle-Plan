//! State -> RGBA color table construction.
//!
//! This is the only buffer scrubbing rewrites (MAP_REPLICA_PLAN Phase 2
//! acceptance gate): one `R8G8B8A8_UNORM` element per state id. Grounding
//! rule: a state with a recorded owner whose color is in
//! `country_colors.json` gets that color at full opacity; everything else
//! gets the no-owner sentinel (alpha 0), which makes the pixel shader show
//! the grounded `map_visual.rgbx` underlay instead.

use std::collections::BTreeMap;

/// Alpha 0 = "no recorded owner color": the shader shows the underlay.
pub const NO_OWNER_SENTINEL_RGBA: u32 = 0;

/// Encode an owner RGB as a little-endian `R8G8B8A8_UNORM` texel at full
/// opacity (bytes r, g, b, 255 in memory).
pub fn encode_owner_rgba(color: [u8; 3]) -> u32 {
    u32::from(color[0]) | (u32::from(color[1]) << 8) | (u32::from(color[2]) << 16) | 0xFF00_0000
}

/// Build the compact state -> RGBA table of length `max_state_id + 1`.
/// Index = state id. States without a recorded owner, and states whose
/// owner tag has no entry in `country_colors.json`, get the sentinel.
/// Impassable (wasteland) states are NOT special-cased: the engine's political
/// mapmode ignores the `impassable` flag on the country-colour plane, so an
/// owned impassable state gets the same owner colour as passable land (verified
/// byte-exact against the captured GradientBorderChannel1: 0/22 impassable
/// states are gray).
pub fn build_state_color_table(
    max_state_id: u32,
    owners: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> Vec<u32> {
    let len = max_state_id as usize + 1;
    let mut table = vec![NO_OWNER_SENTINEL_RGBA; len];
    for (state_id, tag) in owners {
        let index = *state_id as usize;
        if index >= len {
            continue;
        }
        if let Some(color) = country_colors.get(tag) {
            table[index] = encode_owner_rgba(*color);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_colors() -> BTreeMap<String, [u8; 3]> {
        let mut colors = BTreeMap::new();
        colors.insert("FRA".to_string(), [10, 20, 30]);
        colors.insert("GER".to_string(), [40, 50, 60]);
        colors
    }

    #[test]
    fn owner_color_is_full_opacity_rgba() {
        let mut owners = BTreeMap::new();
        owners.insert(2u32, "FRA".to_string());
        let table = build_state_color_table(3, &owners, &fixture_colors());
        assert_eq!(table.len(), 4);
        // bytes in memory: r=10, g=20, b=30, a=255
        assert_eq!(table[2], 10 | (20 << 8) | (30 << 16) | 0xFF00_0000);
    }

    #[test]
    fn states_without_owner_get_sentinel() {
        let mut owners = BTreeMap::new();
        owners.insert(1u32, "GER".to_string());
        let table = build_state_color_table(3, &owners, &fixture_colors());
        assert_eq!(table[0], NO_OWNER_SENTINEL_RGBA);
        assert_eq!(table[2], NO_OWNER_SENTINEL_RGBA);
        assert_eq!(table[3], NO_OWNER_SENTINEL_RGBA);
        assert_ne!(table[1], NO_OWNER_SENTINEL_RGBA);
    }

    #[test]
    fn owner_without_country_color_gets_sentinel() {
        let mut owners = BTreeMap::new();
        owners.insert(1u32, "ZZZ".to_string());
        let table = build_state_color_table(1, &owners, &fixture_colors());
        assert_eq!(table[1], NO_OWNER_SENTINEL_RGBA);
    }

    #[test]
    fn out_of_range_state_ids_are_ignored() {
        let mut owners = BTreeMap::new();
        owners.insert(9u32, "FRA".to_string());
        let table = build_state_color_table(3, &owners, &fixture_colors());
        assert_eq!(table.len(), 4);
        assert!(table.iter().all(|&c| c == NO_OWNER_SENTINEL_RGBA));
    }

    #[test]
    fn sentinel_alpha_is_zero_and_owner_alpha_is_full() {
        assert_eq!(NO_OWNER_SENTINEL_RGBA >> 24, 0);
        assert_eq!(encode_owner_rgba([255, 255, 255]) >> 24, 0xFF);
    }
}
