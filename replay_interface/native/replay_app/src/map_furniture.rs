//! Replay-app bridge for map furniture (VP/capital city names and icons).
//!
//! The furniture law lives in `map_viewer::map_furniture`; this module gives
//! replay_app a product-side module boundary without re-deriving placement,
//! tiering, relation color, labels, or capital selection. The CLI remains
//! fail-closed until the remaining draw-time residuals are captured/decompiled.
#![allow(dead_code)]

use std::collections::BTreeMap;

use map_viewer::{
    compute_furniture_markers, furniture_layer_from_resolver, FurnitureLawInputs, FurnitureLayer,
    FurnitureMarker, GameFileResolver, VpIconDefines,
};
use replay_core::{CountryDiplomacy, Leaf, WorldFrame};

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

    /// Build replay-hour furniture from the engine-current capital carrier.
    ///
    /// Exact a729 consumers resolve `CCountry+0x1018` for map focus, display
    /// coordinates, and capital-relative ordering. The history-file capital
    /// supplied in `inputs` is therefore intentionally replaced, never used
    /// as a fallback. A missing current carrier omits the capital marker.
    pub fn from_world_frame(frame: &WorldFrame, inputs: &FurnitureLawInputs<'_>) -> Self {
        let capital_states = replay_current_capital_states(frame);
        let replay_inputs = FurnitureLawInputs {
            province_ids: inputs.province_ids,
            width: inputs.width,
            height: inputs.height,
            province_to_state: inputs.province_to_state,
            state_owners: inputs.state_owners,
            state_victory_points: inputs.state_victory_points,
            capital_states: &capital_states,
            localisation: inputs.localisation,
            vp_icon_defines: inputs.vp_icon_defines,
        };
        Self::from_law_inputs(inputs.width, inputs.height, &replay_inputs)
    }
}

/// Selected-hour current capitals for map/UI consumers.
///
/// `identity.original_capital` is deliberately absent from this projection:
/// it is historical provenance, while the engine's map/UI consumers resolve
/// the relocated/current capital. Unobtained/absent/zero values fail closed.
pub fn replay_current_capital_states(frame: &WorldFrame) -> BTreeMap<String, u32> {
    measured_current_capital_states(&frame.diplomacy().countries)
}

fn measured_current_capital_states(
    countries: &BTreeMap<String, CountryDiplomacy>,
) -> BTreeMap<String, u32> {
    countries
        .iter()
        .filter_map(|(tag, country)| match &country.capital_state {
            Leaf::Measured(state_id) if *state_id > 0 => Some((tag.clone(), *state_id)),
            Leaf::Measured(_) | Leaf::Absent | Leaf::Unobtained => None,
        })
        .collect()
}

/// Static game-file reference layer only. Scrubbed replay frames must use
/// [`ReplayFurnitureLayer::from_world_frame`] so relocations are not lost.
pub fn load_static_reference_furniture_layer(
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

    #[test]
    fn world_frame_current_capital_replaces_static_history_capital() {
        // The typed WorldFrame construction is covered by replay_core's
        // fx_capital_moves gate. This bridge gate proves the typed measured
        // current leaf replaces the history-file input at the marker law.
        let province_ids = vec![100, 100, 200, 200];
        let province_to_state = BTreeMap::from([(100, 5), (200, 6)]);
        let state_owners = BTreeMap::from([
            (5, "FRA".to_string()),
            (6, "FRA".to_string()),
        ]);
        let state_victory_points = BTreeMap::from([
            (5, BTreeMap::from([(100, 10)])),
            (6, BTreeMap::from([(200, 8)])),
        ]);
        let history_capitals = BTreeMap::from([("FRA".to_string(), 5)]);
        let current_capitals = measured_current_capital_states(&BTreeMap::from([(
            "FRA".to_string(),
            CountryDiplomacy {
                faction_name: Leaf::Unobtained,
                faction_is_leader: Leaf::Unobtained,
                capital_state: Leaf::Measured(6),
                is_major: Leaf::Unobtained,
            },
        )]));
        let localisation = BTreeMap::new();
        let history_inputs = FurnitureLawInputs {
            province_ids: &province_ids,
            width: 4,
            height: 1,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            state_victory_points: &state_victory_points,
            capital_states: &history_capitals,
            localisation: &localisation,
            vp_icon_defines: None,
        };
        let current_inputs = FurnitureLawInputs {
            capital_states: &current_capitals,
            ..history_inputs
        };

        let markers = compute_furniture_markers(&current_inputs);
        assert_eq!(
            markers
                .iter()
                .find(|marker| marker.kind == FurnitureMarkerKind::Capital)
                .map(|marker| marker.state_id),
            Some(6)
        );
    }
}
