
use super::Compared;
use crate::{document::Document, playhead_host::mirror::Mirrors};
use viewer_layer_core::layers::Layer;
use viewer_layers_political::naval::Naval;
use serde_json::json;
use std::{collections::BTreeMap, io, sync::Arc};
use map_display_data::landscape::Attributes;
use viewer_map_modes::map_mode::MapMode;

const WARNING: &str = "Naval dominance FX withheld";

fn naval(layer: &dyn Layer) -> &Naval {
    layer.any().downcast_ref::<Naval>().expect("the naval line's layer")
}

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(Naval::new(Arc::new(document.content.naval_dominance.clone())))
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

fn differ(compared: &mut Compared, names: [&str; 2], ours: (&BTreeMap<u32, Attributes>, &[String]), theirs: (&BTreeMap<u32, Attributes>, &[String])) {
    let keys: std::collections::BTreeSet<&u32> = ours.0.keys().chain(theirs.0.keys()).collect();
    compared.entries += keys.len() as u64 + 1;
    for key in keys {
        if ours.0.get(key) != theirs.0.get(key) {
            note(compared, format!("region {}: {} {:?}, {} {:?}", key.wrapping_sub(1), names[0], theirs.0.get(key), names[1], ours.0.get(key)));
        }
    }
    if ours.1 != theirs.1 {
        note(compared, format!("warnings: {} {:?}, {} {:?}", names[0], theirs.1, names[1], ours.1));
    }
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = naval(layer);
    let hour = document.hour()?;
    let mut compared = Compared::default();
    if !layer.drawn() {
        compared.entries = 1;
        compared.notes = json!({"drawn": false});
        return Ok(compared);
    }
    let (today, source) = match hour.political_product(&MapMode::default(), layer.choice(), None, None) {
        Ok(product) => {
            let regions = &product.layers.regions;
            (Ok((regions.strategic_attributes.clone(), regions.warnings.iter().filter(|text| text.starts_with(WARNING)).cloned().collect::<Vec<_>>())), "political part")
        }
        Err(refused) => match hour.vision(layer.choice()) {
            Ok(vision) => {
                let fill = document.content.naval_dominance.read(document.frame()?, vision.ours());
                (Ok((fill.attributes.into_iter().map(|(id, attributes)| (id + 1, attributes)).collect(), fill.warnings)), "naval read: the political part is refused")
            }
            Err(_) => (Err(refused.to_string()), "refused"),
        },
    };
    match (today, layer.viewer()) {
        (Err(refused), None) => {
            compared.entries = 1;
            compared.notes = json!({"today_refused": refused});
        }
        (Err(refused), Some(viewer)) => {
            compared.entries = 1;
            note(&mut compared, format!("today refuses the hour ({refused}), the layer draws it for slot {viewer}"));
        }
        (Ok(_), None) => {
            compared.entries = 1;
            note(&mut compared, "the layer has no vision, today draws the hour".into());
        }
        (Ok((attributes, warnings)), Some(viewer)) => {
            let ours = layer.warnings();
            differ(&mut compared, ["today", "the layer"], (layer.attributes(), &ours), (&attributes, &warnings));
            let withheld = ours.first().map_or(0, |text| text.matches("; ").count() + 1);
            compared.notes = json!({"source": source, "viewer": viewer, "regions": attributes.len(), "withheld": withheld});
        }
    }
    Ok(compared)
}

pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = naval(layer);
    let fresh = layer.composed(mirrors);
    let mut compared = Compared::default();
    differ(&mut compared, ["composed", "stepped"], (layer.attributes(), &layer.warnings()), (fresh.attributes(), &fresh.warnings()));
    Ok(compared)
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = naval(layer);
    let [regions, settled, sides] = layer.marked();
    let ([viewer, relations, epoch], moved) = layer.clears();
    vec![("regions", regions), ("settled", settled), ("sides read", sides), ("all for the viewer", viewer), ("all for a relation", relations), ("all for the epoch or status", epoch), ("parts moved (bits)", u64::from(moved))]
}
