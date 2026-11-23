
use viewer_layers_counters::naval_markers::{NavalMarkers, Shore};
use crate::document::Document;
use viewer_layer_core::{layers::{Layer, Upstream}, engagements::Index, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{io, sync::Arc};
use viewer_counters_naval::naval_markers::NavalMarkerValues;
use viewer_engagements::engagement_history::History;

pub fn make_with(document: &Document, index: Option<Index>) -> Box<dyn Layer> {
    Box::new(NavalMarkers::new(Arc::clone(&document.content) as Arc<dyn Shore>, index))
}

fn of(layer: &dyn Layer) -> &NavalMarkers {
    layer.any().downcast_ref::<NavalMarkers>().expect("the naval markers line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("makes", of(layer).made() as u64)]
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

fn entries(values: &NavalMarkerValues) -> u64 {
    let combats = values.combats.as_ref().map_or(0, |combats| combats.icons.len() + combats.placement.len() + combats.undecided.len() + 2);
    (combats + values.results.facts.len() + values.results.provinces.len() + 3) as u64
}

fn compare(compared: &mut Compared, a: &NavalMarkerValues, b: &NavalMarkerValues, what: &str) {
    compared.entries += entries(a).max(entries(b));
    match (&a.combats, &b.combats) {
        (Some(x), Some(y)) => {
            for (index, (p, q)) in x.icons.iter().zip(&y.icons).enumerate() {
                if p != q {
                    note(compared, format!("icon {index}: {what} {p:?}, the layer {q:?}"));
                }
            }
            if x.icons.len() != y.icons.len() {
                note(compared, format!("icons: {what} {}, the layer {}", x.icons.len(), y.icons.len()));
            }
            for (name, differs) in [("placement", x.placement != y.placement), ("unplaced", x.unplaced != y.unplaced), ("undecided", x.undecided != y.undecided), ("hq", x.hq != y.hq), ("provenance", x.provenance != y.provenance)] {
                if differs {
                    note(compared, format!("{name} differs: {what} {x:?}, the layer {y:?}"));
                }
            }
        }
        (x, y) if x.is_some() != y.is_some() => note(compared, format!("combats: {what} {x:?}, the layer {y:?}")),
        _ => {}
    }
    for (name, differs) in [("results", a.results != b.results), ("live", a.live != b.live), ("live_undecided", a.live_undecided != b.live_undecided), ("gated", a.gated != b.gated)] {
        if differs {
            note(compared, format!("{name} differs: {what} {:?}, the layer {:?}", (&a.results, &a.live, &a.live_undecided, &a.gated), (&b.results, &b.live, &b.live_undecided, &b.gated)));
        }
    }
}

pub fn today_with(layer: &dyn Layer, document: &Document, today: &History) -> io::Result<Compared> {
    let layer = of(layer);
    let mut compared = Compared::default();
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the naval markers layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.values().is_some() {
            note(&mut compared, "the view draws no naval combat marker, the layer holds its hour part".to_owned());
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let tables = match document.hour()?.hour_tables(&choice, None).and_then(|tables| tables.counters().refine_engagement(Some(today))) {
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
    let Some(mine) = layer.values() else {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({:?}), today draws it", layer.refused()));
        return Ok(compared);
    };
    let theirs = tables.naval_marker_values();
    compare(&mut compared, &theirs, mine, "today");
    let (icons, omitted) = theirs.combats.as_ref().map_or((0, 0), |combats| (combats.icons.iter().filter(|icon| icon.is_ok()).count(), combats.icons.iter().filter(|icon| icon.is_err()).count()));
    compared.notes = json!({
        "live_combats": theirs.live.len(), "icons": icons, "omitted": omitted,
        "placement_causes": theirs.combats.as_ref().map_or(0, |combats| combats.placement.len()),
        "hq_frames": theirs.combats.as_ref().map(|combats| match &combats.hq { Ok(frames) => json!(frames.len()), Err(cause) => json!(cause) }),
        "results": theirs.results.facts.len(), "results_undecided": theirs.results.undecided, "live_undecided": theirs.live_undecided, "gated": theirs.gated.len(),
        "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    let mut compared = Compared::default();
    match (layer.values(), fresh.values()) {
        (Some(a), Some(b)) => compare(&mut compared, a, b, "composed"),
        (a, b) if a.is_some() != b.is_some() => {
            compared.entries = 1;
            note(&mut compared, format!("refused: {:?} against {:?}", layer.refused(), fresh.refused()));
        }
        _ => compared.entries = 1,
    }
    let census = |table: replay_data::rows::TableId| mirrors.get(table).map(|mirror| json!({"status": format!("{:?}", mirror.status()), "rows": mirror.rows().count()}));
    use replay_data::rows::TableId as T;
    compared.notes = json!({
        "combats": census(T::NavalCombats), "vectors": census(T::NavalProvinceCombats), "details": census(T::NavalCombatDetails),
        "transfers": census(T::NavalCombatTransfers), "units": census(T::NavalTransferUnits), "ships_in_combat": census(T::NavalCombatShips),
        "ships": census(T::Ships), "task_forces": census(T::TaskForces), "guns": census(T::RailwayGunCounterInputs),
        "results": census(T::NavalResults), "result_members": census(T::NavalResultMembers), "result_countries": census(T::NavalResultCountries),
    });
    Ok(compared)
}
