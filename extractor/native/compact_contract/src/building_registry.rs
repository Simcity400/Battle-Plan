//! by the collector and recorder so an id cannot acquire different names at

pub const BUILDING_REGISTRY_EXE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const BUILDING_TYPE_ID_TO_TOKEN: &[(u32, &str)] = &[
    (0x27D2, "naval_supply_hub"),
    (0x2FAB, "infrastructure"),
    (0x2FB6, "air_base"),
    (0x31F6, "naval_base"),
    (0x32B4, "nuclear_reactor"),
    (0x33F7, "rocket_site"),
    (0x3C8F, "fuel_silo"),
    (0x4BF6, "dockyard"),
    (0x4CBE, "supply_node"),
    (0x4CC1, "rail_way"),
    (0x4E22, "arms_factory"),
    (0x4E2D, "industrial_complex"),
    (0x4E44, "synthetic_refinery"),
    (0x5751, "bunker"),
    (0x5752, "coastal_bunker"),
    (0x7542, "naval_facility"),
    (0x760B, "stronghold_network"),
    (0x766E, "anti_air_building"),
    (0x7702, "radar_station"),
    (0x7734, "mega_gun_emplacement"),
    (0x77C9, "naval_headquarters"),
    (0x782E, "nuclear_reactor_heavy_water"),
    (0x7860, "commercial_nuclear_reactor"),
    (0x7892, "nuclear_facility"),
    (0x78C5, "air_facility"),
    (0x78F7, "land_facility"),
    (0x7929, "dam"),
    (0x7940, "dam_mountain"),
    (0x798C, "cataract_dam_mountain"),
    (0x79BE, "canal_kiel"),
    (0x79F2, "canal_panama"),
    (0x7A24, "energy_infrastructure"),
    (0x7A57, "industrial_infrastructure"),
    (0x7A89, "landmark_big_ben"),
    (0x7ABC, "landmark_colosseum"),
    (0x7AEE, "landmark_cristo_redentor"),
    (0x7B20, "landmark_eiffel_tower"),
    (0x7B52, "landmark_statue_of_liberty"),
    (0x7B84, "landmark_kremlin"),
    (0x7BB6, "landmark_hofburg_palace"),
    (0x7BE8, "landmark_berlin_reichstag"),
    (0x7C1A, "landmark_berlin_volkshalle"),
    (0x7C4C, "landmark_taj_mahal"),
    (0x7C7E, "landmark_sadabad_complex"),
    (0x7CB0, "landmark_hagia_sophia"),
    (0x7CE2, "landmark_forbidden_city"),
    (0x7D14, "landmark_nanjing_presidential_palace"),
    (0x7D46, "landmark_nanjing_presidential_palace_prc"),
    (0x7D78, "landmark_nanjing_presidential_palace_gen"),
    (0x7DAA, "landmark_great_wall_section"),
    (0x7DDC, "landmark_tokyo_imperial_palace"),
    (0x7E0E, "landmark_national_diet"),
    (0x7E40, "landmark_hakko_ichiu"),
    (0x7E72, "landmark_prague_castle"),
    (0x7EA4, "landmark_bojnice_castle"),
];

pub fn building_token_from_type_id(type_id: u32) -> Option<&'static str> {
    BUILDING_TYPE_ID_TO_TOKEN
        .binary_search_by_key(&type_id, |(id, _)| *id)
        .ok()
        .map(|index| BUILDING_TYPE_ID_TO_TOKEN[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn pinned_registry_is_complete_sorted_and_unique() {
        assert_eq!(BUILDING_TYPE_ID_TO_TOKEN.len(), 55);
        assert!(BUILDING_TYPE_ID_TO_TOKEN
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(
            BUILDING_TYPE_ID_TO_TOKEN
                .iter()
                .map(|(_, token)| *token)
                .collect::<BTreeSet<_>>()
                .len(),
            55
        );
    }

    #[test]
    fn current_build_shifted_and_uninstantiated_ids_are_pinned() {
        assert_eq!(
            building_token_from_type_id(0x766E),
            Some("anti_air_building")
        );
        assert_eq!(building_token_from_type_id(0x7702), Some("radar_station"));
        assert_eq!(
            building_token_from_type_id(0x760B),
            Some("stronghold_network")
        );
        assert_eq!(
            building_token_from_type_id(0x7E40),
            Some("landmark_hakko_ichiu")
        );
        assert_eq!(building_token_from_type_id(0xDEAD_BEEF), None);
    }
}
