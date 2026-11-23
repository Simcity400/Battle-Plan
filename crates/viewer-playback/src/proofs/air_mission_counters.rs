
use viewer_layers_counters::air_mission_counters::{AirMissionCounters, Content, PHASES, Passes};
use crate::document::Document;
use viewer_layer_core::{layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use replay_derive::air_mission_counts::MISSIONS;
use viewer_counters_naval::air_missions::{counts_withheld, fragment_withheld, ground_withheld};

const PHASE_NS: [&str; PHASES.len()] = ["ns registrations", "ns holders", "ns regions", "ns by the hour"];

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(AirMissionCounters::new(Arc::new(Content::new(&document.content.counters))))
}

fn air(layer: &dyn Layer) -> &AirMissionCounters {
    layer.any().downcast_ref::<AirMissionCounters>().expect("the air mission line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = air(layer);
    let mut marked = vec![("regions", layer.marked() as u64), ("held regions", layer.held_regions() as u64)];
    for (name, entities, ns) in layer.phases() {
        marked.push((name, entities));
        marked.push((PHASE_NS[PHASES.iter().position(|&held| held == name).expect("a phase")], ns));
    }
    marked
}

pub fn give(layer: &dyn Layer, document: &Document, choice: &viewer_foundation::perspective::Choice) -> io::Result<()> {
    let Some(layer) = layer.any().downcast_ref::<AirMissionCounters>() else { return Ok(()) };
    layer.set_vision(choice.clone(), document.vision(choice)?);
    let passes: Passes = document.playback.air_rosters();
    layer.set_passes(passes, document.playback.branch());
    Ok(())
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = air(layer);
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the air mission layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if !layer.values().is_empty() || layer.withheld().is_some() {
            note(&mut compared, format!("the view draws no air mission counter, the layer holds {} regions (withheld {:?})", layer.values().len(), layer.withheld()));
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let tables = match document.hour()?.hour_tables(&choice, None) {
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
    let today = tables.counters().air_values();
    let withheld_prefix = counts_withheld("");
    let today_withheld: Vec<&String> = today.refused.keys().filter(|key| key.starts_with(&withheld_prefix)).collect();
    compared.entries += 1;
    if let Some(withheld) = layer.withheld() {
        if today.refused != BTreeMap::from([(withheld.to_owned(), 1)]) {
            note(&mut compared, format!("the counts: the layer withholds them ({withheld}), today {:?}", today.refused));
        }
        compared.notes = json!({"withheld": withheld});
        return Ok(compared);
    }
    if !today_withheld.is_empty() {
        note(&mut compared, format!("the counts: today withholds them ({today_withheld:?}), the layer makes them"));
        return Ok(compared);
    }
    let values = layer.values();
    compared.entries += 1;
    if (today.supply_complete, today.supply_unrecorded) != layer.supply() {
        note(&mut compared, format!("supply complete, unrecorded: today {:?}, the layer {:?}", (today.supply_complete, today.supply_unrecorded), layer.supply()));
    }
    let mut fragments: BTreeMap<String, usize> = BTreeMap::new();
    let mut ground_refused: BTreeMap<String, usize> = BTreeMap::new();
    let (mut ground_compared, mut ground_today_none) = (0u64, 0u64);
    for region in today.regions.keys().chain(values.keys()).collect::<BTreeSet<_>>() {
        compared.entries += 1;
        let (theirs, ours) = (today.regions.get(region), values.get(region));
        let Some(ours) = ours else {
            note(&mut compared, format!("region {region}: today counts {theirs:?}, the layer none"));
            continue;
        };
        if theirs != Some(&ours.counts) {
            note(&mut compared, format!("region {region} counts: today {theirs:?}, the layer {:?}", ours.counts));
        }
        let alerts: u32 = (0..MISSIONS).filter(|&kind| today.alerts.contains(&(*region, kind))).fold(0, |bits, kind| bits | 1 << kind);
        if alerts != ours.alerts {
            note(&mut compared, format!("region {region} alerts: today {alerts:#x}, the layer {:#x}", ours.alerts));
        }
        let uncertain: Vec<(usize, &str)> = today.uncertain.range((*region, 0)..=(*region, usize::MAX)).map(|(&(_, kind), &reason)| (kind, reason)).collect();
        if uncertain != ours.uncertain {
            note(&mut compared, format!("region {region} withheld entries: today {uncertain:?}, the layer {:?}", ours.uncertain));
        }
        for (_, reason) in &ours.uncertain {
            *fragments.entry(fragment_withheld(reason)).or_default() += 1;
        }
        let supply = today.supply_complete.then(|| today.supply.get(region).copied()).flatten();
        if supply != ours.supply {
            note(&mut compared, format!("region {region} supply: today {supply:?}, the layer {:?}", ours.supply));
        }
        match (today.ground.get(region), &ours.ground) {
            (Some(theirs), Some(Ok(mine))) if theirs == mine => ground_compared += 1,
            (Some(theirs), mine) => note(&mut compared, format!("region {region} Ground Crews: today {theirs:?}, the layer {mine:?}")),
            (None, Some(Err(reason))) => *ground_refused.entry(ground_withheld(reason)).or_default() += 1,
            (None, Some(Ok(_))) => ground_today_none += 1,
            (None, None) => {}
        }
    }
    let today_fragments: BTreeMap<String, usize> = today.refused.iter().filter(|(key, _)| key.starts_with(&fragment_withheld(""))).map(|(key, &count)| (key.clone(), count)).collect();
    compared.entries += 1;
    if today_fragments != fragments {
        note(&mut compared, format!("withheld entries: today {today_fragments:?}, the layer {fragments:?}"));
    }
    for (reason, &count) in &ground_refused {
        compared.entries += 1;
        if today.refused.get(reason).copied().unwrap_or(0) < count {
            note(&mut compared, format!("{reason}: the layer {count}, today {:?}", today.refused.get(reason)));
        }
    }
    let ground_prefix = ground_withheld("");
    let ground_today_only: BTreeMap<&String, usize> = today.refused.iter().filter(|(key, _)| key.starts_with(&ground_prefix) && !ground_refused.contains_key(*key)).map(|(key, &count)| (key, count)).collect();
    compared.notes = json!({
        "regions": today.regions.len(), "alerts": today.alerts.len(), "withheld_entries": today.uncertain.len(), "held_regions": layer.held_regions(),
        "ground_compared": ground_compared, "ground_where_today_has_none": ground_today_none, "ground_withheld_by_today_alone": ground_today_only,
        "supply_outside_counted_regions": today.supply.keys().filter(|region| !today.regions.contains_key(region)).count(),
        "left_out": ["the hovers (localised text, made where the tooltips are made)"],
        "handed_over": ["vision", "daily air rosters"],
        "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = air(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: 1 + layer.values().len() as u64, differing, first, notes: Value::Null })
}
