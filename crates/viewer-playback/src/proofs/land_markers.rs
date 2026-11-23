
use viewer_layers_counters::land_markers::{Front, LandMarkers};
use crate::document::Document;
use viewer_layer_core::{layers::Layer, engagements::Index, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_counter_kit::combat_markers::LandMarker;
use viewer_engagements::engagement_history::History;
use viewer_foundation::tables::core;

pub fn make_with(document: &Document, index: Option<Index>) -> Box<dyn Layer> {
    Box::new(LandMarkers::new(Arc::clone(&document.content) as Arc<dyn Front>, index))
}

fn of(layer: &dyn Layer) -> &LandMarkers {
    layer.any().downcast_ref::<LandMarkers>().expect("the land markers line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("combats", of(layer).marked() as u64)]
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

type Entry = (Option<LandMarker>, Vec<String>, Vec<String>);

pub fn today_with(layer: &dyn Layer, document: &Document, today: &History) -> io::Result<Compared> {
    let layer = of(layer);
    let mut compared = Compared::default();
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the land markers layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.outcomes().next().is_some() {
            note(&mut compared, format!("the view draws no land combat marker, the layer holds {} combats", layer.outcomes().count()));
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let hour = document.hour()?;
    let tables = match hour.hour_tables(&choice, None).and_then(|tables| tables.counters().refine_engagement(Some(today))) {
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
    let markers = tables.land_markers();
    let provinces: BTreeMap<u64, u32> = core::<replay_data::rows::tables::LandCombats>(document.frame()?).map(|rows| rows.iter().map(|combat| (combat.id, combat.province)).collect()).unwrap_or_default();
    let mut theirs: BTreeMap<u64, Entry> = BTreeMap::new();
    let admitted = |combat: &u64| provinces.get(combat).is_some_and(|&province| layer.admitted(province));
    for marker in &markers.markers {
        if admitted(&marker.id) {
            theirs.entry(marker.id).or_default().0 = Some(marker.clone());
        }
    }
    for (causes, slot) in [(&markers.gaps, 1), (&markers.limits, 2)] {
        for (cause, combats) in causes {
            for combat in combats.iter().filter(|combat| admitted(combat)) {
                let entry = theirs.entry(*combat).or_default();
                if slot == 1 { entry.1.push(cause.clone()) } else { entry.2.push(cause.clone()) }
            }
        }
    }
    let mut mine: BTreeMap<u64, Entry> = BTreeMap::new();
    for (id, province, outcome) in layer.outcomes() {
        if !layer.admitted(province) {
            continue;
        }
        let mut limits = outcome.limits.clone();
        limits.extend(layer.limit().map(str::to_owned));
        if outcome.marker.is_none() && outcome.gaps.is_empty() && limits.is_empty() {
            continue;
        }
        mine.insert(id, (outcome.marker.clone(), outcome.gaps.clone(), limits));
    }
    for entry in theirs.values_mut().chain(mine.values_mut()) {
        entry.1.sort();
        entry.2.sort();
    }
    let (mut seen, mut unseen, mut gaps) = (0u64, 0u64, 0u64);
    for combat in theirs.keys().chain(mine.keys()).collect::<BTreeSet<_>>() {
        compared.entries += 1;
        let (a, b) = (theirs.get(combat), mine.get(combat));
        if let Some((marker, causes, _)) = a {
            match marker {
                Some(marker) if marker.seen => seen += 1,
                Some(_) => unseen += 1,
                None => {}
            }
            gaps += causes.len() as u64;
        }
        if a != b {
            note(&mut compared, format!("combat {combat}: today {a:?}, the layer {b:?}"));
        }
    }
    compared.notes = json!({
        "combats": provinces.len(), "compared": theirs.len(), "seen": seen, "unseen": unseen, "gaps": gaps,
        "limit": layer.limit(), "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    let (differing, first) = layer.differ(&fresh);
    let entries = layer.outcomes().filter(|(_, province, _)| layer.admitted(*province)).count() as u64 + 2;
    let census = |table: replay_data::rows::TableId| mirrors.get(table).map(|mirror| json!({"status": format!("{:?}", mirror.status()), "rows": mirror.rows().count()}));
    use replay_data::rows::TableId as T;
    let notes = json!({
        "land_combats": census(T::LandCombats), "sides": census(T::LandCombatSides), "details": census(T::LandCombatDetails),
        "members": census(T::LandCombatDivisions), "divisions": census(T::Divisions), "panels": census(T::DivisionPanelStats),
        "status": census(T::DivisionStatus), "movement": census(T::DivisionMovement),
    });
    Ok(Compared { entries, differing, first, notes })
}
