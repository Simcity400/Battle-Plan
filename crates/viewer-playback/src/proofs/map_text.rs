
use viewer_layers_political::map_text::{MapText, Source, differ};
use crate::document::Document;
use viewer_layer_core::{layers::{Layer, Upstream}, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{io, sync::Arc};

fn of(layer: &dyn Layer) -> &MapText {
    layer.any().downcast_ref::<MapText>().expect("the map text line's layer")
}

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(MapText::new(Arc::clone(&document.content) as Arc<dyn Source>, document.playback.branch()))
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let choice = viewer_foundation::perspective::Choice::default();
    let today = document.vision(&choice).and_then(|vision| Ok(document.hour()?.map_text(&vision).display())).map_err(|error| format!("map text presented hour failed: {error}"));
    let layer = of(layer);
    let events = today.as_ref().ok().map(|text| match &text.interval {
        map_display_data::MapTextResult::Value { interval } => interval.events.len(),
        map_display_data::MapTextResult::Unavailable { .. } => 0,
    });
    let (differing, first) = differ(layer.product(), today.map(Some));
    Ok(Compared { entries: 1, differing, first, notes: json!({"hour": document.playback.hour(), "made_at": layer.product().map(|made| made.at().1), "events": events}) })
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: 1, differing, first, notes: serde_json::Value::Null })
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [steps, in_place, on_worker] = of(layer).marked();
    vec![("intervals stepped", steps), ("made in place, no event", in_place), ("made on the worker", on_worker)]
}
