//! The resource icons layer's line of the proof tool (`playback_cost::proof`): its icons against
//! today's hour's icons (`resource_icons::hour_icons` over today's frame) and against itself
//! composed anew. Run it with `--map-mode resources`: in any other mode it declares nothing, and
//! is equal while it holds nothing.

use super::ResourceIcons;
use crate::{document::Document, layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{io, sync::Arc};
use viewer_world::map::resource_icons::{HourIcons, hour_icons};

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(ResourceIcons::new(Arc::clone(&document.content) as Arc<dyn super::Source>))
}

fn icons(layer: &dyn Layer) -> &ResourceIcons {
    layer.any().downcast_ref::<ResourceIcons>().expect("the resource icons line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("states", icons(layer).marked())]
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let held = icons(layer);
    if let Some(why) = held.hidden() {
        let mut compared = Compared { entries: 1, ..Compared::default() };
        if !held.icons.is_empty() {
            compared.differing = 1;
            compared.first = Some(format!("{why}, the layer holds {} states' icons", held.icons.len()));
        }
        compared.notes = json!({"not_drawn_at_this_view": why});
        return Ok(compared);
    }
    let content = &document.content;
    let wanted = match &content.resource_icons {
        Ok(icons) => hour_icons(document.frame()?, &icons.origins, &content.names.localisation),
        Err(why) => Err(why.clone()),
    };
    Ok(against(&wanted, &icons(layer).icons(), "today", "the layer"))
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &crate::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = icons(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    Ok(against(&fresh.icons(), &layer.icons(), "composed", "stepped"))
}

/// States whose icon differs or is in one only, and the whole refusal.
fn against(wanted: &HourIcons, got: &HourIcons, theirs: &str, ours: &str) -> Compared {
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    match (wanted, got) {
        (Ok(wanted), Ok(got)) => {
            compared.entries = wanted.len().max(got.len()) as u64;
            for (state, icon) in wanted {
                if got.get(state) != Some(icon) {
                    note(&mut compared, format!("state {state}: {theirs} {icon:?}, {ours} {:?}", got.get(state)));
                }
            }
            for state in got.keys().filter(|state| !wanted.contains_key(state)) {
                note(&mut compared, format!("state {state}: {theirs} none, {ours} {:?}", got.get(state)));
            }
            let drawn = wanted.values().filter(|icon| matches!(icon, Ok(Some(_)))).count();
            compared.notes = json!({"states": wanted.len(), "icons": drawn});
        }
        (Err(wanted), Err(got)) if wanted == got => compared.entries = 1,
        _ => {
            compared.entries = 1;
            note(&mut compared, format!("whole: {theirs} {:?}, {ours} {:?}", wanted.as_ref().err(), got.as_ref().err()));
        }
    }
    compared
}
