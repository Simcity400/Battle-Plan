
use super::{Compared, Document, Layer, Mirrors};
use viewer_layers_forces::{aircraft::Aircraft, ships::Ships};
use viewer_layer_core::rebuilt::Built;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};
use viewer_foundation::perspective::Choice;
use viewer_unit_models::unit_models::naval_differ;

const WAIT: Duration = Duration::from_secs(30);

fn loading(why: String) -> Compared {
    Compared { entries: 0, differing: 0, first: None, notes: json!({"loading": why}) }
}

fn ships(layer: &dyn Layer) -> &Ships {
    layer.any().downcast_ref().expect("the ships line's layer")
}

fn aircraft(layer: &dyn Layer) -> &Aircraft {
    layer.any().downcast_ref().expect("the aircraft line's layer")
}

pub(super) fn ships_make(document: &Document) -> Box<dyn Layer> {
    Box::new(Ships::new(Arc::clone(&document.content) as Arc<dyn viewer_layers_forces::ships::Naval>).expect("the ships layer's worker starts"))
}

pub(super) fn aircraft_make(document: &Document) -> Box<dyn Layer> {
    Box::new(viewer_layers_forces::aircraft::new(Arc::clone(&document.content) as Arc<dyn viewer_layers_forces::aircraft::Source>, true).expect("the aircraft layer's worker starts"))
}

pub(super) fn give(layer: &dyn Layer, side: &crate::playhead_host::MapSide, document: &Document, choice: &Choice) -> io::Result<()> {
    let recording = crate::playhead_host::views::Recording { branch: document.playback.branch() };
    if let Some(ships) = layer.any().downcast_ref::<Ships>() {
        ships.set_source(Arc::clone(side.entry()), recording)?;
        ships.set_vision(choice.clone(), document.vision(choice)?);
        if let Some(hour) = document.frame().ok().map(|frame| frame.hour).filter(|_| ships.drawn()) {
            let _ = ships.rosters().wait(hour, WAIT);
        }
    }
    if let Some(aircraft) = layer.any().downcast_ref::<Aircraft>() {
        aircraft.rebuilt().set_source(Arc::clone(side.entry()), recording, (choice.clone(), None))?;
    }
    Ok(())
}

fn worker<P: Send + Sync + 'static>(rebuilt: &viewer_layer_core::rebuilt::Rebuilt<P>, built: &Built<P>) -> Value {
    let builds = rebuilt.builds();
    json!({
        "builds": builds.count, "build_ms_mean": builds.total_ns as f64 / 1e6 / builds.count.max(1) as f64, "build_ms_most": builds.most_ns as f64 / 1e6,
        "this_build_ms": built.ns as f64 / 1e6, "passed_over": builds.skipped, "discovered": builds.discovered,
        "tables": rebuilt.tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
    })
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

pub(super) fn ships_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    use viewer_unit_history::unit_history::NavalSources;
    use viewer_runtime::unit_class::CounterClass;
    let layer = ships(layer);
    let mut compared = Compared::default();
    if !layer.drawn() {
        compared.entries = 1;
        compared.notes = json!({"drawn": false});
        return Ok(compared);
    }
    if let Err(why) = layer.rosters().wait(document.frame()?.hour, Duration::ZERO) {
        return Ok(loading(why.to_string()));
    }
    let choice = layer.perspective().cloned().unwrap_or_default();
    let today = document.hour()?.prepare_unit_models(&choice);
    let prepared = match (&today, layer.refused()) {
        (Ok(prepared), None) => prepared,
        (Ok(prepared), Some(why)) => {
            compared.entries = 1;
            compared.notes = json!({"layer_refused": why, "today_formations": prepared.ships().formations.len(), "today_candidates": prepared.ships().candidates.len()});
            if !prepared.ships().formations.is_empty() {
                note(&mut compared, format!("the layer refuses the hour ({why}), today places {} formations", prepared.ships().formations.len()));
            }
            return Ok(compared);
        }
        (Err(error), why) => {
            compared.entries = 1;
            compared.notes = json!({"today_refused": error.to_string(), "layer_refused": why});
            return Ok(compared);
        }
    };
    let today = prepared.ships();
    let admitted: std::collections::BTreeSet<u64> = layer.parts().into_keys().collect();
    let candidates: std::collections::BTreeSet<u32> = today.candidates.iter().filter(|candidate| candidate.class == CounterClass::TaskForce).map(|candidate| candidate.province).collect();
    let dropped: std::collections::BTreeSet<u64> = today.positions.iter().filter(|position| admitted.contains(&position.force)).filter(|position| today.candidates.iter().all(|candidate| candidate.identity != position.force)).filter(|position| today.formations.iter().all(|formation| formation.identity != position.force)).map(|position| position.force).collect();
    let keep = |id: u64| admitted.contains(&id) && !dropped.contains(&id);
    let theirs = NavalSources {
        candidates: today.candidates.iter().filter(|candidate| candidate.class == CounterClass::TaskForce && keep(candidate.identity)).cloned().collect(),
        positions: today.positions.iter().filter(|position| keep(position.force)).cloned().collect(),
        formations: today.formations.iter().filter(|formation| keep(formation.identity)).cloned().collect(),
        ..NavalSources::default()
    };
    let (mut mine, report) = layer.sources();
    mine.candidates.retain(|candidate| keep(candidate.identity));
    mine.positions.retain(|position| keep(position.force));
    mine.formations.retain(|formation| keep(formation.identity));
    let (entries, differing, first) = naval_differ(&mine, &theirs);
    (compared.entries, compared.differing, compared.first) = (entries, differing, first);
    let waiting = layer.parts().values().filter(|part| part.is_none()).count();
    if waiting > 0 {
        note(&mut compared, format!("{waiting} admitted task forces have no figure once settled"));
    }
    let (builds, tables) = layer.rosters().measured();
    let [held, admitted_now] = layer.counts();
    compared.notes = json!({
        "task_forces": held, "admitted": admitted_now, "candidates": mine.candidates.len(), "formations": mine.formations.len(), "hulls": report.ship_hulls,
        "today_task_force_provinces": candidates.len(), "dropped_by_competitor_refusal": dropped.len(),
        "left_out": {"competitor_candidates": today.candidates.iter().filter(|candidate| candidate.class != CounterClass::TaskForce).count(), "naval_battles": today.combats.battles.len()},
        "roster_batches": builds.len(), "roster_batch_ms_most": builds.iter().max().map(|ns| *ns as f64 / 1e6), "roster_tables": tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
        "front": layer.front(256.0),
        "window_draws_todays_ships_for": layer.needs_today(),
    });
    Ok(compared)
}

pub(super) fn aircraft_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = aircraft(layer);
    let mut compared = Compared::default();
    if !layer.drawn() {
        compared.entries = 1;
        compared.notes = json!({"drawn": false});
        return Ok(compared);
    }
    let hour = document.frame()?.hour;
    let built = match layer.rebuilt().wait(hour, WAIT) {
        Ok(built) => built,
        Err(why) => {
            if let Some(stopped) = layer.rebuilt().stopped() {
                compared.entries = 1;
                note(&mut compared, stopped.into_inner());
                return Ok(compared);
            }
            return Ok(loading(format!("the aircraft plan is not built at hour {hour}, awaiting the rows of {:?}: {why}", layer.rebuilt().awaiting())));
        }
    };
    let choice = layer.rebuilt().perspective().unwrap_or_default();
    let today = document.hour()?.aircraft_plan(&choice);
    match (&built.product, &today) {
        (Ok(ours), Ok(today)) => {
            let (entries, differing, first) = ours.differ(today);
            (compared.entries, compared.differing, compared.first) = (entries, differing, first);
            let [flights, launches, diagnostics] = ours.counts();
            compared.notes = json!({"flights": flights, "launches": launches, "diagnostics": diagnostics, "worker": worker(layer.rebuilt(), &built)});
        }
        (Err(why), Ok(_)) => {
            compared.entries = 1;
            note(&mut compared, format!("the layer refuses the hour ({why}), today plans it"));
        }
        (Ok(_), Err(error)) => {
            compared.entries = 1;
            note(&mut compared, format!("today refuses the hour ({error}), the layer plans it"));
        }
        (Err(why), Err(error)) => {
            compared.entries = 1;
            if *why != error.to_string() {
                note(&mut compared, format!("the layer refuses the hour ({why}), today otherwise ({error})"));
            }
        }
    }
    Ok(compared)
}

pub(super) fn ships_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = ships(layer);
    if !layer.drawn() {
        return Ok(Compared { entries: 1, notes: json!({"drawn": false}), ..Compared::default() });
    }
    let mut fresh = layer.fresh()?;
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: layer.counts()[1] as u64 + 1, differing, first, notes: Value::Null })
}

pub(super) fn aircraft_fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = aircraft(layer);
    if !layer.drawn() {
        return Ok(Compared { entries: 1, notes: json!({"drawn": false}), ..Compared::default() });
    }
    let mut compared = Compared { entries: 1, ..Compared::default() };
    let Some(built) = layer.product() else {
        if let Some(stopped) = layer.rebuilt().stopped() {
            note(&mut compared, stopped.into_inner());
            return Ok(compared);
        }
        return Ok(loading(format!("the aircraft plan is not built yet, awaiting the rows of {:?}", layer.rebuilt().awaiting())));
    };
    match (layer.again(built), &built.product) {
        (Some(Ok(again)), Ok(ours)) => (compared.entries, compared.differing, compared.first) = ours.differ(&again),
        (Some(Err(error)), Err(why)) if error.to_string() == *why => {}
        (None, _) => {}
        (Some(again), held) => note(&mut compared, format!("built again: {:?} against held {:?}", again.err().map(|error| error.to_string()), held.as_ref().err())),
    }
    Ok(compared)
}

pub(super) fn ships_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [forces, figures, rosters] = ships(layer).marked();
    vec![("task forces", forces as u64), ("figures made", figures as u64), ("rosters asked", rosters as u64)]
}

pub(super) fn aircraft_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [hours, products] = aircraft(layer).marked();
    vec![("hours handed", hours as u64), ("products taken", products as u64)]
}
