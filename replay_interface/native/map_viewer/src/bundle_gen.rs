use crate::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn build_bundle(options: &BundleOptions) -> Result<()> {
    let game = require_dir(&options.game, "game directory")?;
    let resolver_setup = BundleResolverSetup::from_options(game.clone(), &options.mod_selection)?;
    let resolver = &resolver_setup.resolver;
    let out = &options.out;
    fs::create_dir_all(out)?;
    if options.force {
        remove_known_bundle_files(out)?;
    }

    let definition_csv = resolver.resolve_file("map/definition.csv", "definition.csv")?;
    let default_map = resolver.resolve_file("map/default.map", "default.map")?;
    let provinces_bmp = resolver.resolve_file("map/provinces.bmp", "provinces.bmp")?;
    let colormap_dds = resolver.resolve_file(
        "map/terrain/colormap_rgb_cityemissivemask_a.dds",
        "terrain colormap DDS",
    )?;
    let water_dds =
        resolver.resolve_file("map/terrain/colormap_water_0.dds", "water colormap DDS")?;
    let atlas_dds = resolver.resolve_file("map/terrain/atlas0.dds", "terrain atlas DDS")?;
    let atlas_normal_dds =
        resolver.resolve_file("map/terrain/atlas_normal0.dds", "terrain normal atlas DDS")?;
    let underwater_dds = resolver.resolve_file(
        "map/terrain/underwater_terrain_0.dds",
        "underwater terrain DDS",
    )?;
    let waterspec_dds =
        resolver.resolve_file("map/terrain/fow_rgb_waterspec_a.dds", "water spec/fow DDS")?;
    let lean1_dds = resolver.resolve_file("map/terrain/lean1.dds", "water LEAN 1 DDS")?;
    let lean2_dds = resolver.resolve_file("map/terrain/lean2.dds", "water LEAN 2 DDS")?;
    let reflection_dds =
        resolver.resolve_file("map/terrain/reflection.dds", "water reflection cubemap DDS")?;
    let river_diffuse_dds = resolver.resolve_file(
        "map/terrain/RiverSurface_diffuse_0.dds",
        "river diffuse DDS",
    )?;
    let river_normal_dds =
        resolver.resolve_file("map/terrain/RiverSurface_normal_0.dds", "river normal DDS")?;
    let river_masks_dds =
        resolver.resolve_file("map/terrain/RiverSurface_masks.dds", "river masks DDS")?;
    let border_country_dds =
        resolver.resolve_file("map/terrain/border_country_0.dds", "country border DDS")?;
    let border_province_dds =
        resolver.resolve_file("map/terrain/border_province_0.dds", "province border DDS")?;
    let border_impassable_dds = resolver.resolve_file(
        "map/terrain/border_impassable_0.dds",
        "impassable border DDS",
    )?;
    let border_state_dds =
        resolver.resolve_file("map/terrain/border_state_0.dds", "state border DDS")?;
    let border_sea_dds = resolver.resolve_file("map/terrain/border_sea_0.dds", "sea border DDS")?;
    let border_sea_region_dds = resolver.resolve_file(
        "map/terrain/border_sea_region_0.dds",
        "sea-region border DDS",
    )?;
    let citylights_snowmask_dds = resolver.resolve_file(
        "map/terrain/citylights_rgb_snowmask_a_0.dds",
        "city lights/snow mask DDS",
    )?;
    let snow_normal_dds = resolver.resolve_file(
        "map/terrain/snow_normal_rgb_diffuse_a.dds",
        "snow normal/diffuse DDS",
    )?;
    let mud_diffuse_dds = resolver.resolve_file(
        "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
        "mud diffuse/gloss DDS",
    )?;
    let mud_normal_dds = resolver.resolve_file(
        "map/terrain/mud_normal_rgb_spec_a_0.dds",
        "mud normal/spec DDS",
    )?;
    let ice_diffuse_dds =
        resolver.resolve_file("map/terrain/ice_diffuse.dds", "ice diffuse DDS")?;
    let ice_noise_dds = resolver.resolve_file("map/terrain/ice_noise_0.dds", "ice noise DDS")?;
    let material_rgba_specs = material_rgba_specs(resolver)?;
    let raw_material_specs = vec![RawMaterialSpec {
        source: "map/terrain/reflection.dds".to_string(),
        output: "reflection.dds".to_string(),
        path: reflection_dds.clone(),
        role: "water_reflection_cubemap".to_string(),
        shader_samplers: vec!["ReflectionCubeMap".to_string()],
    }];
    let terrain_bmp = resolver.resolve_file("map/terrain.bmp", "terrain.bmp")?;
    let heightmap_bmp = resolver.resolve_file("map/heightmap.bmp", "heightmap.bmp")?;
    let world_normal_bmp = resolver.resolve_file("map/world_normal.bmp", "world_normal.bmp")?;
    let rivers_bmp = resolver.resolve_file("map/rivers.bmp", "rivers.bmp")?;
    let cities_bmp = resolver.resolve_file("map/cities.bmp", "cities.bmp")?;
    let cities_txt = resolver.resolve_file("map/cities.txt", "cities.txt")?;
    let trees_bmp = resolver.resolve_file("map/trees.bmp", "trees.bmp")?;
    let seasons_txt = resolver.resolve_file("map/seasons.txt", "seasons.txt")?;
    let positions_txt = resolver.resolve_file("map/positions.txt", "positions.txt")?;
    let ambient_object_txt =
        resolver.resolve_file("map/ambient_object.txt", "ambient_object.txt")?;
    let buildings_txt = resolver.resolve_file("map/buildings.txt", "buildings.txt")?;
    let railways_txt = resolver.resolve_file("map/railways.txt", "railways.txt")?;
    let supply_nodes_txt = resolver.resolve_file("map/supply_nodes.txt", "supply_nodes.txt")?;
    let unitstacks_txt = resolver.resolve_file("map/unitstacks.txt", "unitstacks.txt")?;
    let weatherpositions_txt =
        resolver.resolve_file("map/weatherpositions.txt", "weatherpositions.txt")?;
    let tree_tint_bmp = resolver.resolve_file("map/terrain/Tree_tint.bmp", "tree tint BMP")?;
    let tree_season_bmp =
        resolver.resolve_file("map/terrain/Tree_season.bmp", "tree season BMP")?;
    let graphics_defines_lua =
        resolver.resolve_file("common/defines/00_graphics.lua", "graphics defines")?;
    let constants_fxh = resolver.resolve_file("gfx/FX/constants.fxh", "shader constants")?;
    let entity_source_paths = vec![
        (
            "gfx/entities/trees.gfx".to_string(),
            resolver.resolve_file("gfx/entities/trees.gfx", "tree entity GFX")?,
        ),
        (
            "gfx/entities/buildings.gfx".to_string(),
            resolver.resolve_file("gfx/entities/buildings.gfx", "building entity GFX")?,
        ),
        (
            "gfx/entities/buildings.asset".to_string(),
            resolver.resolve_file("gfx/entities/buildings.asset", "building entity asset")?,
        ),
        (
            "gfx/entities/mapitems.gfx".to_string(),
            resolver.resolve_file("gfx/entities/mapitems.gfx", "map item entity GFX")?,
        ),
        (
            "gfx/entities/mapitems.asset".to_string(),
            resolver.resolve_file("gfx/entities/mapitems.asset", "map item entity asset")?,
        ),
        (
            "gfx/entities/landmarks.asset".to_string(),
            resolver.resolve_file("gfx/entities/landmarks.asset", "landmark entity asset")?,
        ),
        (
            "common/defines/00_graphics.lua".to_string(),
            graphics_defines_lua.clone(),
        ),
        ("gfx/FX/constants.fxh".to_string(), constants_fxh.clone()),
        (
            "gfx/FX/standardfuncsgfx.fxh".to_string(),
            resolver.resolve_file("gfx/FX/standardfuncsgfx.fxh", "standard map helpers")?,
        ),
        (
            "gfx/FX/pdxmap.shader".to_string(),
            resolver.resolve_file("gfx/FX/pdxmap.shader", "map shader")?,
        ),
        (
            "gfx/FX/pdxmap.fxh".to_string(),
            resolver.resolve_file("gfx/FX/pdxmap.fxh", "map shader include")?,
        ),
        (
            "gfx/FX/shadow.fxh".to_string(),
            resolver.resolve_file("gfx/FX/shadow.fxh", "shadow shader include")?,
        ),
        (
            "gfx/FX/fow.fxh".to_string(),
            resolver.resolve_file("gfx/FX/fow.fxh", "fog of war shader include")?,
        ),
        (
            "gfx/FX/pdxwater.shader".to_string(),
            resolver.resolve_file("gfx/FX/pdxwater.shader", "water shader")?,
        ),
        (
            "gfx/FX/river.shader".to_string(),
            resolver.resolve_file("gfx/FX/river.shader", "river shader")?,
        ),
        (
            "gfx/FX/border.shader".to_string(),
            resolver.resolve_file("gfx/FX/border.shader", "border shader")?,
        ),
        (
            "gfx/FX/tree.shader".to_string(),
            resolver.resolve_file("gfx/FX/tree.shader", "tree shader")?,
        ),
        (
            "gfx/FX/maparrow.shader".to_string(),
            resolver.resolve_file("gfx/FX/maparrow.shader", "map arrow shader")?,
        ),
        (
            "gfx/FX/tiled_pointlights.fxh".to_string(),
            resolver.resolve_file("gfx/FX/tiled_pointlights.fxh", "tiled point lights include")?,
        ),
        (
            "gfx/FX/pdxmesh.shader".to_string(),
            resolver.resolve_file("gfx/FX/pdxmesh.shader", "mesh shader")?,
        ),
        (
            "gfx/models/supply/railroad.shader".to_string(),
            resolver.resolve_file("gfx/models/supply/railroad.shader", "railroad shader")?,
        ),
        (
            "gfx/models/buildings/supply_station.gfx".to_string(),
            resolver.resolve_file(
                "gfx/models/buildings/supply_station.gfx",
                "supply station model GFX",
            )?,
        ),
        (
            "gfx/models/buildings/bridge_railway.gfx".to_string(),
            resolver.resolve_file(
                "gfx/models/buildings/bridge_railway.gfx",
                "rail bridge model GFX",
            )?,
        ),
        (
            "gfx/models/buildings/bridge_railway_large.gfx".to_string(),
            resolver.resolve_file(
                "gfx/models/buildings/bridge_railway_large.gfx",
                "large rail bridge model GFX",
            )?,
        ),
    ];
    let common_building_paths = collect_source_files(resolver, "common/buildings", Some("txt"))?;
    let state_paths = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let strategic_paths = resolver.resolve_dir_files("map/strategicregions", Some("txt"))?;
    let terrain_defs = resolver.resolve_file(
        "common/terrain/00_terrain.txt",
        "common/terrain/00_terrain.txt",
    )?;

    println!("========================================================================");
    println!("Rust Battle Plan map bundle");
    println!("  game: {}", game.display());
    println!("  out : {}", out.display());
    if let Some(modset) = resolver_setup.manifest_modset() {
        println!("  mods: {} ({})", modset.mod_count, modset.selection_source);
        for warning in resolver_setup.warnings() {
            println!("  mod warning: {warning}");
        }
    }
    println!("========================================================================");

    let (definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    let (state_meta, province_to_state, state_owners) = load_states(&state_paths)?;
    let country_colors = load_country_colors(resolver)?;
    let (region_to_provinces, region_meta, province_to_region) =
        load_strategic_regions(&strategic_paths)?;
    let constants_source_text = read_latin1(&constants_fxh)?;
    let shader_constants = load_shader_constant_meta_from_text(&constants_source_text);
    let shader_vector3_constants =
        load_shader_vector3_constant_meta_from_text(&constants_source_text);
    let colormap = load_dds_rgbx(&colormap_dds)?;
    let water = load_dds_rgbx(&water_dds)?;
    let terrain_atlas = load_dds_rgbx(&atlas_dds)?;
    let terrain_atlas_mips = load_dds_rgba_mips(&atlas_dds)?;
    let terrain_normal_atlas = load_dds_rgbx(&atlas_normal_dds)?;
    let terrain_normal_atlas_mips = load_dds_rgba_mips(&atlas_normal_dds)?;
    let underwater = load_dds_rgbx(&underwater_dds)?;
    let waterspec = load_dds_rgbx(&waterspec_dds)?;
    let river_diffuse = load_dds_rgbx(&river_diffuse_dds)?;
    let border_country = load_dds_rgbx(&border_country_dds)?;
    let border_state = load_dds_rgbx(&border_state_dds)?;
    let border_sea = load_dds_rgbx(&border_sea_dds)?;
    let terrain_indices = load_bmp_indexed(&terrain_bmp)?;
    let heightmap = load_bmp_indexed(&heightmap_bmp)?;
    let height_normal = load_bmp_rgbx(&world_normal_bmp)?;
    let rivers = load_bmp_indexed(&rivers_bmp)?;
    let cities = load_bmp_indexed(&cities_bmp)?;
    let trees = load_bmp_indexed(&trees_bmp)?;
    let tree_tint = load_bmp_rgbx(&tree_tint_bmp)?;
    let tree_season = load_bmp_rgbx(&tree_season_bmp)?;
    let terrain_texture_map = load_terrain_texture_map(&terrain_defs)?;
    let terrain_texture_ids = map_terrain_texture_ids(&terrain_indices, &terrain_texture_map);
    let map_object_meta = load_map_object_meta(
        &default_map,
        &cities_txt,
        &cities,
        &trees,
        &terrain_indices,
        &terrain_texture_map,
        &seasons_txt,
        &buildings_txt,
        &railways_txt,
        &supply_nodes_txt,
        &unitstacks_txt,
        &weatherpositions_txt,
        &ambient_object_txt,
        &common_building_paths,
        &entity_source_paths,
    )?;
    // FW7 (Codex addition 1): payload rewriting starts here and `manifest.json`
    // is only rewritten ~450 lines later. Without `--force` (which defaults off)
    // nothing removed the old manifest first, so any failure in between left the
    // *previous* manifest — with the previous source hashes, counts and modset —
    // sitting beside a half-replaced payload set, and every consumer would read
    // it as a coherent bundle. Opening the write scope retires the manifest, and
    // it is the only thing that can hand out a payload path or write the new
    // manifest — so the ordering is a property of the types, not of a comment.
    let scope = BundleWriteScope::open(out)?;
    // The raw output path is deliberately shadowed away here. `OutputPathRetired`
    // has no `join`, no `display` and no `Deref`, so reintroducing an
    // `out.join("payload")` write anywhere below this line is a COMPILE ERROR,
    // not a silently reordered write. Everything after this point must go
    // through `scope`.
    #[allow(unused_variables)]
    let out = OutputPathRetired;
    write_u32_raw(&scope.payload("province_ids.r32"), &bmp.ids)?;
    write_u8_raw(
        &scope.payload("terrain_indices.r8"),
        &terrain_indices.pixels,
    )?;
    write_u8_raw(
        &scope.payload("terrain_texture_ids.r8"),
        &terrain_texture_ids,
    )?;
    write_u8_raw(&scope.payload("heightmap.r8"), &heightmap.pixels)?;
    write_u8_raw(&scope.payload("rivers.r8"), &rivers.pixels)?;
    write_u8_raw(&scope.payload("cities.r8"), &cities.pixels)?;
    write_u8_raw(&scope.payload("trees.r8"), &trees.pixels)?;
    write_u32_raw(&scope.payload("height_normal.rgbx"), &height_normal.pixels)?;
    write_u32_raw(&scope.payload("tree_tint.rgbx"), &tree_tint.pixels)?;
    write_u32_raw(&scope.payload("tree_season.rgbx"), &tree_season.pixels)?;
    write_u32_raw(
        &scope.payload("terrain_diffuse_atlas0.rgbx"),
        &terrain_atlas.pixels,
    )?;
    write_u32_raw(
        &scope.payload("terrain_normal_atlas0.rgbx"),
        &terrain_normal_atlas.pixels,
    )?;
    write_rgba_mip_chain_raw(
        &scope.payload("terrain_diffuse_atlas0_mips.rgba"),
        &terrain_atlas_mips,
    )?;
    write_rgba_mip_chain_raw(
        &scope.payload("terrain_normal_atlas0_mips.rgba"),
        &terrain_normal_atlas_mips,
    )?;
    let shader_rgba_specs = [
        (
            "map/terrain/atlas0.dds",
            "terrain_diffuse_atlas0.rgba",
            &atlas_dds,
        ),
        (
            "map/terrain/atlas_normal0.dds",
            "terrain_normal_atlas0.rgba",
            &atlas_normal_dds,
        ),
        (
            "map/terrain/colormap_rgb_cityemissivemask_a.dds",
            "terrain_color_tint.rgba",
            &colormap_dds,
        ),
        (
            "map/terrain/colormap_water_0.dds",
            "water_color_tint.rgba",
            &water_dds,
        ),
        (
            "map/terrain/underwater_terrain_0.dds",
            "underwater_terrain_0.rgba",
            &underwater_dds,
        ),
        (
            "map/terrain/fow_rgb_waterspec_a.dds",
            "water_spec_fow.rgba",
            &waterspec_dds,
        ),
        ("map/terrain/lean1.dds", "water_lean1.rgba", &lean1_dds),
        ("map/terrain/lean2.dds", "water_lean2.rgba", &lean2_dds),
        (
            "map/terrain/RiverSurface_diffuse_0.dds",
            "river_diffuse_0.rgba",
            &river_diffuse_dds,
        ),
        (
            "map/terrain/RiverSurface_normal_0.dds",
            "river_normal_0.rgba",
            &river_normal_dds,
        ),
        (
            "map/terrain/RiverSurface_masks.dds",
            "river_masks.rgba",
            &river_masks_dds,
        ),
        (
            "map/terrain/citylights_rgb_snowmask_a_0.dds",
            "citylights_snowmask_0.rgba",
            &citylights_snowmask_dds,
        ),
        (
            "map/terrain/snow_normal_rgb_diffuse_a.dds",
            "snow_normal_diffuse.rgba",
            &snow_normal_dds,
        ),
        (
            "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
            "mud_diffuse_gloss_0.rgba",
            &mud_diffuse_dds,
        ),
        (
            "map/terrain/mud_normal_rgb_spec_a_0.dds",
            "mud_normal_spec_0.rgba",
            &mud_normal_dds,
        ),
        (
            "map/terrain/ice_diffuse.dds",
            "ice_diffuse.rgba",
            &ice_diffuse_dds,
        ),
        (
            "map/terrain/ice_noise_0.dds",
            "ice_noise_0.rgba",
            &ice_noise_dds,
        ),
        (
            "map/terrain/border_country_0.dds",
            "border_country_0.rgba",
            &border_country_dds,
        ),
        (
            "map/terrain/border_state_0.dds",
            "border_state_0.rgba",
            &border_state_dds,
        ),
        (
            "map/terrain/border_sea_0.dds",
            "border_sea_0.rgba",
            &border_sea_dds,
        ),
        (
            "map/terrain/border_province_0.dds",
            "border_province_0.rgba",
            &border_province_dds,
        ),
        (
            "map/terrain/border_impassable_0.dds",
            "border_impassable_0.rgba",
            &border_impassable_dds,
        ),
        (
            "map/terrain/border_sea_region_0.dds",
            "border_sea_region_0.rgba",
            &border_sea_region_dds,
        ),
    ];
    let (mut shader_sources, mut shader_source_dimensions) =
        write_shader_rgba_sources(scope.dir(), &shader_rgba_specs)?;
    let (material_sources, material_dimensions) =
        write_material_rgba_sources(scope.dir(), &material_rgba_specs)?;
    shader_sources.extend(material_sources);
    shader_source_dimensions.extend(material_dimensions);
    for spec in &raw_material_specs {
        copy_shader_source(
            scope.dir(),
            &spec.source,
            &spec.output,
            &spec.path,
            &mut shader_sources,
        )?;
        shader_source_dimensions.insert(spec.source.clone(), dds_info(&spec.path)?.dimensions);
    }
    let map_material_contract = build_map_material_contract(
        &material_rgba_specs,
        &raw_material_specs,
        &shader_sources,
        shader_constants,
        shader_vector3_constants,
    )?;
    let mut map_source_specs = vec![
        source_spec("map/default.map", default_map.clone()),
        source_spec("map/cities.bmp", cities_bmp.clone()),
        source_spec("map/cities.txt", cities_txt.clone()),
        source_spec("map/trees.bmp", trees_bmp.clone()),
        source_spec("map/seasons.txt", seasons_txt.clone()),
        source_spec("map/positions.txt", positions_txt.clone()),
        source_spec("map/ambient_object.txt", ambient_object_txt.clone()),
        source_spec("map/buildings.txt", buildings_txt.clone()),
        source_spec("map/railways.txt", railways_txt.clone()),
        source_spec("map/supply_nodes.txt", supply_nodes_txt.clone()),
        source_spec("map/unitstacks.txt", unitstacks_txt.clone()),
        source_spec("map/weatherpositions.txt", weatherpositions_txt.clone()),
        source_spec("map/terrain/Tree_tint.bmp", tree_tint_bmp.clone()),
        source_spec("map/terrain/Tree_season.bmp", tree_season_bmp.clone()),
    ];
    map_source_specs.extend(
        common_building_paths
            .iter()
            .map(|(source, path)| source_spec(source, path.clone())),
    );
    map_source_specs.extend(
        entity_source_paths
            .iter()
            .map(|(source, path)| source_spec(source, path.clone())),
    );
    let mut map_sources = BTreeMap::new();
    for (source, output, path) in &map_source_specs {
        copy_bundle_source(scope.dir(), source, output, path, &mut map_sources)?;
    }
    write_json(&scope.payload("map_object_meta.json"), &map_object_meta)?;
    write_json(
        &scope.payload("map_material_contract.json"),
        &map_material_contract,
    )?;
    write_json(&scope.payload("province_to_state.json"), &province_to_state)?;
    write_json(&scope.payload("state_meta.json"), &state_meta)?;
    write_json(&scope.payload("state_owner_1936.json"), &state_owners)?;
    write_json(&scope.payload("country_colors.json"), &country_colors)?;
    write_json(&scope.payload("province_definitions.json"), &definitions)?;
    write_json(
        &scope.payload("strategic_region_to_provinces.json"),
        &region_to_provinces,
    )?;
    write_json(
        &scope.payload("province_to_strategic_region.json"),
        &province_to_region,
    )?;
    write_json(&scope.payload("strategic_region_meta.json"), &region_meta)?;

    let mut input_dimensions = visual_input_dimensions(&[
        ("map/provinces.bmp", bmp.width, bmp.height),
        (
            "map/terrain.bmp",
            terrain_indices.width,
            terrain_indices.height,
        ),
        ("map/heightmap.bmp", heightmap.width, heightmap.height),
        (
            "map/world_normal.bmp",
            height_normal.width,
            height_normal.height,
        ),
        (
            "map/terrain/colormap_rgb_cityemissivemask_a.dds",
            colormap.width,
            colormap.height,
        ),
        (
            "map/terrain/colormap_water_0.dds",
            water.width,
            water.height,
        ),
        (
            "map/terrain/atlas0.dds",
            terrain_atlas.width,
            terrain_atlas.height,
        ),
        (
            "map/terrain/atlas_normal0.dds",
            terrain_normal_atlas.width,
            terrain_normal_atlas.height,
        ),
        (
            "map/terrain/underwater_terrain_0.dds",
            underwater.width,
            underwater.height,
        ),
        (
            "map/terrain/fow_rgb_waterspec_a.dds",
            waterspec.width,
            waterspec.height,
        ),
        (
            "map/terrain/RiverSurface_diffuse_0.dds",
            river_diffuse.width,
            river_diffuse.height,
        ),
        (
            "map/terrain/border_country_0.dds",
            border_country.width,
            border_country.height,
        ),
        (
            "map/terrain/border_state_0.dds",
            border_state.width,
            border_state.height,
        ),
        (
            "map/terrain/border_sea_0.dds",
            border_sea.width,
            border_sea.height,
        ),
        ("map/rivers.bmp", rivers.width, rivers.height),
        ("map/cities.bmp", cities.width, cities.height),
        ("map/trees.bmp", trees.width, trees.height),
        (
            "map/terrain/Tree_tint.bmp",
            tree_tint.width,
            tree_tint.height,
        ),
        (
            "map/terrain/Tree_season.bmp",
            tree_season.width,
            tree_season.height,
        ),
    ]);
    input_dimensions.extend(shader_source_dimensions);
    input_dimensions.insert(
        "map/terrain/reflection.dds".to_string(),
        dds_dimensions(&reflection_dds)?,
    );

    let manifest = Manifest {
        schema: "battle-plan-rust-map-bundle-v1".to_string(),
        dimensions: Dimensions {
            width: bmp.width,
            height: bmp.height,
        },
        province_id_encoding: "little-endian u32 per pixel".to_string(),
        files: BundleFiles {
            province_ids: "province_ids.r32".to_string(),
            terrain_id_map: "terrain_id_map.rgba".to_string(),
            terrain_runtime: "terrain_runtime.json".to_string(),
            snow_mud_data: "snow_mud_data.rgba".to_string(),
            terrain_scene_cb0: "terrain_scene_cb0.bin".to_string(),
            province_secondary_color_map: "province_secondary_color_map.rgba".to_string(),
            gradient_border_channel1: "gradient_border_channel1.rgba".to_string(),
            gradient_border_channel2: "gradient_border_channel2.rgba".to_string(),
            gradient_border_channel3: "gradient_border_channel3.rgba".to_string(),
            light_index_map: "light_index_map.rgba".to_string(),
            light_data_map: "light_data_map.rgba".to_string(),
            shadow_map: "shadow_map.rgba".to_string(),
            terrain_indices: "terrain_indices.r8".to_string(),
            terrain_texture_ids: "terrain_texture_ids.r8".to_string(),
            heightmap: "heightmap.r8".to_string(),
            rivers: "rivers.r8".to_string(),
            height_normal: "height_normal.rgbx".to_string(),
            cities: "cities.r8".to_string(),
            trees: "trees.r8".to_string(),
            tree_tint: "tree_tint.rgbx".to_string(),
            tree_season: "tree_season.rgbx".to_string(),
            terrain_diffuse_atlas: "terrain_diffuse_atlas0.rgbx".to_string(),
            terrain_normal_atlas: "terrain_normal_atlas0.rgbx".to_string(),
            terrain_diffuse_atlas_mips: "terrain_diffuse_atlas0_mips.rgba".to_string(),
            terrain_normal_atlas_mips: "terrain_normal_atlas0_mips.rgba".to_string(),
            shader_sources: shader_sources.clone(),
            map_sources: map_sources.clone(),
            map_object_meta: "map_object_meta.json".to_string(),
            map_material_contract: "map_material_contract.json".to_string(),
            province_to_state: "province_to_state.json".to_string(),
            state_meta: "state_meta.json".to_string(),
            state_owner_1936: "state_owner_1936.json".to_string(),
            country_colors: "country_colors.json".to_string(),
            province_definitions: "province_definitions.json".to_string(),
            province_to_strategic_region: "province_to_strategic_region.json".to_string(),
            strategic_region_to_provinces: "strategic_region_to_provinces.json".to_string(),
            strategic_region_meta: "strategic_region_meta.json".to_string(),
        },
        counts: BundleCounts {
            definition_rows: definitions.len(),
            distinct_ids_in_provinces_bmp: distinct_count(&bmp.ids),
            states: state_meta.len(),
            state_owners_1936: state_owners.len(),
            province_to_state: province_to_state.len(),
            country_colors: country_colors.len(),
            strategic_regions: region_meta.len(),
            province_to_strategic_region: province_to_region.len(),
            unknown_pixels: bmp.unknown_pixels,
        },
        source: BundleSource {
            game_dir_name: game
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("the game")
                .to_string(),
            default_map: "map/default.map".to_string(),
            definition_csv_fnv1a64: fnv1a64_file(&definition_csv)?,
            provinces_bmp_fnv1a64: fnv1a64_file(&provinces_bmp)?,
            visual_input_fnv1a64: visual_input_hashes(&[
                ("map/default.map", &default_map),
                ("map/terrain.bmp", &terrain_bmp),
                ("map/heightmap.bmp", &heightmap_bmp),
                ("map/world_normal.bmp", &world_normal_bmp),
                (
                    "map/terrain/colormap_rgb_cityemissivemask_a.dds",
                    &colormap_dds,
                ),
                ("map/terrain/colormap_water_0.dds", &water_dds),
                ("map/terrain/atlas0.dds", &atlas_dds),
                ("map/terrain/atlas_normal0.dds", &atlas_normal_dds),
                ("map/terrain/underwater_terrain_0.dds", &underwater_dds),
                ("map/terrain/fow_rgb_waterspec_a.dds", &waterspec_dds),
                ("map/terrain/lean1.dds", &lean1_dds),
                ("map/terrain/lean2.dds", &lean2_dds),
                ("map/terrain/reflection.dds", &reflection_dds),
                (
                    "map/terrain/RiverSurface_diffuse_0.dds",
                    &river_diffuse_dds,
                ),
                (
                    "map/terrain/RiverSurface_normal_0.dds",
                    &river_normal_dds,
                ),
                ("map/terrain/RiverSurface_masks.dds", &river_masks_dds),
                (
                    "map/terrain/citylights_rgb_snowmask_a_0.dds",
                    &citylights_snowmask_dds,
                ),
                (
                    "map/terrain/snow_normal_rgb_diffuse_a.dds",
                    &snow_normal_dds,
                ),
                (
                    "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
                    &mud_diffuse_dds,
                ),
                (
                    "map/terrain/mud_normal_rgb_spec_a_0.dds",
                    &mud_normal_dds,
                ),
                ("map/terrain/ice_diffuse.dds", &ice_diffuse_dds),
                ("map/terrain/ice_noise_0.dds", &ice_noise_dds),
                ("map/terrain/border_country_0.dds", &border_country_dds),
                ("map/terrain/border_province_0.dds", &border_province_dds),
                (
                    "map/terrain/border_impassable_0.dds",
                    &border_impassable_dds,
                ),
                ("map/terrain/border_state_0.dds", &border_state_dds),
                ("map/terrain/border_sea_0.dds", &border_sea_dds),
                (
                    "map/terrain/border_sea_region_0.dds",
                    &border_sea_region_dds,
                ),
                ("map/rivers.bmp", &rivers_bmp),
                ("map/cities.bmp", &cities_bmp),
                ("map/cities.txt", &cities_txt),
                ("map/trees.bmp", &trees_bmp),
                ("map/seasons.txt", &seasons_txt),
                ("map/positions.txt", &positions_txt),
                ("map/ambient_object.txt", &ambient_object_txt),
                ("map/buildings.txt", &buildings_txt),
                ("map/railways.txt", &railways_txt),
                ("map/supply_nodes.txt", &supply_nodes_txt),
                ("map/unitstacks.txt", &unitstacks_txt),
                ("map/weatherpositions.txt", &weatherpositions_txt),
                ("map/terrain/Tree_tint.bmp", &tree_tint_bmp),
                ("map/terrain/Tree_season.bmp", &tree_season_bmp),
                ("common/terrain/00_terrain.txt", &terrain_defs),
            ]
            .into_iter()
            .chain(
                material_rgba_specs
                    .iter()
                    .map(|spec| (spec.source.as_str(), &spec.path)),
            )
            .chain(
                raw_material_specs
                    .iter()
                    .map(|spec| (spec.source.as_str(), &spec.path)),
            )
            .chain(
                map_source_specs
                    .iter()
                    .map(|(source, _, path)| (source.as_str(), path)),
            )
            .collect::<Vec<_>>()
            .as_slice())?,
            visual_input_dimensions: input_dimensions,
        },
        modset: resolver_setup.manifest_modset(),
        viewer_note: "No rendered map underlay is generated here. Product map rendering requires validated the game D3D11 captures or a fully reproduced engine terrain pipeline with all runtime/generated inputs accounted for.".to_string(),
    };
    scope.write_manifest(&manifest)?;

    println!(
        "[province_ids.r32] {}x{}, {} ids",
        bmp.width, bmp.height, manifest.counts.distinct_ids_in_provinces_bmp
    );
    println!("[terrain_indices.r8] from map/terrain.bmp");
    println!("[terrain_texture_ids.r8] from common/terrain/00_terrain.txt texture ids");
    println!("[heightmap.r8] from map/heightmap.bmp");
    println!("[rivers.r8] from map/rivers.bmp");
    println!(
        "[cities.r8] {}x{} from map/cities.bmp",
        cities.width, cities.height
    );
    println!(
        "[trees.r8] {}x{} from map/trees.bmp",
        trees.width, trees.height
    );
    println!(
        "[height_normal.rgbx] {}x{} from map/world_normal.bmp",
        height_normal.width, height_normal.height
    );
    println!(
        "[tree_*.rgbx] tint {}x{}, season {}x{} from map/terrain/Tree_*.bmp",
        tree_tint.width, tree_tint.height, tree_season.width, tree_season.height
    );
    println!(
        "[terrain_*_atlas0.rgbx] diffuse {}x{}, normal {}x{}",
        terrain_atlas.width,
        terrain_atlas.height,
        terrain_normal_atlas.width,
        terrain_normal_atlas.height
    );
    println!(
        "[shader_sources] {} alpha-preserving material inputs",
        shader_sources.len()
    );
    println!(
        "[map_sources] {} exact source files copied without rendering claims",
        map_sources.len()
    );
    println!(
        "[map_object_meta.json] {} city groups, {} static source tables",
        map_object_meta.cities.city_groups.len(),
        map_object_meta.static_sources.len()
    );
    println!(
        "[map_material_contract.json] {} shipped material sources, {} shader constants, {} generated/runtime blockers",
        map_material_contract.shipped_sources.len(),
        map_material_contract.shader_constants.len(),
        map_material_contract.generated_runtime_inputs.len()
    );
    println!("------------------------------------------------------------------------");
    println!("states: {}", state_meta.len());
    println!("strategic regions: {}", region_meta.len());
    println!("unknown pixels: {}", bmp.unknown_pixels);
    println!("bundle written: {}", scope.dir().display());
    Ok(())
}

/// A bundle output directory whose stale `manifest.json` has already been
/// retired (FW7, Codex addition 1).
///
/// The ordering this enforces used to be a comment: `build_bundle` overwrote
/// payloads from the province raster onward and only rewrote the manifest ~450
/// lines later, and `--force` — the one thing that removed the old manifest —
/// defaults off. Any failure in between left the previous manifest, with the
/// previous source hashes, counts and modset, describing a half-replaced
/// payload set that every consumer would read as coherent.
///
/// Making the scope the ONLY source of a payload path and the ONLY way to write
/// the manifest turns "retire first" from a convention into something the
/// compiler enforces: you cannot name a file inside the bundle without holding a
/// scope, and you cannot hold a scope without having retired the manifest.
pub(crate) struct BundleWriteScope {
    out: PathBuf,
}

/// Shadows the raw output path once a [`BundleWriteScope`] is open. It has no
/// path methods at all, so any write that tries to address the bundle directory
/// without going through the scope fails to compile.
pub(crate) struct OutputPathRetired;

impl BundleWriteScope {
    /// Create the output directory and retire any stale manifest.
    pub(crate) fn open(out: &Path) -> Result<Self> {
        fs::create_dir_all(out)?;
        invalidate_bundle_manifest(out)?;
        Ok(Self {
            out: out.to_path_buf(),
        })
    }

    /// Path of one payload inside the bundle.
    pub(crate) fn payload(&self, name: &str) -> PathBuf {
        self.out.join(name)
    }

    /// The bundle directory, for helpers that write several payloads.
    pub(crate) fn dir(&self) -> &Path {
        &self.out
    }

    /// Write the new manifest. Reachable only from a scope, i.e. only after the
    /// stale one was retired.
    pub(crate) fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        write_json(&self.out.join("manifest.json"), manifest)
    }
}

/// Retire `manifest.json` before any payload in `out` is overwritten (FW7,
/// Codex addition 1). A bundle directory with payloads but no manifest is
/// unloadable, which is the honest state during a regeneration; a bundle with a
/// stale manifest over fresh payloads is a silently wrong one.
pub(crate) fn invalidate_bundle_manifest(out: &Path) -> Result<()> {
    let manifest = out.join("manifest.json");
    match fs::remove_file(&manifest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "could not retire {} before regenerating the bundle: {error}",
            manifest.display()
        )
        .into()),
    }
}

pub(crate) fn remove_known_bundle_files(out: &Path) -> Result<()> {
    for name in [
        "manifest.json",
        "province_ids.r32",
        "terrain_id_map.rgba",
        "terrain_runtime.json",
        "snow_mud_data.rgba",
        "terrain_scene_cb0.bin",
        "province_secondary_color_map.rgba",
        "gradient_border_channel1.rgba",
        "gradient_border_channel2.rgba",
        "gradient_border_channel3.rgba",
        "light_index_map.rgba",
        "light_data_map.rgba",
        "shadow_map.rgba",
        "terrain_indices.r8",
        "terrain_texture_ids.r8",
        "heightmap.r8",
        "rivers.r8",
        "cities.r8",
        "trees.r8",
        "height_normal.rgbx",
        "tree_tint.rgbx",
        "tree_season.rgbx",
        "terrain_diffuse_atlas0.rgbx",
        "terrain_normal_atlas0.rgbx",
        "terrain_diffuse_atlas0_mips.rgbx",
        "terrain_normal_atlas0_mips.rgbx",
        "terrain_diffuse_atlas0_mips.rgba",
        "terrain_normal_atlas0_mips.rgba",
        "terrain_diffuse_atlas0.rgba",
        "terrain_normal_atlas0.rgba",
        "terrain_color_tint.rgba",
        "water_color_tint.rgba",
        "underwater_terrain_0.rgba",
        "water_spec_fow.rgba",
        "water_lean1.rgba",
        "water_lean2.rgba",
        "reflection.dds",
        "river_diffuse_0.rgba",
        "river_normal_0.rgba",
        "river_masks.rgba",
        "citylights_snowmask_0.rgba",
        "snow_normal_diffuse.rgba",
        "mud_diffuse_gloss_0.rgba",
        "mud_normal_spec_0.rgba",
        "ice_diffuse.rgba",
        "ice_noise_0.rgba",
        "border_country_0.rgba",
        "border_state_0.rgba",
        "border_sea_0.rgba",
        "border_province_0.rgba",
        "border_impassable_0.rgba",
        "border_sea_region_0.rgba",
        "source_default.map",
        "source_cities.bmp",
        "source_cities.txt",
        "source_trees.bmp",
        "source_seasons.txt",
        "source_positions.txt",
        "source_ambient_object.txt",
        "source_buildings.txt",
        "source_railways.txt",
        "source_supply_nodes.txt",
        "source_unitstacks.txt",
        "source_weatherpositions.txt",
        "source_tree_tint.bmp",
        "source_tree_season.bmp",
        "map_object_meta.json",
        "map_material_contract.json",
        "province_to_state.json",
        "state_meta.json",
        "state_owner_1936.json",
        "country_colors.json",
        "province_definitions.json",
        "strategic_region_to_provinces.json",
        "province_to_strategic_region.json",
        "strategic_region_meta.json",
    ] {
        let path = out.join(name);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    for entry in fs::read_dir(out)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("source_") || (name.starts_with("shader_") && name.ends_with(".rgba")) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub(crate) fn import_terrain_runtime_inputs(options: &ImportTerrainRuntimeOptions) -> Result<()> {
    let manifest_path = options.bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }

    let source = require_file(
        &options.terrain_id_map_rgba,
        "runtime-extracted TerrainIDMap RGBA",
    )?;
    let expected_len = checked_rgba_byte_len(options.width, options.height)?;
    let actual_len = fs::metadata(&source)?.len();
    if actual_len != expected_len as u64 {
        return Err(format!(
            "{} has {} bytes, expected {} for {}x{} RGBA",
            source.display(),
            actual_len,
            expected_len,
            options.width,
            options.height
        )
        .into());
    }

    let terrain_id_map_path = options.bundle.join(&manifest.files.terrain_id_map);
    if terrain_id_map_path.exists() && !options.force {
        return Err(format!(
            "{} already exists; pass --force to replace the captured runtime TerrainIDMap",
            terrain_id_map_path.display()
        )
        .into());
    }
    fs::copy(&source, &terrain_id_map_path)?;

    let runtime = TerrainRuntimeInputs {
        schema: TERRAIN_RUNTIME_SCHEMA.to_string(),
        terrain_id_map: TerrainIdMapRuntimeMeta {
            file: manifest.files.terrain_id_map.clone(),
            width: options.width,
            height: options.height,
            byte_len: expected_len,
            fnv1a64: fnv1a64_file(&terrain_id_map_path)?,
            format: TERRAIN_ID_MAP_FORMAT.to_string(),
            channel_order: terrain_id_map_channel_order(),
            sampler: terrain_id_map_sampler(),
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: "runtime_extracted_texture_no_static_fallback".to_string(),
            },
        },
        map_pow2: MapPow2RuntimeMeta {
            x: options.map_pow2_x,
            y: options.map_pow2_y,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: options.map_pow2_extraction.clone(),
            },
        },
        fow_pow2: MapPow2RuntimeMeta {
            x: options.fow_pow2_x,
            y: options.fow_pow2_y,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: options.fow_pow2_extraction.clone(),
            },
        },
        runtime_textures: BTreeMap::new(),
        runtime_constant_buffers: BTreeMap::new(),
        runtime_vertex_buffers: BTreeMap::new(),
        runtime_input_layouts: BTreeMap::new(),
    };

    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    write_json(
        &options.bundle.join(&manifest.files.terrain_runtime),
        &runtime,
    )?;
    println!(
        "imported TerrainIDMap {}x{} ({} bytes), MAP_POW2 {}, {}, and FOW_POW2 {}, {} into {}",
        runtime.terrain_id_map.width,
        runtime.terrain_id_map.height,
        runtime.terrain_id_map.byte_len,
        runtime.map_pow2.x,
        runtime.map_pow2.y,
        runtime.fow_pow2.x,
        runtime.fow_pow2.y,
        options.bundle.display()
    );
    Ok(())
}

pub(crate) fn import_runtime_texture_input(options: &ImportRuntimeTextureOptions) -> Result<()> {
    let manifest_path = options.bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let terrain_runtime = required_bundle_file(
        &options.bundle,
        "terrain runtime input contract",
        &manifest.files.terrain_runtime,
    )?;
    let mut runtime: TerrainRuntimeInputs = serde_json::from_slice(&fs::read(&terrain_runtime)?)?;
    let terrain_id_map_path = options.bundle.join(&manifest.files.terrain_id_map);
    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    let output = runtime_texture_file_for_symbol(&manifest, &options.shader_symbol)?;

    let pdxmap_shader_source =
        required_map_source_path(&options.bundle, &manifest, "gfx/FX/pdxmap.shader")?;
    validate_pdxmap_sampler_contract(&pdxmap_shader_source)?;
    let pdxmap_shader_text = read_latin1(&pdxmap_shader_source)?;
    let sampler = parse_pdxmap_sampler_contract(&pdxmap_shader_text, &options.shader_symbol)?;

    let source_label = format!("runtime-extracted {} RGBA", options.shader_symbol);
    let source = require_file(&options.source_rgba, &source_label)?;
    let expected_len = checked_rgba_byte_len(options.width, options.height)?;
    let actual_len = fs::metadata(&source)?.len();
    if actual_len != expected_len as u64 {
        return Err(format!(
            "{} has {} bytes, expected {} for {}x{} RGBA",
            source.display(),
            actual_len,
            expected_len,
            options.width,
            options.height
        )
        .into());
    }

    let output_path = options.bundle.join(&output);
    if output_path.exists() && !options.force {
        return Err(format!(
            "{} already exists; pass --force to replace the captured runtime {} texture",
            output_path.display(),
            options.shader_symbol
        )
        .into());
    }
    fs::copy(&source, &output_path)?;

    runtime.runtime_textures.insert(
        options.shader_symbol.clone(),
        RuntimeTextureRuntimeMeta {
            file: output.clone(),
            width: options.width,
            height: options.height,
            byte_len: expected_len,
            fnv1a64: fnv1a64_file(&output_path)?,
            format: RUNTIME_RGBA8_FORMAT.to_string(),
            sampler,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: "runtime_extracted_texture_no_static_fallback".to_string(),
            },
        },
    );

    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    validate_runtime_texture_sampler_contract(&runtime, &pdxmap_shader_source)?;
    write_json(&terrain_runtime, &runtime)?;
    println!(
        "imported {} {}x{} ({} bytes) into {}",
        options.shader_symbol,
        options.width,
        options.height,
        expected_len,
        options.bundle.display()
    );
    Ok(())
}

pub(crate) fn import_runtime_constant_buffer_input(
    options: &ImportRuntimeConstantBufferOptions,
) -> Result<()> {
    let manifest_path = options.bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let terrain_runtime = required_bundle_file(
        &options.bundle,
        "terrain runtime input contract",
        &manifest.files.terrain_runtime,
    )?;
    let mut runtime: TerrainRuntimeInputs = serde_json::from_slice(&fs::read(&terrain_runtime)?)?;
    let terrain_id_map_path = options.bundle.join(&manifest.files.terrain_id_map);
    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;

    let output = runtime_constant_buffer_file_for_symbol(&manifest, &options.shader_symbol)?;
    let bind_point = runtime_constant_buffer_bind_point_for_symbol(&options.shader_symbol)?;
    let standard_functions_source =
        required_map_source_path(&options.bundle, &manifest, "gfx/FX/standardfuncsgfx.fxh")?;
    let standard_functions_text = read_latin1(&standard_functions_source)?;
    let layout = terrain_scene_cb0_layout(&standard_functions_text)?;
    let required_offsets = terrain_scene_cb0_required_offsets(&layout)?;

    let source_label = format!(
        "runtime-extracted {} constant buffer",
        options.shader_symbol
    );
    let source = require_file(&options.source_bin, &source_label)?;
    let byte_len = usize::try_from(fs::metadata(&source)?.len())
        .map_err(|_| format!("{} is too large to address", source.display()))?;
    validate_constant_buffer_byte_len(byte_len, layout.byte_len)?;

    let output_path = options.bundle.join(&output);
    if output_path.exists() && !options.force {
        return Err(format!(
            "{} already exists; pass --force to replace the captured runtime {} constant buffer",
            output_path.display(),
            options.shader_symbol
        )
        .into());
    }
    fs::copy(&source, &output_path)?;

    runtime.runtime_constant_buffers.insert(
        options.shader_symbol.clone(),
        RuntimeConstantBufferRuntimeMeta {
            file: output.clone(),
            byte_len,
            fnv1a64: fnv1a64_file(&output_path)?,
            format: RUNTIME_CONSTANT_BUFFER_FORMAT.to_string(),
            bind_point,
            layout_source: TERRAIN_SCENE_CB0_LAYOUT_SOURCE.to_string(),
            layout_byte_len: layout.byte_len,
            required_offsets,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: "runtime_extracted_constant_buffer_no_static_fallback".to_string(),
            },
        },
    );

    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    write_json(&terrain_runtime, &runtime)?;
    println!(
        "imported {} constant buffer {} bytes into {}",
        options.shader_symbol,
        byte_len,
        options.bundle.display()
    );
    Ok(())
}

pub(crate) fn import_runtime_vertex_buffer_input(
    options: &ImportRuntimeVertexBufferOptions,
) -> Result<()> {
    let manifest_path = options.bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let terrain_runtime = required_bundle_file(
        &options.bundle,
        "terrain runtime input contract",
        &manifest.files.terrain_runtime,
    )?;
    let mut runtime: TerrainRuntimeInputs = serde_json::from_slice(&fs::read(&terrain_runtime)?)?;
    let terrain_id_map_path = options.bundle.join(&manifest.files.terrain_id_map);
    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;

    if options.shader_symbol != TERRAIN_VERTEX_BUFFER_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported runtime vertex buffer symbol {}",
            options.shader_symbol
        )
        .into());
    }

    let capture: TerrainVertexBufferCaptureFile =
        serde_json::from_slice(&fs::read(&options.capture_json)?)?;
    validate_terrain_vertex_buffer_capture_file(&capture)?;
    let source_sample = options
        .capture_json
        .parent()
        .ok_or_else(|| {
            format!(
                "{} has no parent directory for captured vertex buffer sample",
                options.capture_json.display()
            )
        })?
        .join(&capture.file);
    let source_sample = require_file(
        &source_sample,
        "runtime-extracted TerrainVertexBuffer sample",
    )?;
    let actual_len = usize::try_from(fs::metadata(&source_sample)?.len())
        .map_err(|_| format!("{} is too large to address", source_sample.display()))?;
    if actual_len != capture.captured_byte_len {
        return Err(format!(
            "{} has {} bytes, capture metadata expects {}",
            source_sample.display(),
            actual_len,
            capture.captured_byte_len
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&source_sample)?;
    if actual_hash != capture.fnv1a64 {
        return Err(format!(
            "{} hash changed from {} to {}",
            source_sample.display(),
            capture.fnv1a64,
            actual_hash
        )
        .into());
    }

    let output = capture.file.clone();
    let output_path = options.bundle.join(&output);
    if output_path.exists() && !options.force {
        return Err(format!(
            "{} already exists; pass --force to replace the captured runtime {} vertex buffer",
            output_path.display(),
            options.shader_symbol
        )
        .into());
    }
    fs::copy(&source_sample, &output_path)?;

    runtime.runtime_vertex_buffers.insert(
        options.shader_symbol.clone(),
        RuntimeVertexBufferRuntimeMeta {
            file: output.clone(),
            source_byte_width: capture.source_byte_width,
            captured_byte_len: capture.captured_byte_len,
            fnv1a64: fnv1a64_file(&output_path)?,
            format: RUNTIME_VERTEX_BUFFER_FORMAT.to_string(),
            stride: capture.stride,
            offset: capture.offset,
            minimum_stride_source: capture.minimum_stride_source.clone(),
            draw_call: capture.draw_call.clone(),
            draw_count: capture.draw_count,
            draw_start: capture.draw_start,
            base_vertex_location: capture.base_vertex_location,
            terrain_id_srv_slot: capture.terrain_id_srv_slot,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: capture.source.clone(),
            },
        },
    );

    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    write_json(&terrain_runtime, &runtime)?;
    println!(
        "imported {} vertex buffer sample {} bytes, stride {}, draw {} into {}",
        options.shader_symbol,
        capture.captured_byte_len,
        capture.stride,
        capture.draw_call,
        options.bundle.display()
    );
    Ok(())
}

pub(crate) fn import_runtime_input_layout(options: &ImportRuntimeInputLayoutOptions) -> Result<()> {
    let manifest_path = options.bundle.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != "battle-plan-rust-map-bundle-v1" {
        return Err(format!("unsupported map bundle schema: {}", manifest.schema).into());
    }
    let terrain_runtime = required_bundle_file(
        &options.bundle,
        "terrain runtime input contract",
        &manifest.files.terrain_runtime,
    )?;
    let mut runtime: TerrainRuntimeInputs = serde_json::from_slice(&fs::read(&terrain_runtime)?)?;
    let terrain_id_map_path = options.bundle.join(&manifest.files.terrain_id_map);
    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;

    if options.shader_symbol != TERRAIN_INPUT_LAYOUT_SYMBOL {
        return Err(format!(
            "production terrain renderer blocked: unsupported runtime input layout symbol {}",
            options.shader_symbol
        )
        .into());
    }

    let capture: TerrainInputLayoutCaptureFile =
        serde_json::from_slice(&fs::read(&options.capture_json)?)?;
    validate_terrain_input_layout_capture_file(&capture)?;
    if runtime
        .runtime_input_layouts
        .contains_key(&options.shader_symbol)
        && !options.force
    {
        return Err(format!(
            "{} already records {}; pass --force to replace the captured runtime input layout",
            terrain_runtime.display(),
            options.shader_symbol
        )
        .into());
    }

    runtime.runtime_input_layouts.insert(
        options.shader_symbol.clone(),
        RuntimeInputLayoutRuntimeMeta {
            element_count: capture.element_count,
            elements: capture.elements.clone(),
            format: RUNTIME_INPUT_LAYOUT_FORMAT.to_string(),
            vertex_stride: capture.vertex_stride,
            vertex_offset: capture.vertex_offset,
            shader_input_source: capture.shader_input_source.clone(),
            draw_call: capture.draw_call.clone(),
            draw_count: capture.draw_count,
            draw_start: capture.draw_start,
            base_vertex_location: capture.base_vertex_location,
            terrain_id_srv_slot: capture.terrain_id_srv_slot,
            source: RuntimeInputSource {
                provenance: normalized_provenance(&options.provenance),
                extraction: capture.source.clone(),
            },
        },
    );

    validate_terrain_runtime_inputs(&runtime, &manifest, &options.bundle, &terrain_id_map_path)?;
    write_json(&terrain_runtime, &runtime)?;
    println!(
        "imported {} input layout {} elements, stride {}, draw {} into {}",
        options.shader_symbol,
        capture.element_count,
        capture.vertex_stride,
        capture.draw_call,
        options.bundle.display()
    );
    Ok(())
}

pub(crate) fn runtime_texture_file_for_symbol(manifest: &Manifest, symbol: &str) -> Result<String> {
    match symbol {
        SNOW_MUD_DATA_SYMBOL => Ok(manifest.files.snow_mud_data.clone()),
        PROVINCE_SECONDARY_COLOR_MAP_SYMBOL => {
            Ok(manifest.files.province_secondary_color_map.clone())
        }
        GRADIENT_BORDER_CHANNEL1_SYMBOL => Ok(manifest.files.gradient_border_channel1.clone()),
        GRADIENT_BORDER_CHANNEL2_SYMBOL => Ok(manifest.files.gradient_border_channel2.clone()),
        GRADIENT_BORDER_CHANNEL3_SYMBOL => Ok(manifest.files.gradient_border_channel3.clone()),
        LIGHT_INDEX_MAP_SYMBOL => Ok(manifest.files.light_index_map.clone()),
        LIGHT_DATA_MAP_SYMBOL => Ok(manifest.files.light_data_map.clone()),
        SHADOW_MAP_SYMBOL => Ok(manifest.files.shadow_map.clone()),
        _ => Err(format!(
            "production terrain renderer blocked: unsupported runtime texture symbol {symbol}"
        )
        .into()),
    }
}

pub(crate) fn runtime_constant_buffer_file_for_symbol(
    manifest: &Manifest,
    symbol: &str,
) -> Result<String> {
    match symbol {
        TERRAIN_SCENE_CB0_SYMBOL => Ok(manifest.files.terrain_scene_cb0.clone()),
        _ => Err(format!(
            "production terrain renderer blocked: unsupported runtime constant buffer symbol {symbol}"
        )
        .into()),
    }
}

pub(crate) fn runtime_constant_buffer_bind_point_for_symbol(symbol: &str) -> Result<u32> {
    match symbol {
        TERRAIN_SCENE_CB0_SYMBOL => Ok(TERRAIN_SCENE_CB0_BIND_POINT),
        _ => Err(format!(
            "production terrain renderer blocked: unsupported runtime constant buffer symbol {symbol}"
        )
        .into()),
    }
}

pub(crate) fn checked_rgba_byte_len(width: usize, height: usize) -> Result<usize> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("RGBA dimensions overflow byte length: {width}x{height}").into())
}

pub(crate) fn checked_rgba32f_byte_len(width: usize, height: usize) -> Result<usize> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(16))
        .ok_or_else(|| {
            format!("R32G32B32A32_FLOAT dimensions overflow byte length: {width}x{height}").into()
        })
}

pub(crate) fn normalized_provenance(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "runtime_extraction_unlabeled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn terrain_id_map_channel_order() -> Vec<String> {
    [
        "ColorRD",
        "ColorLU",
        "ColorRU_or_all_same_plus_100",
        "initial_diffuse_alpha",
    ]
    .iter()
    .map(|value| (*value).to_string())
    .collect()
}

pub(crate) fn terrain_id_map_sampler() -> TerrainRuntimeSampler {
    TerrainRuntimeSampler {
        mag_filter: "Point".to_string(),
        min_filter: "Point".to_string(),
        mip_filter: "None".to_string(),
        address_u: "Clamp".to_string(),
        address_v: "Clamp".to_string(),
    }
}

pub(crate) fn terrain_linear_wrap_sampler() -> TerrainRuntimeSampler {
    TerrainRuntimeSampler {
        mag_filter: "Linear".to_string(),
        min_filter: "Linear".to_string(),
        mip_filter: "Linear".to_string(),
        address_u: "Wrap".to_string(),
        address_v: "Wrap".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::collections::BTreeMap;
    use std::fs;

    /// FW7 (Codex addition 1): a regeneration retires the stale manifest before
    /// it can name a single payload path, so an interrupted run leaves an
    /// unloadable directory rather than a stale manifest over half-replaced
    /// payloads. `--force` defaults off, so nothing else removed it.
    ///
    /// The ordering is structural: `payload()` and `write_manifest()` are
    /// reachable only through a `BundleWriteScope`, and `open()` is the only
    /// constructor. This test would not compile against a version that let
    /// `build_bundle` write a payload without opening the scope first.
    #[test]
    fn opening_the_write_scope_retires_the_stale_manifest_before_any_payload_path_exists() {
        let root = temp_test_dir("bundle_write_scope");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest_path = root.join("manifest.json");
        write_json(&manifest_path, &minimal_test_manifest()).expect("write manifest");
        let payload_path = root.join("province_ids.r32");
        fs::write(&payload_path, [1u8, 2, 3, 4]).expect("write stale payload");
        assert!(manifest_path.is_file());

        let scope = BundleWriteScope::open(&root).expect("open write scope");
        assert!(
            !manifest_path.exists(),
            "the stale manifest must be gone before any payload path can be named"
        );
        // The stale payload is still there — that is the point: the directory is
        // now unloadable rather than plausibly coherent.
        assert!(payload_path.is_file());

        assert_eq!(scope.payload("province_ids.r32"), payload_path);
        assert_eq!(scope.dir(), root.as_path());
        scope
            .write_manifest(&minimal_test_manifest())
            .expect("write the new manifest");
        assert!(
            manifest_path.is_file(),
            "the bundle becomes loadable again only once the new manifest lands"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The failure mode the ordering exists for: a regeneration that dies part
    /// way through the payload rewrite must leave a directory with NO manifest,
    /// so a consumer refuses it, rather than the previous manifest — previous
    /// source hashes, counts and modset — sitting over half-replaced payloads.
    #[test]
    fn a_regeneration_that_fails_mid_payload_leaves_no_manifest_behind() {
        let root = temp_test_dir("bundle_write_scope_interrupted");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest_path = root.join("manifest.json");
        write_json(&manifest_path, &minimal_test_manifest()).expect("write old manifest");
        fs::write(root.join("province_ids.r32"), [1u8, 2, 3, 4]).expect("write old payload");
        fs::write(root.join("heightmap.r8"), [9u8]).expect("write old payload");

        // The regeneration sequence, aborted after the first payload lands.
        let interrupted = (|| -> Result<()> {
            let scope = BundleWriteScope::open(&root)?;
            write_u32_raw(&scope.payload("province_ids.r32"), &[7u32, 8])?;
            Err("simulated failure part way through the payload rewrite".into())
        })();
        assert!(interrupted.is_err());

        assert!(
            !manifest_path.exists(),
            "the stale manifest must not survive a partial regeneration"
        );
        // The first payload was replaced and the second was not — exactly the
        // mixed state the old manifest would have described as coherent.
        assert_eq!(
            portable_test_fs_retry::read(root.join("province_ids.r32"))
                .expect("read payload")
                .len(),
            8
        );
        assert_eq!(
            portable_test_fs_retry::read(root.join("heightmap.r8")).expect("read payload"),
            [9u8]
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// A first-time bake has no manifest to retire, and that is not an error.
    #[test]
    fn write_scope_opens_cleanly_on_a_fresh_directory() {
        let root = temp_test_dir("bundle_write_scope_fresh").join("nested");
        assert!(!root.exists());
        let scope = BundleWriteScope::open(&root).expect("open on a fresh directory");
        assert!(scope.dir().is_dir());
        assert!(!root.join("manifest.json").exists());
        invalidate_bundle_manifest(&root).expect("absent manifest is not an error");
        let _ = fs::remove_dir_all(root.parent().expect("parent"));
    }

    #[test]
    fn terrain_runtime_import_rejects_wrong_byte_count() {
        let root = temp_test_dir("bad_terrain_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = minimal_test_manifest();
        write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
        let source = root.join("bad_terrain_id_map.rgba");
        fs::write(&source, [1u8, 2, 3]).expect("write short source");

        let options = ImportTerrainRuntimeOptions {
            bundle: root.clone(),
            terrain_id_map_rgba: source,
            width: 1,
            height: 1,
            map_pow2_x: 1.0,
            map_pow2_y: 1.0,
            map_pow2_extraction: "runtime_extracted_shader_constant_no_static_fallback".to_string(),
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            fow_pow2_extraction: "runtime_extracted_shader_constant_no_static_fallback".to_string(),
            provenance: "unit_test_capture".to_string(),
            force: false,
        };
        let err = import_terrain_runtime_inputs(&options)
            .expect_err("wrong TerrainIDMap byte count must block import");
        assert!(err.to_string().contains("expected 4"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn snow_mud_runtime_import_records_strict_shader_metadata() {
        let root = temp_test_dir("snow_mud_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let mut manifest = minimal_test_manifest();
        manifest.files.shader_sources = BTreeMap::from([
            (
                "map/terrain/atlas0.dds".to_string(),
                "shader_map_terrain_atlas0.rgba".to_string(),
            ),
            (
                "map/terrain/atlas_normal0.dds".to_string(),
                "shader_map_terrain_atlas_normal0.rgba".to_string(),
            ),
            (
                "map/terrain/colormap_rgb_cityemissivemask_a.dds".to_string(),
                "shader_map_terrain_colormap_rgb_cityemissivemask_a.rgba".to_string(),
            ),
        ]);
        manifest.files.map_sources = BTreeMap::from([
            (
                "gfx/FX/constants.fxh".to_string(),
                "source_gfx_fx_constants_fxh".to_string(),
            ),
            (
                "gfx/FX/pdxmap.shader".to_string(),
                "source_gfx_fx_pdxmap_shader".to_string(),
            ),
            (
                "gfx/FX/pdxmap.fxh".to_string(),
                "source_gfx_fx_pdxmap_fxh".to_string(),
            ),
            (
                "gfx/FX/standardfuncsgfx.fxh".to_string(),
                "source_gfx_fx_standardfuncsgfx_fxh".to_string(),
            ),
            (
                "gfx/FX/shadow.fxh".to_string(),
                "source_gfx_fx_shadow_fxh".to_string(),
            ),
            (
                "gfx/FX/fow.fxh".to_string(),
                "source_gfx_fx_fow_fxh".to_string(),
            ),
        ]);
        write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
        write_json(
            &root.join("map_material_contract.json"),
            &minimal_test_contract(),
        )
        .expect("write contract");
        for output in manifest.files.shader_sources.values() {
            fs::write(root.join(output), [7u8]).expect("write shader source file");
        }
        for (source, output) in &manifest.files.map_sources {
            let bytes = minimal_map_source_bytes(source);
            fs::write(root.join(output), bytes).expect("write map source file");
        }
        fs::write(root.join(&manifest.files.terrain_id_map), [1u8; 16])
            .expect("write terrain id map");
        let runtime = minimal_test_runtime(&manifest, &root.join(&manifest.files.terrain_id_map));
        write_json(&root.join(&manifest.files.terrain_runtime), &runtime)
            .expect("write terrain runtime contract");

        let source = root.join("captured_snow_mud.rgba");
        fs::write(&source, [1u8, 2, 3, 4, 5, 6, 7, 8]).expect("write snow mud source");
        let options = ImportRuntimeTextureOptions {
            bundle: root.clone(),
            source_rgba: source,
            width: 2,
            height: 1,
            shader_symbol: SNOW_MUD_DATA_SYMBOL.to_string(),
            provenance: "unit_test_snow_mud_capture".to_string(),
            force: false,
        };
        import_runtime_texture_input(&options).expect("import SnowMudData runtime texture");

        let runtime: TerrainRuntimeInputs =
            serde_json::from_slice(&portable_test_fs_retry::read(
                root.join(&manifest.files.terrain_runtime)
            ).unwrap())
                .expect("read runtime contract");
        let snow = runtime
            .runtime_textures
            .get(SNOW_MUD_DATA_SYMBOL)
            .expect("SnowMudData runtime texture metadata");
        assert_eq!(snow.file, manifest.files.snow_mud_data);
        assert_eq!((snow.width, snow.height), (2, 1));
        assert_eq!(snow.byte_len, 8);
        assert_eq!(snow.format, RUNTIME_RGBA8_FORMAT);
        assert_eq!(snow.sampler, terrain_linear_wrap_sampler());
        assert_eq!(snow.source.provenance, "unit_test_snow_mud_capture");
        assert_eq!(
            fs::metadata(root.join(&snow.file))
                .expect("snow mud output")
                .len(),
            8
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn snow_mud_runtime_import_rejects_wrong_byte_count() {
        let root = temp_test_dir("bad_snow_mud_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        let source = root.join("bad_snow_mud.rgba");
        fs::write(&source, [1u8, 2, 3]).expect("write bad snow mud source");

        let options = ImportRuntimeTextureOptions {
            bundle: root.clone(),
            source_rgba: source,
            width: 1,
            height: 1,
            shader_symbol: SNOW_MUD_DATA_SYMBOL.to_string(),
            provenance: "unit_test_snow_mud_capture".to_string(),
            force: false,
        };
        let err = import_runtime_texture_input(&options)
            .expect_err("wrong SnowMudData byte count must block import");
        assert!(err.to_string().contains("expected 4"));
        assert!(!root.join(&manifest.files.snow_mud_data).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn generic_runtime_texture_import_accepts_known_pdxmap_symbol() {
        let root = temp_test_dir("province_secondary_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        let source = root.join("captured_province_secondary.rgba");
        fs::write(&source, [10u8, 20, 30, 40]).expect("write province secondary source");

        let options = ImportRuntimeTextureOptions {
            bundle: root.clone(),
            source_rgba: source,
            width: 1,
            height: 1,
            shader_symbol: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL.to_string(),
            provenance: "unit_test_secondary_color_capture".to_string(),
            force: false,
        };
        import_runtime_texture_input(&options)
            .expect("import ProvinceSecondaryColorMap runtime texture");

        let runtime: TerrainRuntimeInputs =
            serde_json::from_slice(&portable_test_fs_retry::read(root.join(&manifest.files.terrain_runtime)).unwrap())
                .expect("read runtime contract");
        let secondary = runtime
            .runtime_textures
            .get(PROVINCE_SECONDARY_COLOR_MAP_SYMBOL)
            .expect("ProvinceSecondaryColorMap runtime texture metadata");
        assert_eq!(secondary.file, manifest.files.province_secondary_color_map);
        assert_eq!(secondary.sampler, terrain_linear_wrap_sampler());
        assert_eq!(secondary.format, RUNTIME_RGBA8_FORMAT);
        assert_eq!(
            fs::metadata(root.join(&secondary.file))
                .expect("secondary output")
                .len(),
            4
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn runtime_texture_file_lookup_rejects_unknown_symbols() {
        let manifest = minimal_test_manifest();
        let err = runtime_texture_file_for_symbol(&manifest, "InventedRuntimeMap")
            .expect_err("unknown runtime texture symbols must fail closed");
        assert!(err
            .to_string()
            .contains("unsupported runtime texture symbol"));
    }

    #[test]
    fn terrain_scene_cb0_runtime_import_records_strict_shader_layout() {
        let root = temp_test_dir("scene_cb0_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        fs::write(
            root.join(
                manifest
                    .files
                    .map_sources
                    .get("gfx/FX/standardfuncsgfx.fxh")
                    .expect("standard functions source output"),
            ),
            minimal_standard_functions_source(),
        )
        .expect("write standard functions source");
        let source = root.join("captured_scene_cb0.bin");
        fs::write(&source, valid_scene_cb0_bytes()).expect("write scene cb0 source");
        let options = ImportRuntimeConstantBufferOptions {
            bundle: root.clone(),
            source_bin: source,
            shader_symbol: TERRAIN_SCENE_CB0_SYMBOL.to_string(),
            provenance: "unit_test_scene_cb0_capture".to_string(),
            force: false,
        };
        import_runtime_constant_buffer_input(&options).expect("import TerrainSceneCB0");

        let runtime: TerrainRuntimeInputs =
            serde_json::from_slice(&portable_test_fs_retry::read(root.join(&manifest.files.terrain_runtime)).unwrap())
                .expect("read runtime contract");
        let cb0 = runtime
            .runtime_constant_buffers
            .get(TERRAIN_SCENE_CB0_SYMBOL)
            .expect("TerrainSceneCB0 metadata");
        assert_eq!(cb0.file, manifest.files.terrain_scene_cb0);
        assert_eq!(cb0.byte_len, 416);
        assert_eq!(cb0.format, RUNTIME_CONSTANT_BUFFER_FORMAT);
        assert_eq!(cb0.bind_point, TERRAIN_SCENE_CB0_BIND_POINT);
        assert_eq!(cb0.layout_source, TERRAIN_SCENE_CB0_LAYOUT_SOURCE);
        assert_eq!(cb0.layout_byte_len, 416);
        assert_eq!(
            cb0.required_offsets["vGBCamDistOverride_GBOutlineCutoff"],
            128
        );
        assert_eq!(cb0.required_offsets["vCamPos"], 208);
        assert_eq!(cb0.required_offsets["vGlobalTime"], 236);
        assert_eq!(
            cb0.required_offsets["vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed"],
            240
        );
        assert_eq!(cb0.required_offsets["GB_TextureHeight"], 400);
        assert_eq!(cb0.source.provenance, "unit_test_scene_cb0_capture");
        assert_eq!(
            fs::metadata(root.join(&cb0.file))
                .expect("scene cb0 output")
                .len(),
            416
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_scene_cb0_runtime_import_rejects_short_buffer() {
        let root = temp_test_dir("bad_scene_cb0_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        fs::write(
            root.join(
                manifest
                    .files
                    .map_sources
                    .get("gfx/FX/standardfuncsgfx.fxh")
                    .expect("standard functions source output"),
            ),
            minimal_standard_functions_source(),
        )
        .expect("write standard functions source");
        let source = root.join("bad_scene_cb0.bin");
        fs::write(&source, [0u8; 16]).expect("write short scene cb0 source");
        let options = ImportRuntimeConstantBufferOptions {
            bundle: root.clone(),
            source_bin: source,
            shader_symbol: TERRAIN_SCENE_CB0_SYMBOL.to_string(),
            provenance: "unit_test_scene_cb0_capture".to_string(),
            force: false,
        };
        let err = import_runtime_constant_buffer_input(&options)
            .expect_err("short TerrainSceneCB0 must fail closed");
        assert!(err.to_string().contains("smaller than shader layout"));
        assert!(!root.join(&manifest.files.terrain_scene_cb0).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_vertex_buffer_runtime_import_records_strict_capture_metadata() {
        let root = temp_test_dir("terrain_vertex_buffer_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        let capture_json = write_test_terrain_vertex_buffer_capture(&root, 32);
        let options = ImportRuntimeVertexBufferOptions {
            bundle: root.clone(),
            capture_json,
            shader_symbol: TERRAIN_VERTEX_BUFFER_SYMBOL.to_string(),
            provenance: "unit_test_vertex_capture".to_string(),
            force: false,
        };
        import_runtime_vertex_buffer_input(&options).expect("import TerrainVertexBuffer");

        let runtime: TerrainRuntimeInputs =
            serde_json::from_slice(&portable_test_fs_retry::read(root.join(&manifest.files.terrain_runtime)).unwrap())
                .expect("read runtime contract");
        let vertex_buffer = runtime
            .runtime_vertex_buffers
            .get(TERRAIN_VERTEX_BUFFER_SYMBOL)
            .expect("TerrainVertexBuffer metadata");
        assert_eq!(vertex_buffer.file, "terrain_vertex_buffer_sample.bin");
        assert_eq!(vertex_buffer.source_byte_width, 4096);
        assert_eq!(vertex_buffer.captured_byte_len, 64);
        assert_eq!(vertex_buffer.format, RUNTIME_VERTEX_BUFFER_FORMAT);
        assert_eq!(vertex_buffer.stride, 32);
        assert_eq!(vertex_buffer.offset, 0);
        assert_eq!(vertex_buffer.draw_call, "DrawIndexed");
        assert_eq!(vertex_buffer.terrain_id_srv_slot, 5);
        assert_eq!(vertex_buffer.source.provenance, "unit_test_vertex_capture");
        assert_eq!(
            fs::metadata(root.join(&vertex_buffer.file))
                .expect("vertex buffer sample output")
                .len(),
            64
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_vertex_buffer_runtime_import_rejects_zero_stride() {
        let root = temp_test_dir("bad_terrain_vertex_buffer_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        write_minimal_runtime_import_bundle(&root);
        let capture_json = write_test_terrain_vertex_buffer_capture(&root, 0);
        let options = ImportRuntimeVertexBufferOptions {
            bundle: root.clone(),
            capture_json,
            shader_symbol: TERRAIN_VERTEX_BUFFER_SYMBOL.to_string(),
            provenance: "unit_test_vertex_capture".to_string(),
            force: false,
        };
        let err = import_runtime_vertex_buffer_input(&options)
            .expect_err("zero terrain vertex stride must fail closed");
        assert!(err.to_string().contains("stride 0"));
        assert!(!root.join("terrain_vertex_buffer_sample.bin").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_input_layout_runtime_import_records_strict_capture_metadata() {
        let root = temp_test_dir("terrain_input_layout_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let manifest = write_minimal_runtime_import_bundle(&root);
        let capture_json =
            write_test_terrain_input_layout_capture(&root, 8, valid_input_elements());
        let options = ImportRuntimeInputLayoutOptions {
            bundle: root.clone(),
            capture_json,
            shader_symbol: TERRAIN_INPUT_LAYOUT_SYMBOL.to_string(),
            provenance: "unit_test_input_layout_capture".to_string(),
            force: false,
        };
        import_runtime_input_layout(&options).expect("import TerrainInputLayout");

        let runtime: TerrainRuntimeInputs =
            serde_json::from_slice(&portable_test_fs_retry::read(root.join(&manifest.files.terrain_runtime)).unwrap())
                .expect("read runtime contract");
        let input_layout = runtime
            .runtime_input_layouts
            .get(TERRAIN_INPUT_LAYOUT_SYMBOL)
            .expect("TerrainInputLayout metadata");
        assert_eq!(input_layout.format, RUNTIME_INPUT_LAYOUT_FORMAT);
        assert_eq!(input_layout.element_count, 2);
        assert_eq!(input_layout.vertex_stride, 8);
        assert_eq!(input_layout.vertex_offset, 0);
        assert_eq!(input_layout.draw_call, "DrawIndexed");
        assert_eq!(input_layout.terrain_id_srv_slot, 5);
        assert_eq!(
            input_layout.source.provenance,
            "unit_test_input_layout_capture"
        );
        assert_eq!(input_layout.elements[0].semantic_name, "POSITION");
        assert_eq!(input_layout.elements[1].semantic_name, "TEXCOORD");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_input_layout_runtime_import_rejects_wrong_height_format() {
        let root = temp_test_dir("bad_terrain_input_layout_runtime_import");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        write_minimal_runtime_import_bundle(&root);
        let mut elements = valid_input_elements();
        elements[1].format = DXGI_FORMAT_R32G32B32A32_SINT;
        let capture_json = write_test_terrain_input_layout_capture(&root, 8, elements);
        let options = ImportRuntimeInputLayoutOptions {
            bundle: root.clone(),
            capture_json,
            shader_symbol: TERRAIN_INPUT_LAYOUT_SYMBOL.to_string(),
            provenance: "unit_test_input_layout_capture".to_string(),
            force: false,
        };
        let err = import_runtime_input_layout(&options)
            .expect_err("wrong TEXCOORD format must fail closed");
        assert!(err.to_string().contains("TEXCOORD0 format"));
        let _ = fs::remove_dir_all(&root);
    }
}
