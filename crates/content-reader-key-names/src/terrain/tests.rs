use super::*;

fn database(sources: &[&str]) -> TerrainDefinitions {
    let mut definitions = TerrainDefinitions::default();
    for source in sources {
        definitions.append(source.as_bytes()).unwrap();
    }
    definitions
}

fn category(definitions: &TerrainDefinitions, index: u8) -> Option<&[u8]> {
    let at = definitions.palette.get(&index)?.category?;
    Some(&definitions.categories[at].name)
}

#[test]
fn naval_terrain_keeps_common_and_ordered_class_modifier_operands() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "categories={ ocean={ minimum_seazone_dominance=250 units={ attack=-0.1 } naval_range=1.25 custom_hull={ naval_range=0.5 units={ movement=-0.2 } } custom_hull={ naval_range=-0.2 units={ defence=0.3 } } } }",
    ]);
    let terrain = &definitions.categories[0];
    assert_eq!(terrain.modifiers.values[0].value, 125_000);
    assert_eq!(terrain.classes.len(), 2);
    assert_eq!(terrain.classes[0].0, b"custom_hull");
    assert_eq!(terrain.classes[0].1.values[0].value, 50_000);
    assert_eq!(terrain.classes[1].1.values[0].value, -20_000);
    assert_eq!(terrain.minimum_seazone_dominance, 25_000_000);
    assert_eq!(terrain.common_adjuster.attack, -10_000);
    assert_eq!(terrain.class_adjusters.len(), 2);
    assert_eq!(terrain.class_adjusters[0].movement, -20_000);
    assert_eq!(terrain.class_adjusters[1].defence, 30_000);
    assert_eq!(
        terrain.modifiers.values[0]
            .value
            .wrapping_add(terrain.classes[0].1.values[0].value),
        175_000
    );
}

#[test]
fn category_colours_read_over_white() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "categories={ bare={} forest={ color={ 89 199 85 } } pale={ color={ 0.5 0.25 1 0.5 } } packed={ color=0x80102030 } bad={ color={ 10 20 30 } color=0xg } zero={ color={ 10 20 30 } color=0x } }",
    ]);
    let colors: Vec<_> = definitions
        .categories
        .iter()
        .map(TerrainCategory::color)
        .collect();
    assert_eq!(
        colors,
        [
            Ok([1.0; 4]),
            Ok([89.0 / 255.0, 199.0 / 255.0, 85.0 / 255.0, 1.0]),
            Ok([0.5, 0.25, 1.0, 0.5]),
            Ok([16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0, 128.0 / 255.0]),
            Ok([10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]),
            Ok([0.0; 4]),
        ]
    );
    assert_eq!(definitions.skipped.len(), 1, "{:?}", definitions.skipped);
    assert!(definitions.skipped[0].contains("0xg"), "{:?}", definitions.skipped);
}

#[test]
fn water_reads_its_regions_naval_terrain_or_ocean() {
    content_reader_script::test_game::enter();
    let definitions = database(&["categories={ deep={} Ocean={} ocean={} }"]);
    assert_eq!(definitions.naval_category(Some("DEEP")), Some(0));
    assert_eq!(definitions.naval_category(Some("missing")), Some(1));
    assert_eq!(definitions.naval_category(None), Some(1));
    let without = database(&["categories={ deep={} }"]);
    assert_eq!(without.naval_category(None), None);
}

#[test]
fn a_redeclared_category_appends_and_a_lookup_takes_the_first() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "categories = { plains = { movement_cost = 1 } Plains = { movement_cost = 2 } }",
        "categories = { plains = { movement_cost = 3 } } terrain = { a = { type = PLAINS color = { 0 } } }",
    ]);
    let names: Vec<&[u8]> = definitions
        .categories
        .iter()
        .map(|category| category.name.as_slice())
        .collect();
    assert_eq!(names, [&b"plains"[..], b"Plains", b"plains"]);
    assert_eq!(definitions.category(b"PLAINS"), Some(0));
    assert_eq!(definitions.category(b"missing"), None);
    assert_eq!(definitions.palette[&0].category, Some(0));
    assert_eq!(definitions.categories[2].movement_cost, 300_000);
}

#[test]
fn a_type_sees_only_the_categories_read_before_it() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "terrain = { early = { type = hills color = { 1 } } } \
         categories = { plains = { } } \
         terrain = { same_file = { type = plains color = { 2 } } \
                     ahead = { type = hills color = { 3 } } }",
        "categories = { hills = { } } \
         terrain = { later = { type = Plains color = { 4 } } \
                     own = { type = hills color = { 5 } } }",
    ]);
    assert_eq!(definitions.palette[&1].category, None);
    assert_eq!(definitions.palette[&3].category, None);
    assert_eq!(category(&definitions, 2), Some(&b"plains"[..]));
    assert_eq!(category(&definitions, 4), Some(&b"plains"[..]));
    assert_eq!(category(&definitions, 5), Some(&b"hills"[..]));
}

#[test]
fn entries_keep_their_fields_and_the_last_palette_writer_wins() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "@snow = yes categories = { plains = { } desert = { } } \
         terrain = { desert = { type = plains color = { 11 } texture = 11 } \
                     desert = { type = desert type = plains color = { 12 } texture = 8 texture = 9 } \
                     snow = { type = desert color = { 16 } color = { 17 300 } perm_snow = @snow } \
                     city = { type = plains color = { 13 } spawn_city = yes } \
                     bare = { type = plains } }",
        "terrain = { override = { type = desert color = { 11 } } }",
    ]);
    assert_eq!(definitions.palette[&12].name, "desert");
    assert_eq!(definitions.palette[&12].atlas_index, 9);
    assert_eq!(category(&definitions, 12), Some(&b"plains"[..]));
    assert_eq!(definitions.palette[&16], definitions.palette[&17]);
    assert!(definitions.palette[&16].perm_snow);
    assert_eq!(definitions.palette.len(), 5);
    assert!(definitions.palette[&13].spawn_city);
    assert_eq!(definitions.palette[&13].atlas_index, 0);
    assert_eq!(definitions.palette[&11].name, "override");
    assert_eq!(category(&definitions, 11), Some(&b"desert"[..]));
}

#[test]
fn combat_widths_keep_their_value_past_a_failed_conversion() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "@w = 42 categories={urban={combat_width=96 combat_support_width=32 combat_width=@w} \
          marsh={combat_width=0.5} bad={combat_support_width=wide} none={} \
          kept={COMBAT_WIDTH=40 combat_width=wide} scoped={@s = 12 combat_width=@s} \
          quoted={\"combat_width\"=70 combat_support_width=\"@w\"} \
          late={combat_width=@later} @later = 9 after={combat_width=@later}}",
    ]);
    let widths: Vec<_> = definitions
        .categories
        .iter()
        .map(|c| (c.combat_width, c.combat_support_width))
        .collect();
    assert_eq!(widths[0], (4_200_000, 3_200_000));
    assert_eq!(widths[1], (50_000, 0));
    assert_eq!(widths[2], (0, 0));
    assert_eq!(widths[3], (0, 0));
    assert_eq!(widths[4], (4_000_000, 0));
    assert_eq!(widths[5], (1_200_000, 0));
    assert_eq!(widths[6], (0, 0));
    assert_eq!(widths[7], (0, 0));
    assert_eq!(widths[8], (900_000, 0));
    let skipped = &definitions.skipped;
    assert_eq!(skipped.len(), 4, "{skipped:?}");
    assert!(
        skipped[0].contains("terrain category bad: combat_support_width wide does not convert")
    );
    assert!(skipped[1].ends_with("keeps 40"), "{}", skipped[1]);
    assert!(skipped[2].contains("combat_support_width @w does not convert"));
    assert!(skipped[3].contains("combat_width @later does not convert"));
}

#[test]
fn authored_cost_keeps_radix_prefixes_fraction_and_clamp() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "categories={octal={movement_cost=010} hex={movement_cost=0x10} suffix={movement_cost=1junk.25} signed={movement_cost=1.-2} clamped={movement_cost=08} invalid={movement_cost=bad} omitted={}}",
    ]);
    let costs: Vec<_> = definitions
        .categories
        .iter()
        .map(|category| category.movement_cost)
        .collect();
    assert_eq!(
        costs,
        [800_000, 1_600_000, 125_000, 98_000, 1_000, 1_000, 0]
    );
    assert!(definitions.skipped[0].contains("movement_cost bad does not convert"));
    assert_eq!(content_reader_parser::fixed5::script_fixed5(b"-0.1234567"), Some(-12_345));
    assert_eq!(
        content_reader_parser::fixed5::script_fixed5(b"92233720368547.75807"),
        Some(i64::MAX)
    );
}

#[test]
fn a_malformed_entry_is_skipped_and_named_and_unsupported_forms_are_refused() {
    content_reader_script::test_game::enter();
    for (entry, message) in [
        ("plains", "key `plains` has no operator"),
        (
            "plains = 1",
            "field `plains` is `1` where its reader reads a block",
        ),
    ] {
        let source = format!("categories = {{ {entry} forest = {{ }} }}");
        let loaded = parse_definitions(source.as_bytes()).unwrap();
        let names: Vec<&[u8]> = loaded
            .categories
            .iter()
            .map(|category| category.name.as_slice())
            .collect();
        assert_eq!(names, [b"plains".as_slice()], "{source}");
        assert!(
            loaded.skipped[0].contains(message),
            "{source}: {:?}",
            loaded.skipped
        );
    }
    let entry = |fields: &str| {
        let source = format!(
            "categories = {{ c = {{ }} }} terrain = {{ a = {{ {fields} }} b = {{ color = {{ 1 }} }} }}"
        );
        parse_definitions(source.as_bytes()).unwrap()
    };
    let loaded = entry("color = { 3 } texture = x texture = 5 perm_snow = maybe spawn_city = yes");
    assert_eq!(loaded.skipped.len(), 2, "{:?}", loaded.skipped);
    assert!(loaded.skipped[0].contains("texture is not an i32"));
    assert!(loaded.skipped[1].contains("perm_snow is not yes or no"));
    let a = &loaded.palette[&3];
    assert_eq!((a.atlas_index, a.perm_snow, a.spawn_city), (5, false, true));
    assert_eq!(loaded.palette[&1].name, "b");
    let loaded = entry("color = { 3 } texture = 5 texture = x type = c");
    assert_eq!(loaded.palette[&3].atlas_index, 5);
    assert_eq!(loaded.palette[&3].category, Some(0));
    let loaded = entry("color = { 3 } type = c type = { }");
    assert_eq!(loaded.palette[&3].category, None);
    assert!(
        loaded
            .skipped
            .iter()
            .any(|note| note.contains("type { names no category")),
        "{:?}",
        loaded.skipped
    );
    let loaded = entry("color = { x 3 }");
    assert!(loaded.skipped[0].contains("reads as 0"));
    assert_eq!(loaded.palette[&0].name, "a");
    assert_eq!(loaded.palette[&3].name, "a");
    for slot in [-1, -257, i32::MIN, i32::MAX] {
        let loaded = entry(&format!("color = {{ 0 }} texture = {slot}"));
        assert_eq!(loaded.palette[&0].atlas_index, slot as u32);
        assert_eq!(loaded.palette[&0].atlas_index as u8, slot as u8);
    }
    for fields in ["color = { -1 }"] {
        let loaded = entry(fields);
        assert!(
            loaded.skipped[0].contains("skipped"),
            "{fields}: {:?}",
            loaded.skipped
        );
        assert!(!loaded.palette.contains_key(&0), "{fields}");
        assert_eq!(loaded.palette[&1].name, "b");
    }
    let loaded = parse_definitions(b"categories = { c = { } } terrain = { a = 1 }").unwrap();
    assert!(loaded.skipped[0].contains("field `a` is `1` where its reader reads a block"));
    assert!(loaded.palette.is_empty());
    let loaded = parse_definitions(
        b"\"categories\" = { skipped = { } } categories = { a = { \"movement_cost\" # comment
 = 2 } } terrain = { t = { \"texture\" = 1 TEXTURE = 3 Color = { 5 } } }",
    )
    .unwrap();
    assert_eq!(loaded.categories.len(), 1);
    assert_eq!(loaded.categories[0].movement_cost, 0);
    assert_eq!(loaded.palette[&5].atlas_index, 3);
    let error = parse_definitions(b"terrain = { }").unwrap_err().to_string();
    assert!(error.contains("declares no terrain category"), "{error}");
    let error = parse_definitions(
        b"categories = { a = { units = { attack = -0.1 } forest = { odd = { } } } }",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.starts_with(
            "terrain: line 1: odd has a block value, which modifier value reader reads"
        ),
        "{error}"
    );
    let error =
        parse_definitions(b"categories = { a = { buildings_max_level = { workshop = { } } } }")
            .unwrap_err()
            .to_string();
    assert!(
        error.starts_with(
            "terrain: line 1: workshop is read by the terrain building-level statement reader"
        ),
        "{error}"
    );
}

#[test]
fn default_map_names_its_bitmaps_and_tree_indices() {
    content_reader_script::test_game::enter();
    let map = parse_default_map(
        br#"definitions = "definition.csv"
            provinces = "provinces.bmp"
            positions = "positions.txt"
            terrain = "terrain.bmp"
            rivers = "rivers.bmp"
            heightmap = "heightmap.bmp"
            tree_definition = "trees.bmp"
            continent = "continent.txt"
            #climate = "climate.txt"
            tree = { 3 4 7 10 }"#,
    )
    .unwrap();
    assert_eq!(map.provinces, "provinces.bmp");
    assert_eq!(map.terrain, "terrain.bmp");
    assert_eq!(map.tree_definition, "trees.bmp");
    assert_eq!(map.tree_indices, vec![3, 4, 7, 10]);
    assert_eq!(
        map.other,
        vec![
            ("positions".into(), "positions.txt".to_owned()),
            ("continent".into(), "continent.txt".to_owned()),
        ]
    );
}

#[test]
fn a_default_map_without_a_required_name_is_refused() {
    content_reader_script::test_game::enter();
    let complete = br#"definitions = "d" provinces = "p" terrain = "t" rivers = "r"
         heightmap = "h" tree_definition = "td""#;
    assert!(parse_default_map(complete).unwrap().tree_indices.is_empty());
    for key in [
        "definitions",
        "provinces",
        "terrain",
        "rivers",
        "heightmap",
        "tree_definition",
    ] {
        let broken = String::from_utf8(complete.to_vec())
            .unwrap()
            .replace(key, "removed");
        let error = parse_default_map(broken.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("default.map has no {key}")),
            "{error}"
        );
    }
    assert!(parse_default_map(br#"tree = { 300 }"#).is_err());
}

#[test]
fn the_replay_classes_hold_for_terrain() {
    content_reader_script::test_game::enter();
    let definitions = database(&[
        "@w = 30 categories = { a = { \"combat_width\" = 10 COMBAT_WIDTH = @w } \
         b = { movement_cost = { combat_width = 20 } combat_support_width = 5 } }",
        "categories = { c = { combat_width = @w } \
         d = { combat_width = 40",
    ]);
    let widths: Vec<_> = definitions
        .categories
        .iter()
        .map(|c| {
            (
                String::from_utf8_lossy(&c.name).into_owned(),
                c.combat_width,
                c.combat_support_width,
            )
        })
        .collect();
    assert_eq!(
        widths,
        [
            ("a".to_owned(), 3_000_000, 0),
            ("b".to_owned(), 2_000_000, 0),
            ("combat_support_width".to_owned(), 0, 0),
            ("c".to_owned(), 0, 0),
            ("d".to_owned(), 4_000_000, 0),
        ]
    );
    assert!(
        definitions
            .skipped
            .iter()
            .any(|note| note.contains("field `movement_cost` is a block")),
        "{:?}",
        definitions.skipped
    );
}
