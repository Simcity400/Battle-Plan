use crate::*;
use std::fs;
use std::path::Path;

fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn rgba_dds(pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0; 132];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [(4, 124u32), (12, 1), (16, 1), (28, 1), (76, 32),
        (80, 0x41), (88, 32), (92, 255), (96, 65280), (100, 16711680),
        (104, 4278190080), (108, 0x1000)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[128..].copy_from_slice(&pixel);
    bytes
}

fn rgb_bmp(pixel: [u8; 3]) -> Vec<u8> {
    let mut bytes = vec![0; 58];
    bytes[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, 58u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    bytes[54..57].copy_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    bytes
}

#[test]
fn modvfs_image_decoders_and_province_readers_use_admitted_mod_bytes() {
    let root = crate::test_support::temp_test_dir("modvfs_image_consumers");
    let vanilla = root.join("vanilla");
    let mod_root = root.join("mod");
    for (dir, value) in [(&vanilla, 1u8), (&mod_root, 7u8)] {
        write(dir, "map/terrain/atlas0.dds", rgba_dds([value, 20, 30, 255]));
        let mut info = rgba_dds([value, 20, 30, 255]);
        if value == 7 {
            info[16..20].copy_from_slice(&2u32.to_le_bytes());
            info.extend_from_slice(&[value, 20, 30, 255]);
        }
        write(dir, "map/terrain/info.dds", info);
        write(dir, "map/provinces.bmp", rgb_bmp([value, 20, 30]));
        write(dir, "map/definition.csv", format!("0;0;0;0;land;false;plains;0\n1;{value};20;30;land;false;plains;0\n"));
        write(dir, "map/heightmap.bmp", b"");
        crate::test_support::write_indexed_bmp(&dir.join("map/heightmap.bmp"), 1, 1, value);
    }
    let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).unwrap()
        .bind_all_test_content(&"d4".repeat(32)).unwrap();
    let dds = resolver.resolve_required("map/terrain/atlas0.dds", "atlas").unwrap();
    let bytes = resolver.read_resolved_bytes(&dds).unwrap();
    assert_eq!(decode_dds_rgba(&bytes, &dds).unwrap().pixels, [7, 20, 30, 255]);
    assert_eq!(decode_dds_rgba_mips(&bytes, &dds).unwrap()[0].pixels, [7, 20, 30, 255]);
    assert_eq!(decode_dds_rgbx(&bytes, &dds).unwrap().pixels, [0x0007141e]);
    assert_eq!(dds_info_bytes(&bytes, &dds).unwrap().mip_count, 1);
    assert_eq!(dds_dimensions_bytes(&bytes, &dds).unwrap().width, 1);
    let info = resolver.resolve_required("map/terrain/info.dds", "metadata").unwrap();
    let info_bytes = resolver.read_resolved_bytes(&info).unwrap();
    assert_eq!(dds_info_bytes(&info_bytes, &info).unwrap().dimensions.width, 2);
    assert_eq!(dds_dimensions_bytes(&info_bytes, &info).unwrap().width, 2);
    let indexed = resolver.resolve_required("map/heightmap.bmp", "heightmap").unwrap();
    assert_eq!(decode_bmp_indexed(&resolver.read_resolved_bytes(&indexed).unwrap(), &indexed).unwrap().pixels, [7]);
    let provinces = resolver.resolve_required("map/provinces.bmp", "provinces").unwrap();
    let province_bytes = resolver.read_resolved_bytes(&provinces).unwrap();
    assert_eq!(decode_bmp_rgbx(&province_bytes, &provinces).unwrap().pixels, [0x0007141e]);
    let definition = resolver.resolve_required("map/definition.csv", "definitions").unwrap();
    let (definitions, colors) = load_definition_csv(&resolver, &definition).unwrap();
    assert_eq!(definitions[&1].rgb, [7, 20, 30]);
    assert_eq!(decode_provinces_bmp(&province_bytes, &provinces, &colors).unwrap().ids, [1]);
    let anchors = province_anchors_from_resolver(&resolver).unwrap();
    assert_eq!(anchors.anchor(1).unwrap().pixel_count, 1);
    assert!(crate::province_positions::engine_bitmap_neighbours(&resolver).unwrap().is_empty());
    assert!(resolver.read_resolved_bytes(&vanilla.join("map/provinces.bmp")).is_err());
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_water_shader_validation_uses_the_admitted_mod_source() {
    let root = crate::test_support::temp_test_dir("modvfs_water_source");
    fs::create_dir_all(&root).unwrap();
    let manifest = crate::test_support::write_valid_water_map_frame_bundle(&root, |frame| frame);
    let frame: MapFrameRuntimeBundle = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let vanilla = root.join("vanilla");
    let mods = root.join("mod");
    write(&vanilla, PDXWATER_SHADER_SOURCE, b"vanilla water source");
    write(&mods, PDXWATER_SHADER_SOURCE, b"mod water source");
    let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mods]).unwrap()
        .bind_all_test_content(&"f6".repeat(32)).unwrap();
    assert_eq!(crate::frame_bundle::map_frame_water_shader_source(&resolver, &frame, "water fixture").unwrap(), "mod water source");
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_text_parsers_and_export_hashes_use_admitted_mod_bytes() {
    let root = crate::test_support::temp_test_dir("modvfs_text_consumers");
    let vanilla = root.join("vanilla");
    let mod_root = root.join("mod");
    let fixtures = [
        ("common/countries/test.txt", "color = { 1 2 3 }", "color = { 7 8 9 }"),
        ("common/countries/colors.txt", "AAA = { color = { 1 2 3 } }", "AAA = { color = { 7 8 9 } }"),
        ("common/countries/cosmetic.txt", "AAA_TEST = { color = { 1 2 3 } }", "AAA_TEST = { color = { 7 8 9 } }"),
        ("common/country_tags/test.txt", "AAA = \"countries/base.txt\"", "AAA = \"countries/test.txt\""),
        ("map/default.map", "tree = { 1 }", "tree = { 7 }"),
        ("map/cities.txt", "pixel_step_x = 1", "pixel_step_x = 7"),
        ("map/seasons.txt", "winter = { start_date = 1.1 }", "winter = { start_date = 7.1 }"),
        ("gfx/FX/test.fxh", "name = \"vanilla\"", "name = \"mod winner\""),
        ("history/countries/AAA - Test.txt", "set_cosmetic_tag = AAA_BASE", "set_cosmetic_tag = AAA_TEST"),
    ];
    for (path, base, changed) in fixtures { write(&vanilla, path, base); write(&mod_root, path, changed); }
    let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).unwrap()
        .bind_all_test_content(&"e5".repeat(32)).unwrap();
    let resolve = |path| resolver.resolve_required(path, path).unwrap();
    assert_eq!(load_country_file_color(&resolver, &resolve(fixtures[0].0)).unwrap(), Some([7, 8, 9]));
    assert_eq!(load_colors_txt_overrides(&resolver, &resolve(fixtures[1].0)).unwrap()["AAA"], [7, 8, 9]);
    assert_eq!(load_cosmetic_colors(&resolver, &resolve(fixtures[2].0)).unwrap()["AAA_TEST"], [7, 8, 9]);
    let tags = resolver.resolve_dir_files_recursive("common/country_tags", Some("txt")).unwrap();
    assert_eq!(load_country_tag_files(&resolver, &tags).unwrap()["AAA"], "countries/test.txt");
    assert_eq!(load_default_map_meta(&resolver, &resolve(fixtures[4].0)).unwrap().tree_indices, [7]);
    assert_eq!(load_city_source_meta(&resolver, &resolve(fixtures[5].0)).unwrap().pixel_step_x, Some(7));
    assert_eq!(load_season_source_meta(&resolver, &resolve(fixtures[6].0)).unwrap().blocks[0].start_date.as_deref(), Some("7.1"));
    let shader = resolve(fixtures[7].0);
    assert_eq!(load_text_source_catalog(&resolver, &shader).unwrap().quoted_values["name"], ["mod winner"]);
    let history = resolver.resolve_dir_files_recursive("history/countries", Some("txt")).unwrap();
    assert_eq!(cosmetic_tags_1936_from_history_files(&resolver, &history).unwrap()["AAA"], "AAA_TEST");
    let expected = fnv1a64_bytes(fixtures[7].2.as_bytes());
    assert_eq!(visual_input_hashes(&resolver, &[("shader", &shader)]).unwrap()["shader"], expected);
    let out = root.join("output");
    fs::create_dir_all(&out).unwrap();
    let mut copied = std::collections::BTreeMap::new();
    copy_shader_source(&resolver, &out, fixtures[7].0, "shader.fxh", &shader, &mut copied).unwrap();
    copy_bundle_source(&resolver, &out, fixtures[7].0, "source.fxh", &shader, &mut copied).unwrap();
    assert_eq!(fs::read(out.join("shader.fxh")).unwrap(), fixtures[7].2.as_bytes());
    assert_eq!(fs::read(out.join("source.fxh")).unwrap(), fixtures[7].2.as_bytes());
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_admitted_reads_keep_mod_winners_and_refuse_changed_snapshots() {
    let root = crate::test_support::temp_test_dir("modvfs_bytes");
    let vanilla = root.join("vanilla");
    let mod_root = root.join("mod");
    let paths = ["gfx/posteffect_volumes.txt", "gfx/FX/test.shader", "gfx/FX/test.fxh",
        "gfx/models/test.mesh", "gfx/models/test.anim", "gfx/fonts/test.fnt",
        "gfx/map/terrain/colorcorrection.tga", "map/seasons.txt", "map/buildings.txt",
        "map/terrain/underwater_terrain_0.dds", "interface/test.gui", "interface/test.gfx",
        "common/map_modes/test.txt", "common/scripted_guis/test.txt", "events/test.txt"];
    for path in paths { write(&vanilla, path, b"vanilla"); write(&mod_root, path, b"mod winner"); }
    let raw = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).unwrap();
    for path in paths { assert_eq!(raw.read_required_bytes(path, path).unwrap(), b"mod winner", "{path}"); }
    let admitted = raw.bind_all_test_content(&"a1".repeat(32)).unwrap();
    for path in paths {
        assert_eq!(admitted.read_required_bytes(path, path).unwrap(), b"mod winner", "{path}");
        assert_eq!(admitted.read_required_text(path, path).unwrap(), "mod winner", "{path}");
        let key = admitted.admitted_used_content_key(path).unwrap().unwrap();
        assert_eq!(admitted.content_admission_lease().unwrap().read_used_content(&key).unwrap(), b"mod winner");
    }
    assert!(admitted.read_required_bytes("gfx/unrecorded.dds", "unrecorded").is_err());
    let path = paths[0];
    let key = admitted.admitted_used_content_key(path).unwrap().unwrap();
    let snapshot = admitted.resolve_required(path, path).unwrap();
    let mut permissions = fs::metadata(&snapshot).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&snapshot, permissions).unwrap();
    fs::write(snapshot, b"bad winner").unwrap();
    assert!(admitted.read_required_bytes(path, path).is_err());
    assert!(admitted.content_admission_lease().unwrap().read_used_content(&key).is_err());
    drop(admitted);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_defines_execute_mod_scripts_after_vanilla_and_use_path_override() {
    let root = crate::test_support::temp_test_dir("modvfs_defines");
    let vanilla = root.join("vanilla");
    let mod_root = root.join("mod");
    write(&vanilla, "common/defines/00_defines.lua", b"NDefines = { NGraphics = {}, NMilitary = {}, NSupply = {} }");
    write(&vanilla, "common/defines/zz_graphics.lua", b"NDefines.NGraphics.AIRBASE_ICON_DISTANCE_CUTOFF = 10\nNDefines.NGraphics.NAVALBASE_ICON_DISTANCE_CUTOFF = 20\nNDefines.NGraphics.DRAW_COUNTRY_NAMES_CUTOFF = 30");
    write(&vanilla, "common/defines/override.lua", b"error('overridden vanilla script executed')");
    write(&mod_root, "common/defines/override.lua", b"NDefines.NSupply.MAX_RAILWAY_LEVEL = 7");
    write(&mod_root, "common/defines/01_mod.lua", b"NDefines.NGraphics.AIRBASE_ICON_DISTANCE_CUTOFF = NDefines.NGraphics.AIRBASE_ICON_DISTANCE_CUTOFF * 3\nNDefines.NGraphics.NAVALBASE_ICON_DISTANCE_CUTOFF = 40\nNDefines.NGraphics.DRAW_COUNTRY_NAMES_CUTOFF = 60\nNDefines.NMilitary.RIVER_SMALL_START_INDEX = 1\nNDefines.NMilitary.RIVER_SMALL_STOP_INDEX = 3\nNDefines.NMilitary.RIVER_LARGE_STOP_INDEX = 8\nNDefines.NGraphics.RIVER_FADE_FROM = 70\nNDefines.NGraphics.RIVER_FADE_TO = 20");
    let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).unwrap()
        .bind_all_test_content(&"b2".repeat(32)).unwrap();
    let icons = base_icon_defines_from_resolver(&resolver).unwrap();
    assert_eq!((icons.airbase_cutoff, icons.navalbase_cutoff), (30.0, 40.0));
    assert_eq!(draw_country_names_cutoff_from_resolver(&resolver).unwrap(), 60.0);
    assert_eq!(rail_max_level_from_resolver(&resolver).unwrap(), 7);
    let palette = RiverPaletteDefines::from_resolver(&resolver).unwrap();
    assert_eq!((palette.small_start_index, palette.small_stop_index, palette.large_stop_index), (1, 3, 8));
    let fade = RiverFadeDefines::from_resolver(&resolver).unwrap();
    assert_eq!((fade.fade_from, fade.fade_to), (70.0, 20.0));
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_directory_scan_excludes_unrecorded_local_winners() {
    let root = crate::test_support::temp_test_dir("modvfs_directory");
    write(&root, "interface/first.gui", b"first");
    write(&root, "interface/second.gui", b"second");
    let (resolver, _) = GameFileResolver::vanilla(&root).unwrap()
        .bind_exact_test_content("interface/first.gui", &"c3".repeat(32)).unwrap();
    assert!(resolver.resolve_dir_files("interface", Some("gui")).unwrap_err()
        .to_string().contains("content_namespace_unwitnessed"));
    assert_eq!(resolver.read_required_bytes("interface/first.gui", "recorded GUI").unwrap(), b"first");
    assert!(resolver.read_required_bytes("interface/second.gui", "unrecorded GUI").is_err());
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modvfs_graphics_consumers_observe_evaluated_mod_scalars_and_arrays() {
    let root = crate::test_support::temp_test_dir("modvfs_graphics_consumers");
    let vanilla = root.join("vanilla");
    let mod_root = root.join("mod");
    let fields = [
        "TONE_MAP_MIDDLE_GREY", "TONE_MAP_LUMINANCE_WHITE", "MIN_HDR_ADJUSTMENT",
        "MAX_HDR_ADJUSTMENT", "HDR_ADJUSTMENT_SPEED", "BLOOM_WIDTH", "BLOOM_SCALE",
        "BRIGHT_THRESHOLD", "EMISSIVE_BLOOM_STRENGTH", "POSTEFFECT_PER_PROVINCE_MIN_SNOW",
        "POSTEFFECT_PER_PROVINCE_MAX_SNOW", "POSTEFFECT_TOTAL_MIN_SNOW", "POSTEFFECT_TOTAL_MAX_SNOW",
        "POSTEFFECT_FEATHER_MIN_DISTANCE", "POSTEFFECT_FEATHER_MAX_DISTANCE", "POSTEFFECT_FEATHER_AT_MIN",
        "POSTEFFECT_FEATHER_AT_MAX", "RAILWAY_Y_OFFSET", "RAILWAY_CAMERA_CUTOFF",
        "RAILWAY_CAMERA_CUTOFF_SPEED", "RAILWAY_ICON_CUTOFF", "SUPPLY_ICON_CUTOFF",
        "SUPPLY_ICON_UNUSED_CUTOFF", "SUPPLY_ICON_DISCONNECTED_CUTOFF", "SUPPLY_ICON_OK_CUTOFF",
        "SUPPLY_ICON_NUMBERS_CUTOFF", "SUPPLY_ICON_END_CUTOFF", "RAILWAY_BRIDGE_WIDTH",
        "RAILWAY_BRIDGE_LARGE_WIDTH", "RAILWAY_BRIDGE_GAP_WIDTH", "RAILWAY_BRIDGE_GAP_LARGE_WIDTH",
        "RAILWAY_MAP_ARROW_THIN_LEVEL_THRESHOLD", "RAILWAY_MAP_ARROW_MEDIUM_LEVEL_THRESHOLD",
        "RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD", "NAVAL_REGION_TRANSPARENCY_RATIO_MIN",
        "NAVAL_REGION_TRANSPARENCY_RATIO_MAX", "NAVAL_REGION_TRANSPARENCY_RANGE",
        "STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP", "MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY",
        "PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY", "FACTIONS_MEMBER_TRANSPARENCY",
        "TREE_FADE_NEAR", "TREE_FADE_FAR", "PROVINCE_BORDER_FADE_NEAR", "PROVINCE_BORDER_FADE_FAR",
        "STATE_BORDER_FADE_NEAR", "STATE_BORDER_FADE_FAR", "WEATHER_DISTANCE_CUTOFF",
        "WEATHER_DISTANCE_FADE_LENGTH", "WEATHER_ZOOM_IN_CUTOFF", "WEATHER_ZOOM_IN_FADE_LENGTH",
        "WEATHER_ZOOM_IN_FADE_FACTOR", "WEATHER_PLAYBACK_RATE", "WEATHER_PLAYBACK_RATE_CUTOFF",
        "WEATHER_PLAYBACK_RATE_LENGTH",
    ];
    let mut base = String::from("NDefines = { NGraphics = {}, NMapMode = {} }\n");
    let mut patch = String::new();
    for key in fields {
        base.push_str(&format!("NDefines.NGraphics.{key} = 1\n"));
        patch.push_str(&format!("NDefines.NGraphics.{key} = NDefines.NGraphics.{key} + 2\n"));
    }
    for key in ["POSTEFFECT_FEATHER_MAX_DISTANCE", "TREE_FADE_FAR", "PROVINCE_BORDER_FADE_FAR", "STATE_BORDER_FADE_FAR"] {
        patch.push_str(&format!("NDefines.NGraphics.{key} = 9\n"));
    }
    for key in ["MAP_MODE_INFRA_RANGE_COLOR_FROM", "MAP_MODE_INFRA_RANGE_COLOR_TO",
        "STRATEGIC_MODE_ENEMY_STRIPES_COLOR", "STRATEGIC_MODE_OUR_STRIPES_COLOR", "STRATEGIC_MODE_ALLY_STRIPES_COLOR",
        "STRATEGIC_AIR_COLOR_BAD", "STRATEGIC_AIR_COLOR_GOOD", "STRATEGIC_AIR_COLOR_AVERAGE",
        "STRATEGIC_AIR_COLOR_NEUTRAL", "FACTIONS_COLOR_NOT_MEMBER", "OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR"] {
        base.push_str(&format!("NDefines.NGraphics.{key} = {{0, 0, 0, 0}}\n"));
        patch.push_str(&format!("NDefines.NGraphics.{key} = {{0.25, 0.5, 0.75, 1}}\n"));
    }
    base.push_str("NDefines.NGraphics.SUPPLY_ICON_SHIFT = {0, 0, 0}\n");
    patch.push_str("NDefines.NGraphics.SUPPLY_ICON_SHIFT = {2, 4, 6}\n");
    for key in ["OCCUPATION_RESISTANCE_MAP_MODE_COLORS", "OCCUPATION_COMPLIANCE_MAP_MODE_COLORS"] {
        base.push_str(&format!("NDefines.NGraphics.{key} = {{0,0,0,0,0,100,0,0,0,0}}\n"));
        patch.push_str(&format!("NDefines.NGraphics.{key} = {{0,0.25,0.5,0.75,1,100,0.5,0.5,0.5,1}}\n"));
    }
    for key in fields.into_iter().chain(["MAP_MODE_INFRA_RANGE_COLOR_FROM", "MAP_MODE_INFRA_RANGE_COLOR_TO", "STRATEGIC_MODE_ENEMY_STRIPES_COLOR", "STRATEGIC_MODE_OUR_STRIPES_COLOR", "STRATEGIC_MODE_ALLY_STRIPES_COLOR", "FACTIONS_COLOR_NOT_MEMBER", "OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR", "OCCUPATION_RESISTANCE_MAP_MODE_COLORS", "OCCUPATION_COMPLIANCE_MAP_MODE_COLORS"]) {
        if ["FABRICATE_CLAIM_SELECTED_SECONDARY_COLOR", "FABRICATE_CLAIM_TARGET_COUNTRY_SECONDARY_COLOR", "FABRICATE_CLAIM_NON_TARGET_COUNTRY_SECONDARY_COLOR", "FABRICATE_CLAIM_ALREADY_CLAIM_SECONDARY_COLOR", "FABRICATE_CLAIM_ALREADY_CORE_SECONDARY_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_DEFAULT_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_MAX_LEVEL_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_LEVEL_LOW_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_LEVEL_HI_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_MAX_AMOUNT_QUEUED_COLOR", "CONSTRUCTION_MAP_MODE_BUILDING_QUEUED_COLOR", "MAP_MODE_MANPOWER_RANGE_MAX", "MAP_MODE_MANPOWER_RANGE_COLOR_FROM", "MAP_MODE_MANPOWER_RANGE_COLOR_TO", "MAP_MODE_INFRA_RANGE_COLOR_FROM", "MAP_MODE_INFRA_RANGE_COLOR_TO", "MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY", "CONSTRUCTION_MAP_MODE_TRANSPARENCY_OVERRIDE", "PEACE_CONFERENCE_CURRENT_SELECTED_SECONDARY_COLOR", "PEACE_CONFERENCE_SELECTABLE_SECONDARY_COLOR", "PEACE_CONFERENCE_CONTESTED_SECONDARY_COLOR", "PEACE_CONFERENCE_CHANGE_TARGET_TAG_SECONDARY_COLOR", "PEACE_CONFERENCE_DIFFERENT_STACKABLE_SECONDARY_COLOR", "FACTIONS_COLOR_NOT_MEMBER", "FACTIONS_MEMBER_TRANSPARENCY", "PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY", "SELECTED_COUNTRY_HIGHLIGHT_THICKNESS_MULT", "STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP", "STRATEGIC_MODE_ENEMY_STRIPES_COLOR", "STRATEGIC_MODE_OUR_STRIPES_COLOR", "STRATEGIC_MODE_ALLY_STRIPES_COLOR", "RADAR_RANGE_STRIPES_COLOR", "RADAR_RANGE_COLOR", "RADAR_RANGE_ALLIED_COLOR", "RADAR_RANGE_SELECTED_COLOR", "RADAR_ROTATION_SPEED", "AIR_RANGE_CAN_ASSIGN_MISSION_STRIPES_COLOR", "AIR_RANGE_CANNOT_ASSIGN_MISSION_STRIPES_COLOR", "AIR_RANGE_INDICATOR_DEFAULT_COLOR", "AIR_RANGE_INDICATOR_NO_WINGS_COLOR", "AIR_RANGE_INDICATOR_ROTATION_SPEED", "AIR_MISSION_ARROW_ACTIVE_COLOR", "AIR_MISSION_ARROW_NONACTIVE_COLOR", "AIR_MISSION_ARROW_SELECTED_COLOR", "AIR_TRANSFER_ARROW_COLOR", "NAVAL_REGION_ACCESS_AVOID_COLOR", "NAVAL_REGION_ACCESS_BLOCK_COLOR", "NAVAL_REGION_FADE_WHEN_FLEET_SELECTED", "NAVAL_REGION_TRANSPARENCY_RATIO_MAX", "NAVAL_REGION_TRANSPARENCY_RATIO_MIN", "NAVAL_REGION_TRANSPARENCY_RANGE", "NAVAL_DOMINANCE_TREND_BAR_HIDE_RATIO", "AIR_REGION_FADE_WHEN_WING_SELECTED", "UI_CONFIGURABLE_SLOT_FROM", "UI_CONFIGURABLE_SLOT_TO", "MAP_MODE_TERRAIN_TRANSPARENCY", "MAP_MODE_NAVAL_TERRAIN_TRANSPARENCY", "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_LOW", "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_HIGH", "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_LOW", "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_HIGH", "MAP_MODE_INTEL_MAX_HORIZONTAL_STACK", "RAILWAY_GUN_RANGE_INDICATOR_DEFAULT_COLOR", "RAILWAY_GUN_RANGE_INDICATOR_ROTATION_SPEED", "RAILWAY_GUN_RANGE_STRIPES_COLOR", "DEPLOYED_GENERAL_UNIT_LINE_COLOR", "DEPLOYED_GENERAL_UNIT_LINE_COLOR_ABILITY_ACTIVE", "SHOW_DEPLOYED_GENERAL_COMMUNICATION_LINES", "SHOW_DEPLOYED_GENERAL_NO_COMMUNICATION_LINES", "SHOW_DEPLOYED_GENERAL_FRONT_LINES", "DEPLOYED_GENERAL_FRONT_LINE_MAX_CAMERA_HEIGHT", "SHOW_DEPLOYED_GENERAL_BASE_PLATES", "DEPLOYED_GENERAL_BASE_PLATE_RADIUS", "DEPLOYED_GENERAL_BASE_PLATE_COLOR", "PREPARING_RAID_ARROW_COLOR", "READY_RAID_ARROW_COLOR", "ACTIVE_RAID_ARROW_COLOR", "OCCUPATION_MAP_MODE_COUNTRY_STRIPE_ALPHA", "OPERATIVE_MAP_MODE_INVALID_COUNTRY_TARGET_TRANSPARENCY", "COASTAL_FACILITY_OFFSET", "NAVAL_HEADQUARTER_OFFSET", "SUPPLY_MAP_MODE_COUNTRY_BORDER_CAMERA_DISTANCE", "SUPPLY_MAP_MODE_COUNTRY_BORDER_OUTLINE_CUTOFF", "GRADIENT_BORDERS_THICKNESS_SUPPLY_COUNTRY_BORDER", "SUPPLY_COUNTRY_BORDER_PLAYER_COLOR", "SUPPLY_COUNTRY_BORDER_FRIEND_COLOR", "SUPPLY_COUNTRY_BORDER_ACCESS_COLOR", "SUPPLY_MAP_MODE_REACH_COLOR", "SUPPLY_MAP_MODE_BEST_FLOW_DISPLAY", "SUPPLY_MAP_MODE_STATUS_COLOR", "SUPPLY_STATUS_DISPLAY_THRESHOLD", "SUPPLY_HOVERED_STATE_COLOR_INDEX", "SUPPLY_HOVERED_PROVINCE_COLOR_INDEX", "PEACE_HOVERED_STATE_COLOR_INDEX", "PEACE_CLAIMED_STATE_COLOR_INDEX", "SELECTION_HOVERED_STATE_COLOR_INDEX_CONTROLLED", "SELECTION_HOVERED_STATE_COLOR_INDEX_FOREIGN", "CONSTRUCTION_PRIMARY_VALID_BUILD_TARGET_PROVINCE_COLOR_INDEX", "CONSTRUCTION_PRIMARY_INVALID_BUILD_TARGET_PROVINCE_COLOR_INDEX", "CONSTRUCTION_PRIMARY_FOREIGN_BUILD_TARGET_PROVINCE_COLOR_INDEX", "CONSTRUCTION_SECONDARY_VALID_BUILD_TARGET_PROVINCE_COLOR_INDEX", "CONSTRUCTION_SECONDARY_INVALID_BUILD_TARGET_PROVINCE_COLOR_INDEX", "CONSTRUCTION_SECONDARY_FOREIGN_BUILD_TARGET_PROVINCE_COLOR_INDEX", "FACTION_THEATER_COLOR_INDEX", "FACTION_THEATER_HIGHLIGHT_COLOR_INDEX"].contains(&key) {
            base = base.replace(&format!("NDefines.NGraphics.{key}"), &format!("NDefines.NMapMode.{key}"));
            patch = patch.replace(&format!("NDefines.NGraphics.{key}"), &format!("NDefines.NMapMode.{key}"));
        }
    }
    write(&vanilla, "common/defines/zz_base.lua", base);
    write(&mod_root, "common/defines/01_mod.lua", patch);
    let resolver = GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).unwrap()
        .bind_all_test_content(&"d4".repeat(32)).unwrap();
    assert_eq!(crate::grading_law::load_graphics_grading_defines(&resolver).unwrap().bloom_scale, 3.0);
    assert_eq!(rail_geometry_defines_from_resolver(&resolver).unwrap().supply_icon_shift, [2.0,4.0,6.0]);
    assert_eq!(infra_gradient_colors_from_resolver(&resolver).unwrap().from, [0.25,0.5,0.75,1.0]);
    assert_eq!(naval_tint_defines_from_resolver(&resolver).unwrap().transparency_range, 3.0);
    assert_eq!(strategic_air_colors_from_resolver(&resolver).unwrap().good, [0.25,0.5,0.75,1.0]);
    assert_eq!(admin_mapmode_defines_from_resolver(&resolver).unwrap().ideology_color_transparency, 3.0);
    assert_eq!(factions_defines_from_resolver(&resolver).unwrap().member_transparency, 3.0);
    assert_eq!(tree_fade_window_from_resolver(&resolver).unwrap().far_f32_bits, 9.0f32.to_bits());
    assert_eq!(border_fade_windows_from_resolver(&resolver).unwrap().province.far, 9.0);
    assert_eq!(crate::weather_law::weather_fade_defines_from_resolver(&resolver).unwrap().0.playback_rate, 3.0);
    assert_eq!(occupation_palette_from_resolver(&resolver).unwrap().resistance[0].rgba, [0.25,0.5,0.75,1.0]);
    drop(resolver);
    fs::remove_dir_all(root).unwrap();
}
