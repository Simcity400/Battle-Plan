use super::*;
use content_reader_script::modifiers::{Modifier, Modifiers};
use replay_data::rows::ModifierDefinitionRow;
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn naval_terrain_shares_the_resolved_land_definitions() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!("hoi4_shared_terrain_{}", std::process::id()));
    let folder = root.join("common/terrain");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("synthetic.txt"),
        b"categories={synthetic={movement_cost=1.5}} ",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let land = Arc::new(content_reader_key_names::terrain::read_definitions(&layers).unwrap());
    let naval = Content::load(&layers, &Defines::new().unwrap(), Arc::clone(&land)).unwrap();
    assert!(Arc::ptr_eq(&land, &naval.terrain));
    assert_eq!(naval.terrain.categories[0].movement_cost, 150_000);
    drop(layers);
    std::fs::remove_dir_all(root).unwrap();
}

fn modifier(key: &str, value: i64) -> Modifiers {
    let mut modifiers = Modifiers::default();
    modifiers.values = vec![Modifier {
        key: key.into(),
        name: key.into(),
        text: String::new(),
        value,
    }];
    modifiers
}

#[test]
fn naval_terrain_prefix_includes_minimum_and_class_before_weather() {
    let mut terrain = Content::test_empty(b"water_deep_ocean");
    let category = &mut Arc::make_mut(&mut terrain.terrain).categories[0];
    category.minimum_seazone_dominance = 250 * ONE;
    category.common_adjuster.attack = -10_000;
    category.common_adjuster.defence = -30_000;
    category.class_adjusters.push(TerrainAdjuster {
        movement: -20_000,
        ..Default::default()
    });
    category.modifiers = modifier("positioning", -5_000);
    category.modifiers.values.push(Modifier {
        key: "unlisted_common".into(),
        name: "unlisted_common".into(),
        text: String::new(),
        value: 1,
    });
    category
        .classes
        .push((b"destroyer".to_vec(), modifier("navy_visibility", 10_000)));
    category.classes[0].1.values.push(Modifier {
        key: "unlisted_class".into(),
        name: "unlisted_class".into(),
        text: String::new(),
        value: 1,
    });
    category
        .classes
        .push((b"destroyer".to_vec(), modifier("navy_visibility", -50_000)));
    category
        .classes
        .push((b"DESTROYER".to_vec(), modifier("navy_visibility", -75_000)));
    category.class_adjusters.push(TerrainAdjuster::default());
    category.class_adjusters.push(TerrainAdjuster::default());
    terrain.ship_classes.push(b"destroyer".to_vec());
    terrain.dominance_ratio = Ok(60_000);
    let rows = [
        ModifierDefinitionRow {
            index: 7,
            name_key: b"POSITIONING".to_vec(),
            category_mask: 1,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 8,
            name_key: b"VISIBILITY".to_vec(),
            category_mask: 1,
            ..Default::default()
        },
    ];
    let definitions =
        Definitions::from_parts(&rows, &[(b"positioning", 7), (b"navy_visibility", 8)], None);
    let loc: Localisation = [
        ("water_deep_ocean", "Deep Ocean"),
        ("destroyer", "Destroyer"),
        ("HUN_destroyer", "Hungarian Destroyer"),
        ("OTHER_SHIPS", "Other ships"),
        ("MINIMUM_SEAZONE_DOMINANCE_NEEDED", "Dominance $VALUE|0$"),
        ("POSITIONING", "Positioning"),
        ("VISIBILITY", "Visibility"),
        ("STAT_ADJUSTER_MOVEMENT", "Movement "),
        ("STAT_ADJUSTER_MOVEMENT_DIFF", "$VALUE|%0$"),
        ("STAT_ADJUSTER_ATTACK", "Attack "),
        ("STAT_ADJUSTER_ATTACK_DIFF", "$VALUE|%0$"),
        ("STAT_ADJUSTER_DEFENCE", "Defence "),
        ("STAT_ADJUSTER_DEFENCE_DIFF", "$VALUE|%0$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let mut dropped = Vec::new();
    let text = terrain
        .prefix(
            b"water_deep_ocean",
            &definitions,
            &NoIcons,
            &loc,
            Ok("HUN"),
            &mut dropped,
        )
        .unwrap();
    assert_eq!(
        text,
        "\u{11}HDeep Ocean\u{11}!\nDominance 375\n\n\u{11}HHungarian Destroyer\u{11}!\nPositioning: \u{11}G-0\u{11}!\nVisibility: \u{11}R+0\u{11}!\nMovement -20%\nAttack -10%\nDefence -30%\n\n\u{11}HOther ships\u{11}!\nPositioning: \u{11}G-0\u{11}!\nAttack -10%\nDefence -30%\n"
    );
    assert_eq!(dropped.len(), 2);
    assert!(dropped[0].contains("unlisted_common"));
    assert!(dropped[1].contains("unlisted_class"));
    assert!(
        terrain
            .prefix(
                b"WATER_DEEP_OCEAN",
                &definitions,
                &NoIcons,
                &loc,
                Ok("HUN"),
                &mut Vec::new()
            )
            .is_err()
    );
}

#[test]
fn naval_terrain_extra_fields_follow_native_category_reader() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!("hoi4_naval_terrain_{}", std::process::id()));
    let path = root.join("common/terrain");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("a.txt"), b"@minimum = 250\ncategories = { water = { minimum_seazone_dominance = @minimum units = { attack = -0.1 } destroyer = { units = { movement = -0.2 } } } }").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let read = content_reader_key_names::terrain::read_definitions(&layers).unwrap();
    assert_eq!(read.categories.len(), 1);
    assert_eq!(read.categories[0].minimum_seazone_dominance, 250 * ONE);
    assert_eq!(read.categories[0].common_adjuster.attack, -10_000);
    assert_eq!(read.categories[0].class_adjusters[0].movement, -20_000);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn filtered_nonempty_class_and_common_blocks_keep_native_newlines() {
    let mut terrain = Content::test_empty(b"sea");
    terrain.ship_classes.push(b"destroyer".to_vec());
    Arc::make_mut(&mut terrain.terrain).categories[0].modifiers = modifier("common", 10_000);
    Arc::make_mut(&mut terrain.terrain).categories[0]
        .classes
        .push((b"destroyer".to_vec(), modifier("class", 10_000)));
    let rows = [
        ModifierDefinitionRow {
            index: 1,
            name_key: b"COMMON".to_vec(),
            category_mask: 0,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 2,
            name_key: b"CLASS".to_vec(),
            category_mask: 0,
            ..Default::default()
        },
    ];
    let definitions = Definitions::from_parts(&rows, &[(b"common", 1), (b"class", 2)], None);
    let loc: Localisation = [
        ("sea", "Sea"),
        ("destroyer", "Destroyer"),
        ("OTHER_SHIPS", "Other ships"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let text = terrain
        .prefix(
            b"sea",
            &definitions,
            &NoIcons,
            &loc,
            Ok("AAA"),
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        text,
        "\u{11}HSea\u{11}!\n\n\u{11}HDestroyer\u{11}!\n\n\n\u{11}HOther ships\u{11}!\n\n"
    );
}
