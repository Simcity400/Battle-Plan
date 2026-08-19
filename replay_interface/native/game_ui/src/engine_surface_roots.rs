
pub type EngineControllerRootSet = &'static [&'static str];

pub const ENGINE_CONTROLLER_ROOTS: EngineControllerRootSet = &[
    "topbar",
    "mini_map_interface",
    "MapmodesInterface_Ingame",
    "theatre_selection_window",
    "countryequipmentdesignerview",
    "tank_designer_view",
    "plane_designer_view",
    "countrydivisiondesignerview",
    "equipment_upgrade_designer_window",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn engine_controller_roots_are_unique_and_include_all_native_designers() {
        assert_eq!(
            ENGINE_CONTROLLER_ROOTS
                .iter()
                .map(|root| root.to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
                .len(),
            ENGINE_CONTROLLER_ROOTS.len()
        );
        assert!(ENGINE_CONTROLLER_ROOTS.contains(&"countrydivisiondesignerview"));
        assert!(ENGINE_CONTROLLER_ROOTS.contains(&"equipment_upgrade_designer_window"));
    }
}
