
use viewer_layers_political::occupation_marks::OccupationMarks;
use crate::document::Document;
use viewer_layer_core::{layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{io, sync::Arc};
use viewer_map_icons::occupation_icons::{Marks, hour_marks};

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(OccupationMarks::new(Arc::clone(&document.content) as Arc<dyn viewer_layers_political::occupation_marks::Source>))
}

fn marks(layer: &dyn Layer) -> &OccupationMarks {
    layer.any().downcast_ref::<OccupationMarks>().expect("the occupation marks line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("states", marks(layer).marked())]
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let held = marks(layer);
    if let Some(why) = held.hidden() {
        let mut compared = Compared { entries: 1, ..Compared::default() };
        let (marks, viewers) = held.retained_marks();
        if marks != 0 || viewers {
            compared.differing = 1;
            compared.first = Some(format!("{why}, the layer holds {} states' marks", marks));
        }
        compared.notes = json!({"not_drawn_at_this_view": why});
        return Ok(compared);
    }
    let wanted = match &document.content.icons.occupation {
        Ok(content) => hour_marks(document.frame()?, content),
        Err(why) => Err(why.clone()),
    };
    Ok(against(&wanted, &marks(layer).marks(), "today", "the layer"))
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = marks(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    Ok(against(&fresh.marks(), &layer.marks(), "composed", "stepped"))
}

fn against(wanted: &Marks, got: &Marks, theirs: &str, ours: &str) -> Compared {
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    match (wanted, got) {
        (Ok(wanted), Ok(got)) => {
            compared.entries = (wanted.marks.len().max(got.marks.len()) + 2) as u64;
            for (state, mark) in &wanted.marks {
                if got.marks.get(state) != Some(mark) {
                    note(&mut compared, format!("state {state}: {theirs} {mark:?}, {ours} {:?}", got.marks.get(state)));
                }
            }
            for state in got.marks.keys().filter(|state| !wanted.marks.contains_key(state)) {
                note(&mut compared, format!("state {state}: {theirs} none, {ours} {:?}", got.marks.get(state)));
            }
            if wanted.viewers != got.viewers {
                note(&mut compared, format!("viewers: {theirs} {} slots, {ours} {}", wanted.viewers.len(), got.viewers.len()));
            }
            if wanted.resistance != got.resistance {
                note(&mut compared, format!("StateResistance: {theirs} {:?}, {ours} {:?}", wanted.resistance, got.resistance));
            }
            let shown = wanted.marks.values().filter(|mark| matches!(mark.resistance, Some(Ok(Some(_))))).count();
            compared.notes = json!({"states": wanted.marks.len(), "shown": shown});
        }
        (Err(wanted), Err(got)) if wanted == got => compared.entries = 1,
        _ => {
            compared.entries = 1;
            note(&mut compared, format!("whole: {theirs} {:?}, {ours} {:?}", wanted.as_ref().err(), got.as_ref().err()));
        }
    }
    compared
}
