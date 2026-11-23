
use super::Compared;
use crate::{document::Document, playhead_host::mirror::Mirrors};
use viewer_layer_core::{layers::Layer, vision::Vision};
use viewer_layers_infrastructure::{intel::Intel, radar::Radar, snow::Snow, weather::Weather};
use serde_json::{Value, json};
use std::io;

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

fn layer<T: 'static>(layer: &dyn Layer) -> &T {
    layer.any().downcast_ref::<T>().expect("the line's own layer")
}

pub(super) fn weather_make(document: &Document) -> Box<dyn Layer> {
    viewer_playback::layer_sources::weather(document)
}

pub(super) fn weather_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let ours = self::layer::<Weather>(layer).kinds();
    let today = map_products::display::weather_field(document.frame()?)?;
    let mut compared = Compared { entries: today.kinds.len().max(ours.len()) as u64, ..Compared::default() };
    for (&id, &kind) in today.kinds.iter() {
        if ours.get(&id) != Some(&kind) {
            note(&mut compared, format!("region {id}: today kind {kind}, the layer {:?}", ours.get(&id)));
        }
    }
    for (&id, &kind) in &ours {
        if today.kinds.get(&id).is_none() {
            note(&mut compared, format!("region {id}: today no kind, the layer {kind}"));
        }
    }
    compared.notes = json!({"regions": today.kinds.len()});
    Ok(compared)
}

pub(super) fn weather_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = self::layer::<Weather>(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (ours, theirs) = (layer.kinds(), fresh.kinds());
    let mut compared = Compared { entries: ours.len().max(theirs.len()) as u64, ..Compared::default() };
    if ours != theirs {
        let first = ours.iter().find(|(id, kind)| theirs.get(id) != Some(kind)).map(|(id, _)| *id).or_else(|| theirs.keys().find(|id| !ours.contains_key(id)).copied());
        note(&mut compared, format!("region {first:?}: stepped {:?}, composed {:?}", first.and_then(|id| ours.get(&id)), first.and_then(|id| theirs.get(&id))));
        compared.differing = ours.iter().filter(|(id, kind)| theirs.get(id) != Some(kind)).count() as u64 + theirs.keys().filter(|id| !ours.contains_key(id)).count() as u64;
    }
    Ok(compared)
}

pub(super) fn weather_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("regions", self::layer::<Weather>(layer).marked())]
}

pub(super) fn snow_make(document: &Document) -> Box<dyn Layer> {
    viewer_playback::layer_sources::snow(document)
}

fn snow_against(layer: &Snow, today: &map_display_data::SnowMud, compared: &mut Compared) {
    let texels: Vec<u8> = layer.texels().iter().flatten().copied().collect();
    let pairs: [(&str, Vec<u8>, Vec<u8>); 2] = [("texels", today.texels.to_vec(), texels), ("unknown", today.unknown.to_vec(), layer.unknown())];
    for (name, theirs, ours) in pairs {
        compared.entries += theirs.len() as u64;
        if theirs.len() != ours.len() {
            note(compared, format!("{name}: today {} entries, the layer {}", theirs.len(), ours.len()));
        }
        for (at, (a, b)) in theirs.iter().zip(&ours).enumerate() {
            if a != b {
                note(compared, format!("{name}[{at}]: today {a}, the layer {b}"));
            }
        }
    }
    let snow: Vec<Option<i64>> = layer.province_snow().collect();
    compared.entries += today.province_snow.len() as u64;
    if snow.len() != today.province_snow.len() {
        note(compared, format!("province_snow: today {} provinces, the layer {}", today.province_snow.len(), snow.len()));
    }
    for (id, (a, b)) in today.province_snow.iter().zip(&snow).enumerate() {
        if a != b {
            note(compared, format!("province_snow[{id}]: today {a:?}, the layer {b:?}"));
        }
    }
    let factors: Vec<Option<u32>> = layer.factors().iter().map(|factor| factor.map(f32::to_bits)).collect();
    match &today.post_factors {
        None => note(compared, "today has no post factors".into()),
        Some(theirs) => {
            compared.entries += theirs.len() as u64;
            for (volume, (a, b)) in theirs.iter().map(|factor| factor.map(f32::to_bits)).zip(&factors).enumerate() {
                if a != *b {
                    note(compared, format!("post factor {volume}: today {a:?}, the layer {b:?}"));
                }
            }
            if theirs.len() != factors.len() {
                note(compared, format!("post factors: today {}, the layer {}", theirs.len(), factors.len()));
            }
        }
    }
    compared.entries += 2;
    if Some(today.summary) != layer.summary() {
        note(compared, format!("summary: today {:?}, the layer {:?}", today.summary, layer.summary()));
    }
    if today.refresh != layer.refresh() {
        note(compared, format!("refresh: today {:?}, the layer {:?}", today.refresh, layer.refresh()));
    }
}

pub(super) fn snow_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = self::layer::<Snow>(layer);
    let content = document.content.products.snow_content()?;
    let mut compared = Compared::default();
    match map_snow::build(&content.cells, document.frame()?, &content.winter) {
        Err(error) => {
            compared.entries = 1;
            if layer.refused().as_deref() != Some(&error.to_string()) {
                note(&mut compared, format!("today refuses the hour ({error}), the layer {:?}", layer.refused()));
            }
        }
        Ok(today) => {
            if let Some(why) = layer.refused() {
                compared.entries = 1;
                note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
            } else {
                snow_against(layer, &today, &mut compared);
                compared.notes = json!({"cells": today.unknown.len(), "provinces": today.province_snow.len(), "summary": format!("{:?}", today.summary), "content_bytes": layer.content_bytes()});
            }
        }
    }
    Ok(compared)
}

pub(super) fn snow_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = self::layer::<Snow>(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let mut compared = Compared { entries: 6, ..Compared::default() };
    if layer.refused() != fresh.refused() {
        note(&mut compared, format!("refused: stepped {:?}, composed {:?}", layer.refused(), fresh.refused()));
    }
    let parts = [
        ("texels", layer.texels() == fresh.texels()),
        ("unknown", layer.unknown() == fresh.unknown()),
        ("province snow", layer.province_snow().eq(fresh.province_snow())),
        ("factors", layer.factors().iter().map(|factor| factor.map(f32::to_bits)).eq(fresh.factors().iter().map(|factor| factor.map(f32::to_bits)))),
        ("summary and refresh", (layer.summary(), layer.refresh()) == (fresh.summary(), fresh.refresh())),
    ];
    for (part, same) in parts {
        if !same {
            note(&mut compared, format!("{part}: stepped differs from composed"));
        }
    }
    compared.entries += layer.unknown().len() as u64;
    compared.notes = Value::Null;
    Ok(compared)
}

pub(super) fn snow_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = self::layer::<Snow>(layer);
    let [provinces, cells, volumes] = layer.marked();
    let [deltas_us, exchange_us, scene_us] = layer.spent();
    let (worker_us, folds) = layer.worker();
    vec![
        ("provinces", provinces),
        ("cells", cells),
        ("volumes", volumes),
        ("us handing steps to the worker on the frame", deltas_us),
        ("us exchanging with the worker on the frame", exchange_us),
        ("us in the scene's take", scene_us),
        ("us on the worker", worker_us),
        ("folds on the worker", folds),
    ]
}

pub(super) fn vision_make(document: &Document) -> Box<dyn Layer> {
    viewer_playback::layer_sources::vision(document)
}

fn vision_cold(document: &Document, choice: &viewer_foundation::perspective::Choice) -> io::Result<viewer_foundation::perspective::Vision> {
    use viewer_foundation::{perspective, tables};
    let frame = document.frame()?;
    let links = tables::core::<replay_data::rows::tables::CountryLinks>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
    let tags = tables::core::<replay_data::rows::tables::Countries>(frame)?.iter().map(|country| (country.canonical_slot, String::from_utf8_lossy(&country.tag).into_owned())).collect();
    let fog = viewer_foundation::visibility::FogContent::loaded(&document.content.products.fog);
    perspective::Vision::read(frame, &links, &tags, choice, fog)
}

pub(super) fn vision_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = self::layer::<Vision>(layer);
    let choice = layer.choice().clone();
    let mut compared = Compared::default();
    let today = document.vision(&choice);
    let cold = vision_cold(document, &choice);
    let agree = match (&today, &cold) {
        (Ok(today), Ok(cold)) => json!(today.differences(cold).0),
        (Err(a), Err(b)) => json!(a.to_string() == b.to_string()),
        _ => json!("one refuses"),
    };
    match (today, layer.vision()) {
        (Err(error), ours) => {
            compared.entries = 1;
            if ours.is_some() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
        }
        (Ok(_), None) => {
            compared.entries = 1;
            note(&mut compared, format!("the layer refuses the hour ({:?}), today draws it", layer.refused()));
        }
        (Ok(today), Some(ours)) => {
            compared.entries = today.entries();
            let (differing, first) = ours.differences(&today);
            compared.differing = differing;
            compared.first = (!first.is_empty()).then(|| first.join("; "));
            let queries = dependent_queries(layer, &today, document, &mut compared)?;
            compared.notes = json!({"provinces": today.province_fog().map(|fog| fog.len()), "gap": today.visibility_gap(), "undecided": today.undecided(), "today_against_cold": agree, "content_bytes": layer.content_bytes(), "dependent_queries": queries});
        }
    }
    Ok(compared)
}

fn dependent_queries(layer: &Vision, today: &viewer_foundation::perspective::Vision, document: &Document, compared: &mut Compared) -> io::Result<u64> {
    use replay_data::rows::tables;
    use viewer_foundation::tables::core;
    let frame = document.frame()?;
    let links: std::collections::BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
    let mut queries = 0;
    if let Ok(divisions) = core::<tables::Divisions>(frame) {
        for division in divisions.iter() {
            let Some(&owner) = links.get(&division.logical_owner) else { continue };
            queries += 1;
            let (ours, theirs) = (layer.sees_land(division.id, owner, division.province), today.sees_land(division.id, owner, division.province));
            if ours != Some(theirs) {
                note(compared, format!("division {} of {owner} in {}: today sees {theirs:?}, the layer {ours:?}", division.id, division.province));
            }
        }
    }
    for owner in links.values().copied().collect::<std::collections::BTreeSet<u32>>() {
        queries += 1;
        let (ours, theirs) = (layer.shares_vision_proven(owner), today.shares_vision_proven(owner));
        if ours != Some(theirs) {
            note(compared, format!("shared vision of {owner}: today {theirs:?}, the layer {ours:?}"));
        }
    }
    compared.entries += queries;
    Ok(queries)
}

pub(super) fn vision_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = self::layer::<Vision>(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let mut compared = Compared::default();
    let (ours, theirs) = (layer.terms_bytes(), fresh.terms_bytes());
    let terms = ours.iter().filter(|(province, byte)| theirs.get(province) != Some(byte)).count() + theirs.keys().filter(|province| !ours.contains_key(province)).count();
    if terms > 0 {
        compared.differing += terms as u64;
        compared.first = Some(format!("{terms} province bytes of the terms differ, the first {:?}", ours.iter().find(|(province, byte)| theirs.get(province) != Some(byte))));
    }
    let gap_status = layer
        .vision()
        .and_then(|seen| seen.visibility_gap())
        .and_then(|name| replay_data::rows::TableId::ALL.iter().find(|table| format!("{table:?}") == name))
        .map(|&table| mirrors.get(table).and_then(crate::playhead_host::mirror::Mirror::not_recorded).unwrap_or_else(|| format!("{table:?} is whole")));
    compared.notes = json!({"terms_bytes": theirs.len(), "gap_status": gap_status});
    match (layer.vision(), fresh.vision()) {
        (Some(ours), Some(theirs)) => {
            compared.entries = theirs.entries() + terms_entries(&fresh);
            let (differing, first) = ours.differences(&theirs);
            compared.differing += differing;
            if !first.is_empty() {
                compared.first = Some(first.join("; "));
            }
        }
        (None, None) => compared.entries = 1 + terms_entries(&fresh),
        (a, b) => {
            compared.entries = 1;
            note(&mut compared, format!("stepped draws {}, composed draws {}", a.is_some(), b.is_some()));
        }
    }
    Ok(compared)
}

fn terms_entries(layer: &Vision) -> u64 {
    layer.terms_bytes().len() as u64
}

pub(super) fn vision_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = self::layer::<Vision>(layer);
    let [units, provinces, sources] = layer.marked();
    let [links, countries, viewer, admitted] = layer.rebuilds();
    let [cutoffs_us, sources_us, provinces_us, sea_us] = layer.spent();
    let (worker_us, folds) = layer.worker();
    vec![
        ("us handing steps to the worker on the frame", layer.copying_us()),
        ("us on the worker", worker_us),
        ("folds on the worker", folds),
        ("us in cutoffs", cutoffs_us),
        ("us in sources", sources_us),
        ("us in provinces", provinces_us),
        ("us in divisions at sea", sea_us),
        ("units", units),
        ("provinces", provinces),
        ("sources", sources),
        ("rebuilt for links", links),
        ("rebuilt for countries", countries),
        ("rebuilt for the viewer", viewer),
        ("rebuilt for admission", admitted),
    ]
}

pub(super) fn intel_make(document: &Document) -> Box<dyn Layer> {
    viewer_playback::layer_sources::intel(document)
}

pub(super) fn intel_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = self::layer::<Intel>(layer);
    let choice = viewer_foundation::perspective::Choice::default();
    let mut compared = Compared::default();
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        note(&mut compared, format!("the layer is refused: {why}"));
        return Ok(compared);
    }
    let today = document.hour()?.intel_map(&choice)?;
    let (texels, fog, fogged) = layer.map();
    compared.entries = today.texels.len() as u64 + 2;
    if today.texels.len() != texels.len() {
        note(&mut compared, format!("texels: today {}, the layer {}", today.texels.len(), texels.len()));
    }
    for (cell, (a, b)) in today.texels.iter().zip(texels).enumerate() {
        if a != b {
            note(&mut compared, format!("texel {cell}: today {a}, the layer {b}"));
        }
    }
    if today.fog != fog || today.fogged_texels != fogged {
        note(&mut compared, format!("fog {} / {} fogged today, {fog} / {fogged} the layer", today.fog, today.fogged_texels));
    }
    let vision = document.vision(&choice)?;
    let held = match (vision.province_fog(), layer.province_bytes()) {
        (Some(today), Some(ours)) => {
            compared.entries += today.len() as u64;
            today.iter().filter(|&(&province, &byte)| ours.get(province as usize) != Some(&byte)).count()
        }
        (None, None) => 0,
        _ => 1,
    };
    if held > 0 {
        compared.differing += held as u64;
        compared.first.get_or_insert(format!("{held} province bytes held differ from today's vision"));
    }
    compared.notes = json!({"fog": today.fog, "fogged": today.fogged_texels, "cells_marked_total": layer.marked()});
    Ok(compared)
}

pub(super) fn intel_fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = self::layer::<Intel>(layer);
    let fresh = layer.composed();
    let ((ours, fog, fogged), (theirs, their_fog, their_fogged)) = (layer.map(), fresh.map());
    let mut compared = Compared { entries: theirs.len() as u64 + 2, ..Compared::default() };
    for (cell, (a, b)) in ours.iter().zip(theirs).enumerate() {
        if a != b {
            note(&mut compared, format!("texel {cell}: stepped {a}, composed {b}"));
        }
    }
    if (ours.len(), fog, fogged) != (theirs.len(), their_fog, their_fogged) {
        note(&mut compared, format!("stepped {} texels fog {fog} fogged {fogged}, composed {} fog {their_fog} fogged {their_fogged}", ours.len(), theirs.len()));
    }
    Ok(compared)
}

pub(super) fn intel_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("cells", self::layer::<Intel>(layer).marked())]
}

pub(super) fn radar_make(document: &Document) -> Box<dyn Layer> {
    viewer_playback::layer_sources::radar(document)
}

fn radar_field(layer: &Radar, document: &Document, ours: u32) -> io::Result<viewer_aircraft::radar_symbols::Field> {
    let hour = document.hour()?;
    let mut field = viewer_aircraft::radar_symbols::Field::default();
    match layer.groups(ours) {
        Ok(groups) => field.groups(groups, ours, hour.heights(), &document.content.radar_centers),
        Err(why) => *field.refused.entry(why).or_default() += 1,
    }
    Ok(field)
}

fn radar_against(ours: &viewer_aircraft::radar_symbols::Field, theirs: &viewer_aircraft::radar_symbols::Field, compared: &mut Compared) {
    compared.entries = theirs.symbols.len().max(ours.symbols.len()) as u64 + 1;
    if ours.symbols.len() != theirs.symbols.len() {
        note(compared, format!("symbols: {} against {}", ours.symbols.len(), theirs.symbols.len()));
    }
    for (at, (a, b)) in ours.symbols.iter().zip(&theirs.symbols).enumerate() {
        if a != b {
            note(compared, format!("symbol {at}: {a:?} against {b:?}"));
        }
    }
    if ours.refused != theirs.refused {
        note(compared, format!("refused: {:?} against {:?}", ours.refused, theirs.refused));
    }
}

pub(super) fn radar_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = self::layer::<Radar>(layer);
    let ours = document.vision(&viewer_foundation::perspective::Choice::default())?.ours();
    let hour = document.hour()?;
    let today = viewer_aircraft::radar_symbols::Field::read(document.frame()?, ours, hour.heights(), &document.content.radar_centers);
    let mut compared = Compared::default();
    let viewer = layer.ours().unwrap_or(u32::MAX);
    radar_against(&radar_field(layer, document, viewer)?, &today, &mut compared);
    compared.entries += 1;
    if layer.ours() != Some(ours) {
        note(&mut compared, format!("viewer from the vision layer {:?}, today {ours}", layer.ours()));
    }
    compared.notes = json!({"symbols": today.symbols.len(), "refused": today.refused.len()});
    Ok(compared)
}

pub(super) fn radar_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = self::layer::<Radar>(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let mut compared = Compared::default();
    match (layer.groups(0), fresh.groups(0)) {
        (Ok(a), Ok(b)) => {
            compared.entries = b.len() as u64;
            if a != b {
                compared.differing = a.iter().zip(&b).filter(|(x, y)| x != y).count() as u64 + a.len().abs_diff(b.len()) as u64;
                compared.first = Some(format!("{:?} against {:?}", a.iter().zip(&b).find(|(x, y)| x != y), (a.len(), b.len())));
            }
        }
        (a, b) => {
            compared.entries = 1;
            if a.as_ref().err() != b.as_ref().err() {
                note(&mut compared, format!("{:?} against {:?}", a.err(), b.err()));
            }
        }
    }
    Ok(compared)
}

pub(super) fn radar_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("groups", self::layer::<Radar>(layer).marked())]
}
