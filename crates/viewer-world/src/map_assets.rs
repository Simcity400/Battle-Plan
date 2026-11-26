use content_reader_layers::ResolvedLayers;
use content_reader_key_names::terrain::{DefaultMap, TerrainDefinitions};
use content_reader_image::native::Decode;
#[cfg(test)]
use content_reader_image::bmp;
use std::{io, sync::Arc};
use crate::scene_content::{LoadMasks, MapAssets, SceneSources};
use viewer_runtime::errors::invalid;
use viewer_gpu::{
    camera::MapSize,
    textures::{atlas_lookup, expect_size, load_image, load_map_texture, load_uploadable, sampled_heights, terrain_flag_mask, terrain_id_map, ColorSpace, MapTexture},
};
use viewer_scene_base::post::PostAssets;

#[expect(clippy::too_many_arguments, reason = "The loader needs recorded geometry, presentation artwork, and decode settings together.")]
pub fn load_with_products(
    products: Option<&map_content::loader::Loader>,
    layers: &ResolvedLayers,
    defines: &content_reader_defines::defines::Defines,
    art_defines: &content_reader_defines::defines::Defines,
    map: MapSize,
    definitions: &TerrainDefinitions,
    default_map: &DefaultMap,
    warnings: &mut Vec<String>,
    quality: u8,
    decode: Decode,
) -> io::Result<(MapAssets, SceneSources, LoadMasks)> {
    if quality > 2 {
        return Err(invalid("Invalid texture quality"));
    }
    warnings.extend(layers.artwork("shader source audit", |stack| {
        let mut lines = Vec::new();
        viewer_scene_base::shader_sources::check(stack, &mut lines).map(|()| lines)
    })?);
    warnings.push("Selected arrow routes use viewer-local selection and animation; particle retained state follows the viewer's submitted world, arrows, labels and meshes.".into());
    let read = |path: &[u8]| layers.read_path(path);
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
    let mut texture_warnings = Vec::new();
    let mut texture_with_status = |name: &str, space: ColorSpace| {
        let path = format!("map/terrain/{name}");
        layers.artwork(&path, |stack| {
            if stack.shows_visual_mods() {
                load_uploadable(&path, &|request| stack.read_path(request), decode, space).map(
                    |texture| MapTexture {
                        texture,
                        unbound: false,
                    },
                )
            } else {
                load_map_texture(
                    &path,
                    &|request| stack.read_path(request),
                    decode,
                    space,
                    &mut texture_warnings,
                )
            }
        })
    };
    let terrain = content_reader_map::terrain_images::read_indexed_image(layers, &format!("map/{}", default_map.terrain))?;
    let heightmap = match products {
        Some(products) => products.heightmap(default_map)?,
        None => content_reader_map::terrain_images::read_indexed_image(layers, &format!("map/{}", default_map.heightmap))?,
    };
    let height_texture =
        sampled_heights(&load_image("map/heightmap.bmp", &read, decode)?, &heightmap);
    let world_normal = world_normal(layers, decode)?;
    let rivers = content_reader_map::terrain_images::read_indexed_image(layers, &format!("map/{}", default_map.rivers))?;
    let trees = content_reader_map::terrain_images::read_indexed_image(layers, &format!("map/{}", default_map.tree_definition))?;
    expect_size(
        "map/terrain.bmp",
        terrain.width,
        terrain.height,
        [map.width, map.height],
    )?;
    match products {
        Some(products) => products.validate_heightmap(&heightmap, [map.width, map.height])?,
        None => expect_size(
            "map/heightmap.bmp",
            heightmap.width,
            heightmap.height,
            [map.width, map.height],
        )?,
    }
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
    let reflection = texture_with_status("reflection.dds", ColorSpace::Srgb)?;
    if !reflection.unbound && reflection.texture.faces == 1 {
        warnings.push("1 2D reflection texture (map/terrain/reflection.dds): cube samples reproduce the native hardware GPU's zero result; native D3D11 software-driver results are device-dependent.".into());
    }
    let reflection = reflection.texture;
    let mut texture = |name: &str, space: ColorSpace| {
        texture_with_status(name, space).map(|loaded| loaded.texture)
    };
    let (arrow_shader, movement_arrows, order_label_shader) =
        layers.artwork("map arrows", |stack| {
            Ok((
                viewer_aircraft::arrow_shader::load(stack, size, decode, "map arrows")?,
                crate::movement_arrows::Assets::load(
                    stack,
                    defines_for(stack),
                    size,
                    decode,
                )?,
                (
                    viewer_aircraft::arrow_shader::load(stack, size, decode, "order labels")?,
                    viewer_gpu::ribbon_content::ArrowTypes::parse(
                        &stack.read_uncovered(viewer_gpu::ribbon_content::MAPARROWS)?,
                    )?
                    .text_technique,
                ),
            ))
        })?;
    let sources = SceneSources {
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
        reflection,
        ice_diffuse: texture("ice_diffuse.dds", ColorSpace::Linear)?,
        ice_noise: texture(&format!("ice_noise_{quality}.dds"), ColorSpace::Linear)?,
        fow_noise: texture(&format!("fow_noise_{quality}.dds"), ColorSpace::Linear)?,
        world_normal,
        terrain_ids,
        icon_images: Vec::new(),
    };
    let assets = MapAssets {
        map,
        mesh_limit: graphics_define(layers, "mesh budget", art_defines, defines, |defines| {
            viewer_scene_base::mesh_budget::Budget::read(defines)
        })?,
        terrain_shader: shader("terrain shader", viewer_terrain::terrain_shader::load)?,
        water_shader: shader("water shader", viewer_terrain::water_shader::load)?,
        sky_shader: shader("sky shader", viewer_terrain::sky_shader::load)?,
        name_shader: shader("map name shader", viewer_map_text::name_shader::load)?,
        arrow_shader,
        movement_arrows,
        order_label_shader,
        tree_shader: shader("tree shadow shader", viewer_scene_base::tree_caster::load)?,
        heightmap: Arc::new(heightmap),
        height_texture,
        post: layers.artwork("post-processing", |stack| {
            PostAssets::load(stack, defines_for(stack), map, decode)
        })?,
        shadow_blur: shader("shadow blur shader", viewer_scene_base::shadow_blur::load)?,
        perm_snow: map_snow::permanent_mask(definitions, &terrain).into(),
        spawn_city: Arc::new(terrain_flag_mask(definitions, &terrain, |kind| {
            kind.spawn_city
        })),
        sight: Default::default(),
    };
    warnings.extend(texture_warnings);
    Ok((assets, sources, LoadMasks { rivers, trees }))
}

fn world_normal(
    layers: &ResolvedLayers,
    decode: Decode,
) -> io::Result<content_reader_image::SharedRgba8Image> {
    layers.artwork("map/world_normal.bmp", |stack| {
        content_reader_map::terrain_images::read_world_normal(stack, decode)
    })
}

pub fn graphics_define<T>(
    layers: &ResolvedLayers,
    name: &str,
    art_defines: &content_reader_defines::defines::Defines,
    defines: &content_reader_defines::defines::Defines,
    read: impl Fn(&content_reader_defines::defines::Defines) -> io::Result<T>,
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

    #[test]
    fn a_mods_world_normal_map_is_drawn() {
        content_reader_script::test_game::enter();
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
        let options = content_reader_layers::ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let normal = |name: &str| {
            let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
            let selected = [content_reader_layers::content::visual::Choice {
                id: format!("mod/{name}.mod"),
                name: name.into(),
            }];
            content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
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
        let options = content_reader_layers::ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let mut layers = ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
        let read = |layers: &ResolvedLayers, name: &str| {
            let defines = content_reader_defines::defines::read(layers).unwrap();
            let art =
                content_reader_defines::defines::read(layers.presentation_defines().unwrap_or(layers))
                    .unwrap();
            graphics_define(layers, name, &art, &defines, |defines| {
                defines.number("NGraphics", name)
            })
            .unwrap()
        };
        assert_eq!(read(&layers, "SIZE"), 2.0);
        let selected = [content_reader_layers::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
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
