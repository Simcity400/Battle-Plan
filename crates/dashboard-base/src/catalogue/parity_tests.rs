use super::{Catalogue, tests::Folder};
use crate::products::equipment_icon::IconContent;
use installed_content_core::equipment::Database;
use std::sync::Arc;

fn equipment_catalogue(equipment: &[(&str, &str)]) -> (Arc<Catalogue>, Folder) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut files = vec![
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("map/continent.txt", "continents={synthetic_continent}"),
        (
            "common/defines/synthetic.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
        ),
    ];
    files.extend_from_slice(equipment);
    let (catalogue, folder) = super::tests::folder_catalogue(&files);
    (Arc::new(catalogue), folder)
}

#[test]
fn d16_equipment_reuse_verifies_covered_reads_in_native_order() {
    let files = [
        ("common/units/equipment/upgrades/z.txt", "upgrades={}"),
        (
            "common/units/equipment/modules/z.txt",
            "equipment_modules={}",
        ),
        (
            "common/units/equipment/a.txt",
            "equipments={synthetic_type={}}",
        ),
    ];
    for remove in [false, true] {
        // Each directory is verified, and simultaneous changes retain the first
        // refusal from the native upgrades/modules/equipment read order.
        for changed in [vec![0], vec![1], vec![2], vec![0, 1, 2]] {
            let (catalogue, folder) = equipment_catalogue(&files);
            let layers = &catalogue.content.layers;
            let names = layers.key_names().unwrap();
            assert!(names.equipment_database().unwrap().lost.is_none());
            assert!(IconContent::load(Arc::clone(&catalogue)).is_ok());
            for index in changed {
                let path = folder.0.join(files[index].0);
                if remove {
                    std::fs::remove_file(path).unwrap();
                } else {
                    std::fs::write(path, "changed covered payload").unwrap();
                }
            }
            let expected = Database::load_complete(layers).err().unwrap();
            let actual = IconContent::load(Arc::clone(&catalogue))
                .err()
                .expect("equipment reuse must retain the complete loader's verified reads");
            assert_eq!(actual.kind(), expected.kind());
            assert_eq!(actual.to_string(), expected.to_string());
        }
    }
}

#[test]
fn d16_equipment_reuse_keeps_the_complete_database_refusal() {
    let (catalogue, _folder) = equipment_catalogue(&[(
        "common/units/equipment/synthetic.txt",
        "equipments={synthetic_type={alias=7} later={}}",
    )]);
    let layers = &catalogue.content.layers;
    let names = layers.key_names().unwrap();
    let lost = names.equipment_database().unwrap().lost.as_ref().unwrap();
    assert!(lost.contains("synthetic_type"), "{lost}");
    let expected = Database::load_complete(layers).err().unwrap();
    assert_eq!(
        expected.to_string(),
        format!("the equipment database cannot be followed past {lost}")
    );
    let actual = IconContent::load(catalogue).err().unwrap();
    assert_eq!(actual.kind(), expected.kind());
    assert_eq!(actual.to_string(), expected.to_string());
}
