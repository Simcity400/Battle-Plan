
use super::Compared;
use crate::{document::Document, playhead_host::mirror::Mirrors};
use viewer_layer_core::layers::Layer;
use viewer_layers_infrastructure::buildings::{Buildings, BuildingsFed, Catalogue, Inspected};
use serde_json::{Value, json};
use std::{io, sync::Arc};
use viewer_foundation::perspective::Choice;

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

fn of(layer: &dyn Layer) -> &BuildingsFed {
    layer.any().downcast_ref::<BuildingsFed>().expect("the buildings line's layer")
}

fn inspected(layer: &BuildingsFed) -> io::Result<Inspected> {
    layer.inspect().ok_or_else(|| io::Error::other("the buildings' worker stopped"))
}

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    let catalogue: Arc<dyn Catalogue> = Arc::clone(&document.content) as _;
    Box::new(BuildingsFed::new(Buildings::new(catalogue)))
}

pub(super) fn give(layer: &dyn Layer, document: &Document, choice: &Choice) -> io::Result<()> {
    if let Some(buildings) = layer.any().downcast_ref::<BuildingsFed>() {
        buildings.set_vision(choice.clone(), document.vision(choice)?);
    }
    Ok(())
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [rows, slots, provinces] = of(layer).marked();
    vec![("building rows", rows as u64), ("slots", slots as u64), ("city provinces", provinces as u64)]
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let choice = of(layer).perspective().cloned().unwrap_or_default();
    let layer = &inspected(of(layer))?;
    let mut compared = Compared::default();
    let (product, _) = match document.map_building_field(&choice) {
        Ok(today) => today,
        Err(error) => {
            compared.entries = 1;
            if layer.refused.is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            compared.notes = json!({"today_refused": error.to_string(), "layer_refused": layer.refused});
            return Ok(compared);
        }
    };
    if let Some(why) = &layer.refused {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
        return Ok(compared);
    }
    let assets = &document.content.map_buildings;
    let today: Vec<_> = product.slots.iter().flat_map(|chunk| chunk.iter()).collect();
    let ours = &layer.slots;
    compared.entries += today.len().max(ours.len()) as u64;
    if today.len() != ours.len() {
        note(&mut compared, format!("today {} slots, the layer {}", today.len(), ours.len()));
    }
    let mut placed = 0;
    for (slot, mine) in today.iter().zip(ours) {
        let mine = assets.display_slot(mine)?;
        placed += usize::from(slot.entity.is_some());
        if **slot != mine {
            note(&mut compared, format!("slot {}: today {slot:?}, the layer {mine:?}", slot.id));
        }
    }
    let city = &product.city_damage;
    let health: Vec<(u32, Option<i64>)> = city.health.iter().map(|(&province, &health)| (province, health)).collect();
    let mine = &layer.health;
    compared.entries += health.len().max(mine.len()) as u64 + 1;
    if health != *mine {
        let at = health.iter().zip(mine).position(|(a, b)| a != b).unwrap_or(health.len().min(mine.len()));
        note(&mut compared, format!("city damage: today {} provinces, the layer {}; first apart at {at}: today {:?}, the layer {:?}", health.len(), mine.len(), health.get(at), mine.get(at)));
    }
    if city.refused != layer.city_refused {
        note(&mut compared, format!("city damage refused: today {:?}, the layer {:?}", city.refused, layer.city_refused));
    }
    let warnings = &layer.warnings;
    compared.entries += 1;
    if product.warnings != *warnings {
        note(&mut compared, format!("warnings: today {:?}, the layer {warnings:?}", product.warnings));
    }
    compared.notes = json!({
        "slots": today.len(), "with_entity": placed, "city_provinces": health.len(), "city_refused": city.refused,
        "warnings": product.warnings.len(), "unreadable_cells": layer.unreadable, "whole_visions": layer.whole_visions,
    });
    Ok(compared)
}

pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let mut fresh = of(layer).fresh();
    fresh.compose(mirrors)?;
    let layer = inspected(of(layer))?;
    let (differing, first) = layer.differ(&fresh.inspect());
    Ok(Compared { entries: layer.slots.len() as u64 + layer.health.len() as u64 + 2, differing, first, notes: Value::Null })
}
