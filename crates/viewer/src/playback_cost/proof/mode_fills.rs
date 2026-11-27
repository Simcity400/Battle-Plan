
use super::{Compared, Document, Layer, Mirrors};
use viewer_layers_political::mode_fills::{Fill, ModeFills, Paint};
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};
use viewer_foundation::perspective::Choice;

const WAIT: Duration = Duration::from_secs(30);

fn fills(layer: &dyn Layer) -> &ModeFills {
    layer.any().downcast_ref().expect("the mode fills line's layer")
}

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(viewer_layers_political::mode_fills::new(Arc::clone(&document.content) as Arc<dyn Paint>, true).expect("the mode fills layer's worker starts"))
}

pub(super) fn give(layer: &dyn Layer, side: &crate::playhead_host::MapSide, document: &Document, choice: &Choice) -> io::Result<()> {
    if let Some(fills) = layer.any().downcast_ref::<ModeFills>() {
        let recording = crate::playhead_host::views::Recording { branch: document.playback.branch() };
        fills.rebuilt().set_source(Arc::clone(side.entry()), recording, (choice.clone(), None))?;
    }
    Ok(())
}

fn differ(ours: &Fill, theirs: &Fill) -> (u64, u64, Option<String>) {
    let (mut entries, mut differing, mut first) = (0, 0, None);
    let mut note = |same: bool, what: String| {
        entries += 1;
        if !same {
            differing += 1;
            first.get_or_insert(what);
        }
    };
    let (a, b) = (&*ours.regions, &*theirs.regions);
    for (name, mine, today) in [("fill key", &a.country_of, &b.country_of), ("owner key", &a.owner_of, &b.owner_of), ("state", &a.state_of, &b.state_of)] {
        let count = mine.len().max(today.len());
        for at in 0..count {
            let (x, y) = (mine.get(at), today.get(at));
            note(x == y, format!("province {at}'s {name}: {x:?} against today's {y:?}"));
        }
    }
    note(a.country_colors == b.country_colors, "the fill keys' colours".into());
    note(a.mode == b.mode, format!("the draw of map mode {}", ours.mode.token()));
    note(a.strategic_attributes == b.strategic_attributes, "the sea regions' naval attributes".into());
    note(a.warnings == b.warnings, format!("warnings {:?} against today's {:?}", a.warnings, b.warnings));
    note(*ours.zone == *theirs.zone, "the demilitarised zone".into());
    note(ours.classes == theirs.classes, "the border class plan".into());
    note(ours.highlighted == theirs.highlighted, "the zone's border plan".into());
    note(ours.mode == theirs.mode, format!("map mode {} against today's {}", ours.mode.token(), theirs.mode.token()));
    (entries, differing, first)
}

fn loading(why: String) -> Compared {
    Compared { entries: 0, differing: 0, first: None, notes: json!({"loading": why}) }
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = fills(layer);
    let mut compared = Compared { entries: 1, ..Compared::default() };
    if !layer.drawn() {
        compared.notes = json!({"drawn": false});
        return Ok(compared);
    }
    let hour = document.frame()?.hour;
    let built = match layer.rebuilt().wait(hour, WAIT) {
        Ok(built) => built,
        Err(why) => {
            if let Some(stopped) = layer.rebuilt().stopped() {
                note(&mut compared, stopped.into_inner());
                return Ok(compared);
            }
            return Ok(loading(format!("the mode fill is not built at hour {hour}, awaiting the rows of {:?}: {why}", layer.rebuilt().awaiting())));
        }
    };
    let (mode, selected) = layer.build().asked();
    let choice = layer.rebuilt().perspective().unwrap_or_default();
    let today = document.content.paint(document.frame()?, &choice, &mode, selected);
    match (&built.product, &today) {
        (Ok(ours), Ok(today)) => {
            (compared.entries, compared.differing, compared.first) = differ(ours, today);
            let builds = layer.rebuilt().builds();
            compared.notes = json!({
                "mode": mode.token(), "provinces": ours.regions.country_of.len(),
                "worker": {"builds": builds.count, "build_ms_mean": builds.total_ns as f64 / 1e6 / builds.count.max(1) as f64, "build_ms_most": builds.most_ns as f64 / 1e6, "this_build_ms": built.ns as f64 / 1e6, "passed_over": builds.skipped, "discovered": builds.discovered, "tables": layer.rebuilt().tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()},
            });
        }
        (Err(why), Ok(_)) => note(&mut compared, format!("the layer refuses the hour ({why}), today draws it")),
        (Ok(_), Err(error)) => note(&mut compared, format!("today refuses the hour ({error}), the layer draws it")),
        (Err(why), Err(error)) if *why != error.to_string() => note(&mut compared, format!("the layer refuses the hour ({why}), today otherwise ({error})")),
        (Err(_), Err(_)) => {}
    }
    Ok(compared)
}

pub(super) fn fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = fills(layer);
    let mut compared = Compared { entries: 1, ..Compared::default() };
    if !layer.drawn() {
        compared.notes = json!({"drawn": false});
        return Ok(compared);
    }
    let Some(built) = layer.product() else {
        if let Some(stopped) = layer.rebuilt().stopped() {
            note(&mut compared, stopped.into_inner());
            return Ok(compared);
        }
        return Ok(loading(format!("the mode fill is not built yet, awaiting the rows of {:?}", layer.rebuilt().awaiting())));
    };
    match (layer.again(built), &built.product) {
        (Some(Ok(again)), Ok(ours)) => (compared.entries, compared.differing, compared.first) = differ(ours, &again),
        (Some(Err(error)), Err(why)) if error.to_string() == *why => {}
        (None, _) => {}
        (Some(again), held) => note(&mut compared, format!("built again: {:?} against held {:?}", again.err().map(|error| error.to_string()), held.as_ref().err())),
    }
    compared.notes = Value::Null;
    Ok(compared)
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [hours, products] = fills(layer).marked();
    vec![("hours handed", hours as u64), ("products taken", products as u64)]
}
