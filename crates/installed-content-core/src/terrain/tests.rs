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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Repeated class names retain the first bucket; common raw values
    // are added before calculating the effective value.
    assert_eq!(
        terrain.modifiers.values[0]
            .value
            .wrapping_add(terrain.classes[0].1.values[0].value),
        175_000
    );
}

#[test]
fn category_colours_read_over_white() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definitions = database(&[
        "categories={ bare={} forest={ color={ 89 199 85 } } pale={ color={ 0.5 0.25 1 0.5 } } }",
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
            // Components at most one are kept; the fourth is the alpha as read.
            Ok([0.5, 0.25, 1.0, 0.5]),
        ]
    );
}

#[test]
fn water_reads_its_regions_naval_terrain_or_ocean() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definitions = database(&["categories={ deep={} Ocean={} ocean={} }"]);
    assert_eq!(definitions.naval_category(Some("DEEP")), Some(0));
    assert_eq!(definitions.naval_category(Some("missing")), Some(1));
    assert_eq!(definitions.naval_category(None), Some(1));
    let without = database(&["categories={ deep={} }"]);
    assert_eq!(without.naval_category(None), None);
}

#[test]
fn a_redeclared_category_appends_and_a_lookup_takes_the_first() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definitions = database(&[
        "terrain = { early = { type = hills color = { 1 } } } \
         categories = { plains = { } } \
         terrain = { same_file = { type = plains color = { 2 } } \
                     ahead = { type = hills color = { 3 } } }",
        "categories = { hills = { } } \
         terrain = { later = { type = Plains color = { 4 } } \
                     own = { type = hills color = { 5 } } }",
    ]);
    // Before any category, and a category of a later file: the null terrain.
    assert_eq!(definitions.palette[&1].category, None);
    assert_eq!(definitions.palette[&3].category, None);
    assert_eq!(category(&definitions, 2), Some(&b"plains"[..]));
    // A later file resolves a category an earlier file declared.
    assert_eq!(category(&definitions, 4), Some(&b"plains"[..]));
    assert_eq!(category(&definitions, 5), Some(&b"hills"[..]));
}

#[test]
fn entries_keep_their_fields_and_the_last_palette_writer_wins() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definitions = database(&[
        "@snow = yes categories = { plains = { } desert = { } } \
         terrain = { desert = { type = plains color = { 11 } texture = 11 } \
                     desert = { type = desert type = plains color = { 12 } texture = 8 texture = 9 } \
                     snow = { type = desert color = { 16 } color = { 17 300 } perm_snow = @snow } \
                     city = { type = plains color = { 13 } spawn_city = yes } \
                     bare = { type = plains } }",
        "terrain = { override = { type = desert color = { 11 } } }",
    ]);
    // Repeated names stay apart by index; a repeated field takes its last value.
    assert_eq!(definitions.palette[&12].name, "desert");
    assert_eq!(definitions.palette[&12].atlas_index, 9);
    assert_eq!(category(&definitions, 12), Some(&b"plains"[..]));
    // `color` appends; indices past a byte are not kept.
    assert_eq!(definitions.palette[&16], definitions.palette[&17]);
    assert!(definitions.palette[&16].perm_snow);
    assert_eq!(definitions.palette.len(), 5);
    assert!(definitions.palette[&13].spawn_city);
    assert_eq!(definitions.palette[&13].atlas_index, 0);
    // A later writer of index 11, in a later file, replaces the first.
    assert_eq!(definitions.palette[&11].name, "override");
    assert_eq!(category(&definitions, 11), Some(&b"desert"[..]));
}

/// Widths retain the last fixed5 value that converts, or constructor zero.
/// Failed conversions are logged and change nothing. Variables hold in
/// document order at every depth.
#[test]
fn combat_widths_keep_their_value_past_a_failed_conversion() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // No movement-cost clamp applies to widths.
    assert_eq!(widths[1], (50_000, 0));
    assert_eq!(widths[2], (0, 0));
    assert_eq!(widths[3], (0, 0));
    assert_eq!(widths[4], (4_000_000, 0));
    assert_eq!(widths[5], (1_200_000, 0));
    // A quoted key is no keyword; a quoted value is never a reference.
    assert_eq!(widths[6], (0, 0));
    // A reference before its definition keeps its text.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definitions = database(&[
        "categories={octal={movement_cost=010} hex={movement_cost=0x10} suffix={movement_cost=1junk.25} signed={movement_cost=1.-2} clamped={movement_cost=08} invalid={movement_cost=bad} omitted={}}",
    ]);
    let costs: Vec<_> = definitions
        .categories
        .iter()
        .map(|category| category.movement_cost)
        .collect();
    // A failed conversion retains zero, then clamps; an omitted cost
    // stays zero.
    assert_eq!(
        costs,
        [800_000, 1_600_000, 125_000, 98_000, 1_000, 1_000, 0]
    );
    assert!(definitions.skipped[0].contains("movement_cost bad does not convert"));
    assert_eq!(crate::fixed5::script_fixed5(b"-0.1234567"), Some(-12_345));
    assert_eq!(
        crate::fixed5::script_fixed5(b"92233720368547.75807"),
        Some(i64::MAX)
    );
}

#[test]
fn a_malformed_entry_is_skipped_and_named_and_unsupported_forms_are_refused() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // A category reads fields whatever its value. A key without a value
    // consumes the next token as its operator; a scalar entry reads tokens
    // after it as its fields, so forest becomes the first category's class
    // entry. Every recovery is noted.
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
    // A malformed field is logged and retains its previous value;
    // the entry and its other fields remain.
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
    // A later bad value preserves an earlier good one. A type value of
    // `{` looks up that text and reports the null terrain; the block's
    // closing brace then ends the entry.
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
    // A colour element that does not parse reads as index 0.
    let loaded = entry("color = { x 3 }");
    assert!(loaded.skipped[0].contains("reads as 0"));
    assert_eq!(loaded.palette[&0].name, "a");
    assert_eq!(loaded.palette[&3].name, "a");
    for slot in [-1, -257, i32::MIN, i32::MAX] {
        let loaded = entry(&format!("color = {{ 0 }} texture = {slot}"));
        assert_eq!(loaded.palette[&0].atlas_index, slot as u32);
        assert_eq!(loaded.palette[&0].atlas_index as u8, slot as u8);
    }
    // A negative palette index takes the native out-of-range write path.
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
    // An entry reads whatever its value. After a scalar it reads the
    // following tokens, here none, then is discarded without colours.
    let loaded = parse_definitions(b"categories = { c = { } } terrain = { a = 1 }").unwrap();
    assert!(loaded.skipped[0].contains("field `a` is `1` where its reader reads a block"));
    assert!(loaded.palette.is_empty());
    // Quoted keys are not keywords: file and terrain readers report and
    // skip them, while a category reads them as modifiers.
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
    // A block under a per-class modifier name is either its registered
    // value or skipped; the file is refused by name.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Quoted keys are not keywords and bare keys fold. Variables stay within
/// their file; a block where one fixed5 value is expected becomes the
/// category's own fields. The end of the text closes every loop.
#[test]
fn the_replay_classes_hold_for_terrain() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            // `movement_cost` read `{`; the block's field is b's own and
            // its `}` ended b, so `combat_support_width` is a category.
            ("b".to_owned(), 2_000_000, 0),
            ("combat_support_width".to_owned(), 0, 0),
            // The other file's variable is its text, which does not
            // convert.
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

/// A BMP of `rows` (bottom first, already padded) with a `dib`-byte header:
/// 40 for 24-bit BI_RGB or 124 for 32-bit BI_BITFIELDS with BGRA masks.
fn bitmap(width: i32, rows: &[&[u8]], dib: u32) -> Vec<u8> {
    let offset = 14 + dib;
    let pixels: Vec<u8> = rows.concat();
    let mut bytes = b"BM".to_vec();
    bytes.extend((offset + pixels.len() as u32).to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(offset.to_le_bytes());
    bytes.extend(dib.to_le_bytes());
    bytes.extend(width.to_le_bytes());
    bytes.extend((rows.len() as i32).to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    let (bits, compression) = if dib == 40 { (24_u16, 0_u32) } else { (32, 3) };
    bytes.extend(bits.to_le_bytes());
    bytes.extend(compression.to_le_bytes());
    bytes.extend((pixels.len() as u32).to_le_bytes());
    bytes.extend([0; 16]);
    if dib == 124 {
        for mask in [0x00FF_0000_u32, 0xFF00, 0xFF, 0xFF00_0000] {
            bytes.extend(mask.to_le_bytes());
        }
    }
    bytes.resize(offset as usize, 0);
    bytes.extend(pixels);
    bytes
}

#[test]
fn the_world_normal_keeps_the_clients_texture_padding() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let bytes = bitmap(
        2,
        &[
            &[1, 2, 3, 4, 5, 6, 0, 0],
            &[7, 8, 9, 10, 11, 12, 0, 0],
            &[13, 14, 15, 16, 17, 18, 0, 0],
        ],
        40,
    );
    let read = |path: &[u8]| match path {
        b"map/world_normal.bmp" => Ok(bytes.clone()),
        _ => Err(io::Error::from(io::ErrorKind::NotFound)),
    };
    let raw = bmp::decode_rgb(&bytes).unwrap();
    assert_eq!(world_normal(&read, Decode::default()).unwrap(), raw);
    let padded = world_normal(
        &read,
        Decode {
            force_pow2_textures: true,
        },
    )
    .unwrap();
    assert_eq!([padded.width, padded.height], [2, 4]);
    assert_eq!(padded.pixels[..raw.pixels.len()], raw.pixels);
    assert_eq!(
        padded.pixels[raw.pixels.len()..],
        [0, 0, 0, 255, 0, 0, 0, 255]
    );
}

#[test]
fn the_world_normal_loads_as_the_game_decodes_a_texture() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let shipped = bitmap(
        2,
        &[&[1, 2, 3, 4, 5, 6, 0, 0], &[7, 8, 9, 10, 11, 12, 0, 0]],
        40,
    );
    let expected = bmp::decode_rgb(&shipped).unwrap();
    let read = |bytes: Vec<u8>| {
        move |path: &[u8]| match path {
            b"map/world_normal.bmp" => Ok(bytes.clone()),
            _ => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    };
    assert_eq!(
        world_normal(&read(shipped), Decode::default()).unwrap(),
        expected
    );
    // A 32-bit form with alpha uniformly 255.
    let bitfields = bitmap(
        2,
        &[
            &[1, 2, 3, 255, 4, 5, 6, 255],
            &[7, 8, 9, 255, 10, 11, 12, 255],
        ],
        124,
    );
    assert_eq!(
        world_normal(&read(bitfields), Decode::default()).unwrap(),
        expected
    );
    let missing = world_normal::<Vec<u8>>(
        &|_: &[u8]| Err(io::Error::from(io::ErrorKind::NotFound)),
        Decode::default(),
    );
    assert!(missing.unwrap_err().to_string().contains("no layer holds"));
    let undecodable = world_normal(&read(b"not an image".to_vec()), Decode::default());
    assert!(
        undecodable
            .unwrap_err()
            .to_string()
            .contains("does not load")
    );
}
