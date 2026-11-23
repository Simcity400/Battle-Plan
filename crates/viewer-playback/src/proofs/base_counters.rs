
use viewer_layers_counters::base_counters::{BaseCounters, BaseCountersFed};
use crate::document::Document;
use viewer_layer_core::{layers::{Layer, Upstream}, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::io;
use viewer_counters_naval::bases::BaseValues;

pub fn make(_document: &Document) -> Box<dyn Layer> {
    Box::new(BaseCountersFed::new(BaseCounters::new()))
}

fn of(layer: &dyn Layer) -> &BaseCountersFed {
    layer.any().downcast_ref::<BaseCountersFed>().expect("the base counters line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("units made", of(layer).made() as u64), ("rows read", of(layer).work())]
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

fn compare(compared: &mut Compared, a: &BaseValues, b: &BaseValues, what: &str) {
    compared.entries += (a.wings.len() + a.controlled.len() + a.site_bases.len() + a.radar.len() + 5) as u64;
    for (key, count) in a.wings.iter() {
        if b.wings.get(key) != Some(count) {
            note(compared, format!("wings at {key:?}: {what} {count:?}, the layer {:?}", b.wings.get(key)));
        }
    }
    if a.wings.len() != b.wings.len() {
        note(compared, format!("wings: {what} {} sites, the layer {}", a.wings.len(), b.wings.len()));
    }
    for (province, controlled) in a.controlled.iter() {
        if b.controlled.get(province) != Some(controlled) {
            note(compared, format!("province {province} controlled: {what} {controlled}, the layer {:?}", b.controlled.get(province)));
        }
    }
    if a.controlled.len() != b.controlled.len() {
        note(compared, format!("controlled: {what} {} provinces, the layer {}", a.controlled.len(), b.controlled.len()));
    }
    for (state, ()) in a.radar.iter() {
        if !b.radar.contains_key(state) {
            note(compared, format!("radar state {state}: {what} shows it, the layer does not"));
        }
    }
    for (key, bases) in a.site_bases.iter() {
        if b.site_bases.get(key) != Some(bases) {
            note(compared, format!("site bases at {key:?}: {what} {bases:?}, the layer {:?}", b.site_bases.get(key)));
        }
    }
    for (name, differs) in [("complete", a.complete != b.complete), ("aircraft_complete", a.aircraft_complete != b.aircraft_complete), ("radar states", a.radar.len() != b.radar.len()), ("radar notes", a.radar_notes != b.radar_notes), ("site bases", a.site_bases.len() != b.site_bases.len())] {
        if differs {
            note(compared, format!("{name} differs: {what} {:?}, the layer {:?}", (a.complete, a.aircraft_complete, a.radar.len(), &a.radar_notes, a.site_bases.len()), (b.complete, b.aircraft_complete, b.radar.len(), &b.radar_notes, b.site_bases.len())));
        }
    }
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = of(layer);
    let mut compared = Compared::default();
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the base counters layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.values().is_some() {
            note(&mut compared, "the view draws no base icon, the layer holds its parts".to_owned());
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let tables = match document.hour()?.hour_tables(&choice, None) {
        Ok(tables) => tables,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            return Ok(compared);
        }
    };
    let Some(mine) = layer.values() else {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({:?}), today draws it", layer.refused()));
        return Ok(compared);
    };
    let theirs = tables.counters().base_values();
    compare(&mut compared, &theirs, mine, "today");
    compared.notes = json!({
        "sites": theirs.wings.len(), "shown_sites": theirs.wings.values().filter(|count| count.shown).count(),
        "provinces": theirs.controlled.len(), "radar_states": theirs.radar.len(), "radar_notes": *theirs.radar_notes,
        "complete": theirs.complete, "aircraft_complete": theirs.aircraft_complete,
        "left_out": ["strategic-location icons", "air volunteer allowances", "naval base, naval HQ and facility icons"],
        "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let mut compared = Compared::default();
    match (layer.values(), fresh.values()) {
        (Some(a), Some(b)) => compare(&mut compared, a, b, "composed"),
        (a, b) if a.is_some() != b.is_some() => {
            compared.entries = 1;
            note(&mut compared, format!("refused: {:?} against {:?}", layer.refused(), fresh.refused()));
        }
        _ => compared.entries = 1,
    }
    Ok(compared)
}
