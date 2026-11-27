use super::{Kind, Log};
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::MapData;
use content_reader_key_names::terrain::{DefaultMap, TerrainDefinitions};
use content_reader_image::native::Decode;
use content_reader_regions::regions::StrategicRegion;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_gpu::camera::MapSize;
use viewer_borders::border_pass::BorderAssets;
use viewer_rail::railways::RailAssets;
use viewer_terrain::rivers::RiverAssets;

pub(super) fn read(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    terrain: Option<&(TerrainDefinitions, DefaultMap)>,
    map: Option<&MapData>,
    regions: Option<&BTreeMap<u32, StrategicRegion>>,
    decode: Decode,
) {
    let (Some(defines), Some((terrain, default_map)), Some(map)) = (defines, terrain, map) else {
        for quality in 0..=2 {
            for kind in ["world", "river", "border"] {
                log.not_run(
                    &[&format!("viewer.{kind}_assets.quality{quality}")],
                    "defines, terrain and map",
                );
            }
        }
        log.not_run(
            &[
                "viewer.tree_assets",
                "viewer.city_assets",
                "viewer.rail_assets",
                "viewer.weather_assets",
                "viewer.map_shader_sources",
                "viewer.map_shader_load",
                "viewer.map_shader_compile",
            ],
            "defines, terrain and map",
        );
        return;
    };
    let size = [map.width, map.height];
    let mut warnings = Vec::new();
    for quality in 0..=2 {
        let reader = format!("viewer.world_assets.quality{quality}");
        let mut loaded = None;
        log.read(&reader, || {
            viewer_world::map_assets::load_with_products(
                None,
                layers,
                defines,
                defines,
                MapSize {
                    width: map.width,
                    height: map.height,
                },
                terrain,
                default_map,
                &mut warnings,
                quality,
                decode,
            )
            .and_then(|(assets, sources, masks)| {
                let (_, terrain_warnings) = sources.reader.read(&assets.heightmap)?;
                warnings.extend(terrain_warnings);
                let summary = (assets.map.width, assets.map.height, assets.post.luts.len());
                if quality == 0 {
                    loaded = Some((assets, masks));
                }
                Ok(summary)
            })
        });
        if let Some((assets, masks)) = loaded {
            trees_and_cities(log, layers, defines, map, (&assets, &masks), decode);
        } else if quality == 0 {
            log.not_run(&["viewer.tree_assets", "viewer.city_assets"], &reader);
        }
        log.read(&format!("viewer.river_assets.quality{quality}"), || {
            RiverAssets::load(layers, size, quality, decode).map(|_| ())
        });
        log.read(&format!("viewer.border_assets.quality{quality}"), || {
            BorderAssets::load(layers, defines, size, quality, decode).map(|_| ())
        });
    }
    log.notes(
        "viewer.world_assets",
        Kind::Warning,
        warnings.iter().collect::<BTreeSet<_>>(),
    );
    log.read("viewer.rail_assets", || {
        RailAssets::load(layers, size, decode).map(|_| ())
    });
    match regions {
        Some(regions) => {
            let region_provinces = viewer_weather_effects::weather_effects::RegionProvinces::new(
                regions,
                |province| map.definitions.contains_key(&province),
            );
            if let Some(withheld) = log.read("viewer.weather_assets", || {
                viewer_weather_effects::weather_effects::Assets::load(
                    layers,
                    defines,
                    size,
                    region_provinces,
                    decode,
                )
                .map(|assets| assets.withheld.clone())
            }) {
                log.notes("viewer.weather_assets", Kind::Refused, &withheld);
            }
        }
        None => log.not_run(&["viewer.weather_assets"], "strategic regions"),
    }
    shaders(log, layers, size, decode);
}

fn trees_and_cities(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: &Defines,
    map: &MapData,
    (assets, masks): (&viewer_world::scene_content::MapAssets, &viewer_world::scene_content::LoadMasks),
    decode: Decode,
) {
    let palettes: Vec<_> = masks
        .trees
        .indices
        .iter()
        .copied()
        .filter(|index| *index != 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut warnings = Vec::new();
    log.read("viewer.tree_assets", || {
        viewer_terrain::trees::TreeAssets::load(
            layers,
            decode,
            &palettes,
            &masks.rivers,
            &mut warnings,
        )
        .map(|_| ())
    });
    log.notes("viewer.tree_assets", Kind::Warning, &warnings);
    let mut warnings = Vec::new();
    log.read("viewer.city_assets", || {
        let (cities, styles) = content_reader_map::cities::read(layers)?;
        let plan = viewer_buildings::sprawl::generate(
            &cities,
            &styles,
            &assets.spawn_city,
            &assets.height_field(),
            &map.province_ids,
            &masks.rivers,
        )?;
        viewer_buildings::sprawl::Sprawl::load(layers, decode, &plan, defines, &mut warnings)
            .map(|_| ())
    });
    log.notes("viewer.city_assets", Kind::Warning, &warnings);
}

fn shaders(log: &mut Log, layers: &ResolvedLayers, size: [u32; 2], decode: Decode) {
    let Some(passes) = log.read("viewer.map_shader_sources", || {
        viewer_scene_base::shader_sources::passes(layers)
    }) else {
        return;
    };
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, size, decode);
    let paths: BTreeSet<_> = passes.into_iter().map(|(_, path)| path).collect();
    for path in paths {
        let Some(effect) = log.read("viewer.map_shader_load", || {
            library
                .load(path.as_bytes(), &defines)
                .map_err(io::Error::other)
        }) else {
            continue;
        };
        for (index, technique) in effect.source.techniques.iter().enumerate() {
            let subject = format!("{path}: {}", technique.name);
            log.read(&format!("viewer.map_shader_compile.{subject}"), || {
                effect.compiled(index).map(|_| ()).map_err(io::Error::other)
            });
        }
    }
    log.note("viewer.map_shader_compile", Kind::Offline,
        "Compiled authored map techniques with the renderer globals; runtime material defines, GPU bindings and recorded draw selection require a rendered campaign frame.");
}
