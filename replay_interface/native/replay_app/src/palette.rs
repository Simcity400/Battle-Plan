
use std::collections::BTreeMap;

use crate::Result;

pub const NO_OWNER_SENTINEL_RGBA: u32 = 0;

pub fn encode_owner_rgba(color: [u8; 3]) -> u32 {
    u32::from(color[0]) | (u32::from(color[1]) << 8) | (u32::from(color[2]) << 16) | 0xFF00_0000
}

pub fn build_state_color_table(
    max_state_id: u32,
    owners: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> Result<Vec<u32>> {
    let len = max_state_id as usize + 1;
    let mut table = vec![NO_OWNER_SENTINEL_RGBA; len];
    for (state_id, tag) in owners {
        let index = *state_id as usize;
        if index >= len {
            continue;
        }
        let color = country_colors.get(tag).ok_or_else(|| {
            format!("state {state_id} owner/controller tag {tag} has no resolved country color")
        })?;
        table[index] = encode_owner_rgba(*color);
    }
    Ok(table)
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
        let table = build_state_color_table(3, &owners, &fixture_colors()).expect("colors");
        assert_eq!(table.len(), 4);
        assert_eq!(table[2], 10 | (20 << 8) | (30 << 16) | 0xFF00_0000);
    }

    #[test]
    fn states_without_owner_get_sentinel() {
        let mut owners = BTreeMap::new();
        owners.insert(1u32, "GER".to_string());
        let table = build_state_color_table(3, &owners, &fixture_colors()).expect("colors");
        assert_eq!(table[0], NO_OWNER_SENTINEL_RGBA);
        assert_eq!(table[2], NO_OWNER_SENTINEL_RGBA);
        assert_eq!(table[3], NO_OWNER_SENTINEL_RGBA);
        assert_ne!(table[1], NO_OWNER_SENTINEL_RGBA);
    }

    #[test]
    fn owner_without_country_color_fails_closed() {
        let mut owners = BTreeMap::new();
        owners.insert(1u32, "ZZZ".to_string());
        let err =
            build_state_color_table(1, &owners, &fixture_colors()).expect_err("missing color");
        let message = err.to_string();
        assert!(
            message.contains("state 1 owner/controller tag ZZZ has no resolved country color"),
            "{message}"
        );
    }

    #[test]
    fn out_of_range_state_ids_are_ignored() {
        let mut owners = BTreeMap::new();
        owners.insert(9u32, "FRA".to_string());
        let table = build_state_color_table(3, &owners, &fixture_colors()).expect("colors");
        assert_eq!(table.len(), 4);
        assert!(table.iter().all(|&c| c == NO_OWNER_SENTINEL_RGBA));
    }

    #[test]
    fn sentinel_alpha_is_zero_and_owner_alpha_is_full() {
        assert_eq!(NO_OWNER_SENTINEL_RGBA >> 24, 0);
        assert_eq!(encode_owner_rgba([255, 255, 255]) >> 24, 0xFF);
    }
}
