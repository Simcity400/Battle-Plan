use content_reader_key_names::key_name_products::KeyNamesExt as _;
use super::{Catalogue, tests::Folder};
use crate::products::equipment_icon::IconContent;
use content_reader_key_names::equipment::Database;
use std::sync::Arc;

pub(super) fn equipment_catalogue(equipment: &[(&str, &str)]) -> (Arc<Catalogue>, Folder) {
    let mut files = vec![
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;unknown;0\n"),
        ("map/continent.txt", "continents={synthetic_continent}"),
        (
            "common/defines/synthetic.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=13}}",
        ),
    ];
    files.extend_from_slice(equipment);
    let (_, folder) = super::tests::folder_catalogue(&files);
    let mut bmp = vec![0_u8; 58];
    bmp[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bmp[54..57].copy_from_slice(&[0, 0, 1]);
    std::fs::write(folder.0.join("map/provinces.bmp"), bmp).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&folder.0, &[])]).unwrap();
    (Arc::new(super::tests::layers_catalogue(layers)), folder)
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
        for changed in [vec![0], vec![1], vec![2], vec![0, 1, 2]] {
            let (catalogue, folder) = equipment_catalogue(&files);
            let layers = &catalogue.content.layers;
            let names = layers.key_name_products().unwrap();
            assert!(names.equipment_database().unwrap().lost.is_none());
            assert!(IconContent::load(Arc::clone(&catalogue)).is_ok());
            let first_changed = files[changed[0]].0;
            for index in changed {
                let path = folder.0.join(files[index].0);
                if remove {
                    std::fs::remove_file(path).unwrap();
                } else {
                    std::fs::write(path, "changed covered payload").unwrap();
                }
            }
            let expected = layers.read(first_changed).expect_err("the first changed file must fail its verified read");
            let actual = IconContent::load(Arc::clone(&catalogue))
                .err()
                .expect("equipment reuse must verify every file in native order");
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
    let names = layers.key_name_products().unwrap();
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

#[test]
fn equipment_units_reuses_the_recorded_database() {
    let (catalogue, _folder) = equipment_catalogue(&[(
        "common/units/equipment/synthetic.txt",
        "equipments={synthetic_type={}}",
    )]);
    let recorded = Database::shared(&catalogue.content.layers).unwrap();
    let dashboard = super::equipment_units(&catalogue).unwrap();
    assert!(Arc::ptr_eq(&recorded, &dashboard));
}

#[test]
fn icon_content_reads_only_files_the_store_key_stamps() {
    use crate::products::equipment_icon::inputs_stamped;
    let (catalogue, _folder) = equipment_catalogue(&[
        ("common/units/equipment/upgrades/z.txt", "upgrades={}"),
        (
            "common/units/equipment/modules/z.txt",
            "equipment_modules={}",
        ),
        (
            "common/units/equipment/a.txt",
            "equipments={synthetic_type={}}",
        ),
        (
            "gfx/interface/equipmentdesigner/graphic_db/a.txt",
            "default = { infantry_equipment_1 = { pool = { icons = { GFX_fallback } } } }",
        ),
    ]);
    let reads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let traced = {
        let reads = Arc::clone(&reads);
        catalogue
            .content
            .layers
            .with_read_trace(Arc::new(move |read: content_reader_layers::content::ContentRead| {
                reads.lock().unwrap().push(read.path);
            }))
    };
    let traced = Arc::new(super::tests::layers_catalogue(traced));
    reads.lock().unwrap().clear();
    IconContent::load(traced).unwrap();
    let reads: Vec<String> = std::mem::take(&mut *reads.lock().unwrap())
        .into_iter()
        .map(|path| String::from_utf8(path).unwrap())
        .collect();
    for read in [
        "map/default.map",
        "map/definition.csv",
        "map/provinces.bmp",
        "map/continent.txt",
        "gfx/interface/equipmentdesigner/graphic_db/a.txt",
    ] {
        assert!(reads.iter().any(|path| path == read), "{read}: {reads:?}");
    }
    let layers = &catalogue.content.layers;
    let stamped = layers.recorded_source_files().unwrap();
    for read in &reads {
        assert!(
            stamped
                .iter()
                .any(|path| path.ends_with(std::path::Path::new(read))),
            "{read} is not stamped: {stamped:?}"
        );
    }
    assert!(inputs_stamped(layers).unwrap());
    let (uncovered, _folder) = super::tests::folder_catalogue(&[
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=music/continent.txt",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0
"),
        ("map/music/continent.txt", "continents={synthetic_continent}"),
    ]);
    assert!(!inputs_stamped(&uncovered.content.layers).unwrap());
}
