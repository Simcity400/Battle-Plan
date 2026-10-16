//! Ordered loading of world textures, shaders and movement artwork.

use installed_content::{
    ResolvedLayers,
    image::{bmp, native::Decode},
    terrain::{DefaultMap, TerrainDefinitions},
};
use std::io;
use viewer_world::map::scene_content::MapAssets;
use {
    viewer_foundation::errors::invalid, viewer_foundation::map::camera::MapSize,
    viewer_foundation::map::textures::ColorSpace, viewer_foundation::map::textures::atlas_lookup,
    viewer_foundation::map::textures::expect_size, viewer_foundation::map::textures::load_image,
    viewer_foundation::map::textures::load_uploadable,
    viewer_foundation::map::textures::sampled_heights,
    viewer_foundation::map::textures::terrain_flag_mask,
    viewer_foundation::map::textures::terrain_id_map, viewer_world::map::post::PostAssets,
};

/// Reads and decodes the whole set, appending any limitation to `warnings`.
///
/// Everything below `map/` is covered content, so it goes through
/// [`ResolvedLayers::read`] and is hash-verified. The terrain, water and city-light textures
/// are artwork, read through [`ResolvedLayers::presentation`] so selected visual mods replace
/// them; the bitmaps that place terrain, heights, rivers and trees are recorded geometry.
#[allow(clippy::too_many_arguments)]
pub fn load(
    layers: &ResolvedLayers,
    defines: &installed_content::defines::Defines,
    map: MapSize,
    definitions: &TerrainDefinitions,
    default_map: &DefaultMap,
    warnings: &mut Vec<String>,
    quality: u8,
    decode: Decode,
) -> io::Result<MapAssets> {
    if quality > 2 {
        return Err(invalid("Invalid texture quality"));
    }
    viewer_world::map::shader_sources::check(layers, warnings)?;
    warnings.push("Selected arrow routes use viewer-local selection and animation; particle retained state follows the viewer's submitted world, arrows, labels and meshes.".into());
    let read = |path: &[u8]| layers.read_path(path);
    // Each texture is checked for the colour space the scene uploads it in, so artwork a
    // visual mod supplies that the upload would refuse is drawn from the recording instead.
    let texture = |name: &str, space: ColorSpace| {
        let path = format!("map/terrain/{name}");
        layers.artwork(&path, |stack| {
            load_uploadable(&path, &|request| stack.read_path(request), decode, space)
        })
    };
    let terrain = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.terrain))?)
        .map_err(|error| invalid(format!("map/{}: {error}", default_map.terrain)))?;
    let heightmap = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.heightmap))?)
        .map_err(|error| invalid(format!("map/{}: {error}", default_map.heightmap)))?;
    // Load the sampled heightmap as a texture file.
    let height_texture =
        sampled_heights(&load_image("map/heightmap.bmp", &read, decode)?, &heightmap);
    let world_normal = installed_content::terrain::read_world_normal(layers, decode)?;
    // rivers
    let rivers = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.rivers))?)
        .map_err(|error| invalid(format!("map/{}: {error}", default_map.rivers)))?;
    // The tree_definition names the tree bitmap. Draw an instance on every nonzero pixel, regardless
    // of palette index; automatic terrain-assignment indices do not restrict tree placement.
    let trees = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.tree_definition))?)
        .map_err(|error| invalid(format!("map/{}: {error}", default_map.tree_definition)))?;
    expect_size(
        "map/terrain.bmp",
        terrain.width,
        terrain.height,
        [map.width, map.height],
    )?;
    expect_size(
        "map/heightmap.bmp",
        heightmap.width,
        heightmap.height,
        [map.width, map.height],
    )?;
    // rivers
    expect_size(
        "map/rivers.bmp",
        rivers.width,
        rivers.height,
        [map.width, map.height],
    )?;
    let atlas_of_palette = atlas_lookup(definitions, &terrain, warnings);
    let terrain_ids = terrain_id_map(
        terrain.width,
        terrain.height,
        &terrain.indices,
        &atlas_of_palette,
    )?;
    let reflection = texture("reflection.dds", ColorSpace::Srgb)?;
    if reflection.faces == 1 {
        // Retain DDS view type; do not convert strips into cube faces.
        warnings.push("1 2D reflection texture (map/terrain/reflection.dds): cube samples reproduce the native hardware GPU's zero result; native D3D11 software-driver results are device-dependent.".into());
    }
    Ok(MapAssets {
        map,
        mesh_limit: viewer_world::map::mesh_budget::Budget::read(defines)?,
        atlas: texture(&format!("atlas{quality}.dds"), ColorSpace::Srgb)?,
        atlas_normal: texture(&format!("atlas_normal{quality}.dds"), ColorSpace::Linear)?,
        colormap: texture("colormap_rgb_cityemissivemask_a.dds", ColorSpace::Srgb)?,
        city_lights: texture(
            &format!("citylights_rgb_snowmask_a_{quality}.dds"),
            ColorSpace::Srgb,
        )?,
        snow: texture("snow_normal_rgb_diffuse_a.dds", ColorSpace::Linear)?,
        mud_diffuse: texture(
            &format!("mud_diffuse_rgb_gloss_a_{quality}.dds"),
            ColorSpace::Srgb,
        )?,
        mud_normal: texture(
            &format!("mud_normal_rgb_spec_a_{quality}.dds"),
            ColorSpace::Linear,
        )?,
        colormap_water: texture(&format!("colormap_water_{quality}.dds"), ColorSpace::Srgb)?,
        underwater: texture(
            &format!("underwater_terrain_{quality}.dds"),
            ColorSpace::Srgb,
        )?,
        lean1: texture("lean1.dds", ColorSpace::Linear)?,
        lean2: texture("lean2.dds", ColorSpace::Linear)?,
        water_specular: texture("fow_rgb_waterspec_a.dds", ColorSpace::Linear)?,
        naval_dominance: texture("naval_dominance_fx.dds", ColorSpace::Srgb)?,
        terrain_shader: viewer_world::map::terrain_shader::load(
            layers,
            [map.width, map.height],
            decode,
        )?,
        water_shader: viewer_world::map::water_shader::load(
            layers,
            [map.width, map.height],
            decode,
        )?,
        sky_shader: viewer_world::map::sky_shader::load(layers, [map.width, map.height], decode)?,
        name_shader: viewer_world::map::name_shader::load(layers, [map.width, map.height], decode)?,
        arrow_shader: viewer_world::map::arrow_shader::load(
            layers,
            [map.width, map.height],
            decode,
            "map arrows",
        )?,
        movement_arrows: viewer_world::map::movement_arrows::Assets::load(
            layers,
            defines,
            [map.width, map.height],
            decode,
        )?,
        order_label_shader: (
            viewer_world::map::arrow_shader::load(
                layers,
                [map.width, map.height],
                decode,
                "order labels",
            )?,
            viewer_foundation::map::ribbon_content::ArrowTypes::parse(
                &layers.read_uncovered(viewer_foundation::map::ribbon_content::MAPARROWS)?,
            )?
            .text_technique,
        ),
        tree_shader: viewer_world::map::tree_caster::load(layers, [map.width, map.height], decode)?,
        reflection,
        ice_diffuse: texture("ice_diffuse.dds", ColorSpace::Linear)?,
        ice_noise: texture(&format!("ice_noise_{quality}.dds"), ColorSpace::Linear)?,
        fow_noise: texture(&format!("fow_noise_{quality}.dds"), ColorSpace::Linear)?,
        world_normal,
        heightmap,
        height_texture,
        terrain_ids,
        // colour
        post: PostAssets::load(layers, defines, map, decode)?,
        shadow_blur: viewer_world::map::shadow_blur::load(layers, [map.width, map.height], decode)?,
        // rivers
        rivers,
        // trees
        trees,
        // weather
        perm_snow: terrain_flag_mask(definitions, &terrain, |kind| kind.perm_snow).into(),
        // close zoom
        spawn_city: terrain_flag_mask(definitions, &terrain, |kind| kind.spawn_city),
        sight: Default::default(),
    })
}
