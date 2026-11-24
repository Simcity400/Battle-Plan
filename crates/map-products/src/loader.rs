use crate::{ProductContent, ProductInputs};
use content_reader_layers::ResolvedLayers;
pub use map_content::loader::*;
use std::{io, path::Path, sync::Arc};
use viewer_gpu::{province_geometry::ProvinceGeometry, province_map::{HeightField, ProvinceMap}};

pub fn load(layers: &ResolvedLayers, user: &Path, language: &str) -> io::Result<ProductContent> {
    let loader = Loader::new(layers);
    let map = Arc::new(loader.map()?);
    let (_, default_map) = loader.terrain()?;
    let defines = loader.defines()?;
    let audio = loader.audio(&defines, user, &mut Vec::new());
    let heightmap = loader.heightmap(&default_map)?;
    loader.validate_heightmap(&heightmap, [map.width, map.height])?;
    let combat = loader.combat(layers)?;
    let mut triggers = TriggerLoader::new(layers, Some(&loader)).load(&defines, &map)?;
    let localisation = layers.artwork("map names", |stack| loader.localisation(stack, language))?;
    let anchors = loader.anchors(layers)?;
    let fog = loader.fog(layers, &map, &defines);
    let buildings = loader.buildings()?;
    let (ideologies, _) = loader.ideologies()?;
    let (resources, _) = loader.resources()?;
    triggers.triggers = Arc::new(combat.names(&buildings, &ideologies, &resources));
    triggers.character_traits = Some((combat.traits, combat.country_traits));
    triggers.ideologies = Some(ideologies);
    triggers.terrain = loader.movement(&defines).ok().map(Arc::new);
    let geometry = ProvinceGeometry::build(&map);
    let heights = HeightField {
        width: heightmap.width,
        height: heightmap.height,
        values: &heightmap.indices,
    };
    let (default_anchors, _) = seed_default_anchors(
        map.definitions.keys().copied(),
        &anchors.positions,
        |province| geometry.centroid(province),
        &heights,
    );
    Ok(ProductContent::from_inputs(
        loader.reads.clone(),
        ProductInputs {
            map: Arc::clone(&map),
            fog,
            anchors: Arc::new(anchors.positions),
            default_anchors: Arc::new(default_anchors),
            bounds: Arc::new(viewer_gpu::province_admission::province_boxes(
                &ProvinceMap {
                    width: map.width,
                    height: map.height,
                    ids: &map.province_ids,
                },
            )),
            localisation,
            map_text: viewer_map_text::animated_map_text::events::Content::new(&buildings),
            audio: audio.map(|source| Arc::clone(&source.content)),
            triggers: Arc::new(triggers),
        },
    )
    .with_snow_loader(loader))
}
