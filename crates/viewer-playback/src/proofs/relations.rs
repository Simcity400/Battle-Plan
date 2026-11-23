
use viewer_layers_counters::relations::Relations;
use crate::document::Document;
use viewer_layer_core::{layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::{Value, json};
use std::io;
use viewer_foundation::perspective;

pub fn make(_document: &Document) -> Box<dyn Layer> {
    Box::new(Relations::new())
}

fn relations(layer: &dyn Layer) -> &Relations {
    layer.any().downcast_ref::<Relations>().expect("the relations line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("recomputes", relations(layer).recomputes())]
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = relations(layer);
    let tables = document.hour()?.hour_tables(&perspective::Choice::default(), None)?;
    let today = tables.counters().type_tints();
    let (differing, first) = layer.index().differ(today);
    Ok(Compared { entries: today.len().max(layer.index().len()).max(1) as u64, differing, first, notes: json!({"countries": today.len()}) })
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = relations(layer);
    let fresh = layer.composed(mirrors);
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: layer.index().len().max(1) as u64, differing, first, notes: Value::Null })
}
