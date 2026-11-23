mod buildings;
mod convoys;
mod intel_icons;
mod land_markers;
mod map_text;
mod mode_fills;
mod names;
mod naval_counters;
mod naval_markers;
mod occupation_marks;
mod orders;
mod rails;
mod resource_icons;
mod ships;
mod trains;
mod units;
mod units_models;
mod aircraft;

use viewer_layer_core::{layers::Layer, vision};
use viewer_layers_counters::counters;
use viewer_layers_infrastructure::{intel, radar, snow, weather};
use viewer_layers_political::scripts;
use std::sync::Arc;
use viewer_gpu::{camera::MapSize, textures::IntelCells};

pub fn vision(document: &crate::document::Document) -> Box<dyn Layer> {
    Box::new(vision::Vision::new(Arc::new(vision::Content::new(document.content.products.fog.as_ref().clone()))))
}

pub fn weather(document: &crate::document::Document) -> Box<dyn Layer> {
    Box::new(weather::Weather::for_branch(document.playback.branch()))
}

pub fn snow(document: &crate::document::Document) -> Box<dyn Layer> {
    let size = viewer_gpu::camera::MapSize::of(&document.content.map);
    let defines = document.content.map_defines;
    let content = document.content.products.snow_content().map(|content| Arc::new(snow::Content::new(content.cells.clone(), content.winter.clone(), size, defines))).map_err(|error| error.to_string());
    Box::new(snow::Snow::new(content))
}

pub fn intel(document: &crate::document::Document) -> Box<dyn Layer> {
    let map = &document.content.map;
    let largest = map.definitions.last_key_value().map_or(0, |(&id, _)| id).max(map.province_ids.iter().copied().max().unwrap_or(0));
    let cells = IntelCells::new(map).map(|cells| Arc::new(intel::Content::new(cells, largest))).map_err(|error| error.to_string());
    Box::new(intel::Intel::new(cells))
}

pub fn radar(_document: &crate::document::Document) -> Box<dyn Layer> {
    Box::new(radar::Radar::new())
}

pub fn counters(document: &crate::document::Document) -> counters::Counters {
    let map = (MapSize::of(&document.content.map), document.content.map_defines);
    counters::Counters::new(Arc::new(counters::Content::new(Arc::clone(&document.content.counters), document.content.movement.clone(), map, document.content.tables)))
}

pub(crate) fn scripts(content: &crate::document::Content) -> scripts::Scripts {
    scripts::Scripts::new(scripts::Held::new(
        Arc::clone(&content.icons.raids),
        Arc::clone(&content.products.triggers),
        content.products.audio.clone(),
        content.tables,
    ))
}
