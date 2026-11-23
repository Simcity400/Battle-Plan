
use viewer_layers_counters::gun_counters::{Content, GunCounters, PHASES};
use crate::document::Document;
use viewer_layer_core::{layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_counters_land::gun_members::{Gun, GunGround};

const PHASE_NS: [&str; PHASES.len()] = ["ns guns", "ns on rail", "ns names"];

pub fn make(document: &Document) -> Box<dyn Layer> {
    let content = Content::new(Arc::clone(&document.content.counters), Arc::clone(&document.content.map), document.content.movement.clone());
    Box::new(GunCounters::new(Arc::new(content)))
}

pub fn guns(layer: &dyn Layer) -> &GunCounters {
    layer.any().downcast_ref::<GunCounters>().expect("the gun counters line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = guns(layer);
    let mut marked = vec![("guns", layer.marked() as u64)];
    for (name, entities, ns) in layer.phases() {
        marked.push((name, entities));
        marked.push((PHASE_NS[PHASES.iter().position(|&held| held == name).expect("a phase")], ns));
    }
    marked
}

pub fn give(layer: &dyn Layer, document: &Document, choice: &viewer_foundation::perspective::Choice) -> io::Result<()> {
    let Some(layer) = layer.any().downcast_ref::<GunCounters>() else { return Ok(()) };
    layer.set_vision(choice.clone(), document.vision(choice)?);
    let tables = document.hour()?.hour_tables(choice, None)?;
    layer.set_relations(Arc::clone(tables.counters().type_tints()));
    layer.set_hostility(Arc::new(tables.counters().gun_hostility()));
    Ok(())
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = guns(layer);
    let hour = document.hour()?;
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the gun counters layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.guns().next().is_some() || layer.census().is_some() {
            note(&mut compared, format!("the view draws no gun counter, the layer holds {} guns (census {:?})", layer.guns().count(), layer.census()));
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let tables = match hour.hour_tables(&choice, None) {
        Ok(tables) => tables,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            compared.notes = json!({"today_refused": error.to_string(), "layer_refused": layer.refused()});
            return Ok(compared);
        }
    };
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
        return Ok(compared);
    }
    let content = &document.content;
    let (anchor, land) = (GunGround::anchor_of(&content.counters), GunGround::land_of(&content.map));
    let ground = GunGround { anchor: &anchor, land: &land, movement: content.movement.as_deref() };
    let today = tables.counters().gun_members(ground);
    compared.entries += 1;
    let census = (!today.complete).then_some(today.recorded);
    if census != layer.census() {
        note(&mut compared, format!("gun census: today {census:?}, the layer {:?}", layer.census()));
    }
    let theirs: BTreeMap<u64, &Gun> = today.guns.iter().map(|(id, gun)| (*id, gun)).collect();
    let ours: BTreeMap<u64, &Gun> = layer.guns().map(|(id, gun)| (*id, gun)).collect();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    for id in theirs.keys().chain(ours.keys()).collect::<BTreeSet<_>>() {
        compared.entries += 1;
        let (a, b) = (theirs.get(id), ours.get(id));
        if a != b {
            note(&mut compared, format!("gun {id}: today {a:?}, the layer {b:?}"));
        }
        let kind = match a {
            Some(Gun::Member(member)) => format!("member{}{}", if member.unplaced { ", unplaced" } else { "" }, if member.location_warning.is_none() { ", location unknown" } else { "" }),
            Some(Gun::Out(why)) => format!("out: {why}"),
            Some(Gun::Unnamed(why)) => format!("unnamed: {why}"),
            None => "absent today".into(),
        };
        *kinds.entry(kind).or_default() += 1;
    }
    compared.notes = json!({
        "guns": theirs.len(), "kinds": kinds, "census_whole": today.complete, "recorded": today.recorded, "refused_tokens": today.refused,
        "left_out": ["position on the route (laid out at the camera)"],
        "handed_over": ["vision", "type-icon relations", "destination hostility"],
        "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = guns(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: 1 + layer.guns().count() as u64, differing, first, notes: Value::Null })
}
