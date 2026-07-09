//! Replay-app bridge for map furniture (VP/capital city names and icons).
//!
//! The furniture law lives in `map_viewer::map_furniture`; this module gives
//! replay_app a product-side module boundary without re-deriving placement,
//! tiering, relation color, labels, or capital selection. The CLI remains
//! fail-closed until the remaining draw-time residuals are captured/decompiled.
#![allow(dead_code)]

use map_viewer::{
    compute_furniture_markers, furniture_layer_from_resolver, FurnitureLawInputs, FurnitureLayer,
    FurnitureMarker, GameFileResolver, VpIconDefines,
};

use crate::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayFurnitureLayer {
    pub width: usize,
    pub height: usize,
    pub markers: Vec<FurnitureMarker>,
    pub vp_icon_defines: Option<VpIconDefines>,
}

impl ReplayFurnitureLayer {
    pub fn from_law_inputs(width: usize, height: usize, inputs: &FurnitureLawInputs<'_>) -> Self {
        Self {
            width,
            height,
            markers: compute_furniture_markers(inputs),
            vp_icon_defines: inputs.vp_icon_defines.cloned(),
        }
    }

    pub fn from_map_viewer_layer(layer: FurnitureLayer) -> Self {
        Self {
            width: layer.width,
            height: layer.height,
            markers: layer.markers,
            vp_icon_defines: Some(layer.vp_icon_defines),
        }
    }
}

pub fn load_replay_furniture_layer(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<ReplayFurnitureLayer> {
    let layer = furniture_layer_from_resolver(resolver, language)?;
    Ok(ReplayFurnitureLayer::from_map_viewer_layer(layer))
}

pub fn replay_vp_icon_relation_column(
    owner_tag: Option<&str>,
    viewed_tag: Option<&str>,
    pair_state: map_viewer::VpIconDiplomaticPairState,
) -> u32 {
    map_viewer::vp_icon_relation_column(owner_tag, viewed_tag, pair_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_viewer::FurnitureMarkerKind;
    use std::collections::BTreeMap;

    #[test]
    fn replay_furniture_module_reproduces_map_viewer_law_fixture() {
        let province_ids = vec![100, 100, 101, 101, 100, 100, 101, 101];
        let province_to_state = BTreeMap::from([(100, 5), (101, 5)]);
        let state_owners = BTreeMap::from([(5, "FRA".to_string())]);
        let state_victory_points = BTreeMap::from([(5, BTreeMap::from([(100, 10), (101, 3)]))]);
        let capital_states = BTreeMap::from([("FRA".to_string(), 5)]);
        let localisation = BTreeMap::from([
            ("VICTORY_POINTS_100".to_string(), "Alpha".to_string()),
            ("VICTORY_POINTS_101".to_string(), "Beta".to_string()),
        ]);
        let inputs = FurnitureLawInputs {
            province_ids: &province_ids,
            width: 4,
            height: 2,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            state_victory_points: &state_victory_points,
            capital_states: &capital_states,
            localisation: &localisation,
            vp_icon_defines: None,
        };

        let replay = ReplayFurnitureLayer::from_law_inputs(4, 2, &inputs);
        let map_viewer = compute_furniture_markers(&inputs);
        assert_eq!(replay.markers, map_viewer);
        assert_eq!(replay.markers.len(), 2);
        assert_eq!(replay.width, 4);
        assert_eq!(replay.height, 2);
        assert_eq!(replay.markers[0].province_id, 100);
        assert_eq!(replay.markers[0].kind, FurnitureMarkerKind::Capital);
        assert_eq!(replay.markers[1].kind, FurnitureMarkerKind::VictoryPoint);
        assert_eq!(
            replay_vp_icon_relation_column(
                Some("SOV"),
                Some("GER"),
                map_viewer::VpIconDiplomaticPairState {
                    same_faction: false,
                    at_war: true,
                },
            ),
            map_viewer::VP_STRIP_COLOR_ENEMY_RED
        );
    }
}
