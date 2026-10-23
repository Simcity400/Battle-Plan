//! Ordered loading of world textures, shaders and movement artwork.

use installed_content_core::{ResolvedLayers, terrain::DefaultMap, terrain::TerrainDefinitions};
use installed_content_image::{bmp, native::Decode};
use std::{io, sync::Arc};
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
    defines: &installed_content_core::defines::Defines,
    art_defines: &installed_content_core::defines::Defines,
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
    // The audit names the shaders the map draws: the visual mods' while they show.
    warnings.extend(layers.artwork("shader source audit", |stack| {
        let mut lines = Vec::new();
        viewer_world::map::shader_sources::check(stack, &mut lines).map(|()| lines)
    })?);
    warnings.push("Selected arrow routes use viewer-local selection and animation; particle retained state follows the viewer's submitted world, arrows, labels and meshes.".into());
    let read = |path: &[u8]| layers.read_path(path);
    // Shaders, arrows and post-processing come from the presentation stack, each falling back to
    // the recording by name (`ResolvedLayers::artwork`), with the graphics defines while visual
    // mods show and the recorded defines when it falls back.
    let defines_for = |stack: &ResolvedLayers| {
        if stack.shows_visual_mods() {
            art_defines
        } else {
            defines
        }
    };
    let size = [map.width, map.height];
    let shader = |name: &str, load: fn(&ResolvedLayers, [u32; 2], Decode) -> io::Result<_>| {
        layers.artwork(name, |stack| load(stack, size, decode))
    };
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
    let world_normal = world_normal(layers, decode)?;
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
    // The arrow effects, movement arrows and order labels all follow `maparrows.txt`, so they
    // come from one stack together.
    let (arrow_shader, movement_arrows, order_label_shader) =
        layers.artwork("map arrows", |stack| {
            Ok((
                viewer_world::map::arrow_shader::load(stack, size, decode, "map arrows")?,
                viewer_world::map::movement_arrows::Assets::load(
                    stack,
                    defines_for(stack),
                    size,
                    decode,
                )?,
                (
                    viewer_world::map::arrow_shader::load(stack, size, decode, "order labels")?,
                    viewer_foundation::map::ribbon_content::ArrowTypes::parse(
                        &stack.read_uncovered(viewer_foundation::map::ribbon_content::MAPARROWS)?,
                    )?
                    .text_technique,
                ),
            ))
        })?;
    Ok(MapAssets {
        map,
        mesh_limit: graphics_define(layers, "mesh budget", art_defines, defines, |defines| {
            viewer_world::map::mesh_budget::Budget::read(defines)
        })?,
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
        terrain_shader: shader("terrain shader", viewer_world::map::terrain_shader::load)?,
        water_shader: shader("water shader", viewer_world::map::water_shader::load)?,
        sky_shader: shader("sky shader", viewer_world::map::sky_shader::load)?,
        name_shader: shader("map name shader", viewer_world::map::name_shader::load)?,
        arrow_shader,
        movement_arrows,
        order_label_shader,
        tree_shader: shader("tree shadow shader", viewer_world::map::tree_caster::load)?,
        reflection,
        ice_diffuse: texture("ice_diffuse.dds", ColorSpace::Linear)?,
        ice_noise: texture(&format!("ice_noise_{quality}.dds"), ColorSpace::Linear)?,
        fow_noise: texture(&format!("fow_noise_{quality}.dds"), ColorSpace::Linear)?,
        world_normal,
        heightmap: Arc::new(heightmap),
        height_texture,
        terrain_ids,
        // colour
        post: layers.artwork("post-processing", |stack| {
            PostAssets::load(stack, defines_for(stack), map, decode)
        })?,
        shadow_blur: shader("shadow blur shader", viewer_world::map::shadow_blur::load)?,
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

/// The world normal map. It is artwork, so a visual mod's is drawn (in any form the game reads),
/// and the recording's, named, when the mod's cannot be.
fn world_normal(
    layers: &ResolvedLayers,
    decode: Decode,
) -> io::Result<installed_content_image::Rgba8Image> {
    layers.artwork("map/world_normal.bmp", |stack| {
        installed_content_core::terrain::read_world_normal(stack, decode)
    })
}

/// A value read from the graphics defines, or, when the visual mods' defines cannot give it, from
/// the recorded defines with the refusal named. Without visual mods both are the recorded defines.
pub fn graphics_define<T>(
    layers: &ResolvedLayers,
    name: &str,
    art_defines: &installed_content_core::defines::Defines,
    defines: &installed_content_core::defines::Defines,
    read: impl Fn(&installed_content_core::defines::Defines) -> io::Result<T>,
) -> io::Result<T> {
    if layers.presentation_defines().is_none() {
        return read(defines);
    }
    read(art_defines).or_else(|error| {
        let recorded = read(defines)?;
        layers.note_overlay_refusal(format!("graphics defines for the {name} ({error})"));
        Ok(recorded)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 24-bit BMP of one pixel, blue, green and red.
    fn pixel(bgr: [u8; 3]) -> Vec<u8> {
        let mut bytes = b"BM".to_vec();
        bytes.extend(58_u32.to_le_bytes());
        bytes.extend([0; 4]);
        bytes.extend(54_u32.to_le_bytes());
        bytes.extend(40_u32.to_le_bytes());
        bytes.extend(1_i32.to_le_bytes());
        bytes.extend(1_i32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(24_u16.to_le_bytes());
        bytes.extend(0_u32.to_le_bytes());
        bytes.extend(4_u32.to_le_bytes());
        bytes.extend([0; 16]);
        bytes.extend(bgr);
        bytes.push(0);
        bytes
    }

    /// The world normal map a visual mod supplies is drawn; one of its that does not read is
    /// drawn from the recording, named.
    #[test]
    fn a_mods_world_normal_map_is_drawn() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_world_normal_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |relative: &str, bytes: &[u8]| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        let (recorded, shown) = (pixel([1, 2, 3]), pixel([30, 20, 10]));
        write("base/map/world_normal.bmp", &recorded);
        write("art/map/world_normal.bmp", &shown);
        write("broken/map/world_normal.bmp", b"not a bitmap");
        for name in ["art", "broken"] {
            let folder = root.join(name).display().to_string().replace('\\', "/");
            write(
                &format!("user/mod/{name}.mod"),
                format!("name=\"{name}\"\npath=\"{folder}\"\n").as_bytes(),
            );
        }
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        let options = installed_content_core::ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let normal = |name: &str| {
            let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
            let selected = [installed_content_core::content::visual::Choice {
                id: format!("mod/{name}.mod"),
                name: name.into(),
            }];
            installed_content_core::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
                .unwrap();
            let normal = world_normal(&layers, Decode::default()).unwrap();
            (normal, layers.read_warnings().overlay_refusals())
        };
        let (art, art_refusals) = normal("art");
        let (broken, broken_refusals) = normal("broken");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(art, bmp::decode_rgb(&shown).unwrap());
        assert!(art_refusals.is_none(), "{art_refusals:?}");
        assert_eq!(broken, bmp::decode_rgb(&recorded).unwrap());
        let refusals = broken_refusals.unwrap();
        assert!(refusals.contains("map/world_normal.bmp ("), "{refusals}");
    }

    /// A graphics define the visual mods' defines cannot give comes from the recorded defines,
    /// the refusal named; one they give is theirs, and without visual mods it is the recording's.
    #[test]
    fn graphics_defines_a_mod_breaks_fall_back_to_the_recording() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_graphics_define_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |relative: &str, bytes: &str| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write(
            "base/common/defines/00_defines.lua",
            "NDefines={NGraphics={SIZE=2,SCALE=3}}",
        );
        write(
            "art/common/defines/00_defines.lua",
            "NDefines={NGraphics={SIZE=7,SCALE=\"large\"}}",
        );
        let art = root.join("art").display().to_string().replace('\\', "/");
        write(
            "user/mod/art.mod",
            &format!("name=\"Art\"\npath=\"{art}\"\n"),
        );
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        let options = installed_content_core::ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
        let read = |layers: &ResolvedLayers, name: &str| {
            let defines = installed_content_core::defines::read(layers).unwrap();
            let art =
                installed_content_core::defines::read(layers.presentation_defines().unwrap_or(layers))
                    .unwrap();
            graphics_define(layers, name, &art, &defines, |defines| {
                defines.number("NGraphics", name)
            })
            .unwrap()
        };
        assert_eq!(read(&layers, "SIZE"), 2.0);
        let selected = [installed_content_core::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        installed_content_core::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
            .unwrap();
        assert!(layers.presentation_defines().is_some());
        assert!(layers.read_warnings().overlay_refusals().is_none());
        assert_eq!(read(&layers, "SIZE"), 7.0);
        assert_eq!(read(&layers, "SCALE"), 3.0);
        let refusals = layers.read_warnings().overlay_refusals().unwrap();
        assert!(
            refusals.contains("graphics defines for the SCALE"),
            "{refusals}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
