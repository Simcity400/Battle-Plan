
use viewer_layers_counters::counters::{PHASES, Source, fed::Fed};

const PHASE_NS: [&str; PHASES.len()] = ["ns groups", "ns orders", "ns plans", "ns arrays", "ns members", "ns whole members", "ns strength", "ns orders only", "ns outside", "ns memberships", "ns rows"];
use crate::document::Document;
use viewer_layer_core::{comparison::Compared, layers::Layer};
use serde_json::json;
use std::{collections::BTreeMap, io};
use viewer_counters_land::members::Member;
use viewer_unit_reads::counter_action::Action;

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(Fed::new(crate::layer_sources::counters(document)))
}

fn counters(layer: &dyn Layer) -> &Fed {
    layer.any().downcast_ref::<Fed>().expect("the counters line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = counters(layer);
    let mut marked = vec![("divisions", layer.marked() as u64)];
    for (name, entities, ns) in layer.phases() {
        marked.push((name, entities));
        marked.push((PHASE_NS[PHASES.iter().position(|&held| held == name).expect("a phase")], ns));
    }
    marked
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = counters(layer);
    let hour = document.hour()?;
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the counters layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.sources_acting().next().is_some() {
            note(&mut compared, format!("the view draws no land counter, the layer holds {} sources", layer.sources_acting().count()));
        }
        compared.notes = json!({"not_drawn_at_this_view": true, "layer_refused": layer.refused()});
        return Ok(compared);
    }
    let tables = match hour.hour_tables(&choice, None) {
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
    let mut ties = 0u64;
    let hour = tables.counters();
    let today: BTreeMap<Source, Vec<(Member, Action)>> = hour
        .division_members()
        .into_iter()
        .filter(|(source, _)| layer.admitted(source.0))
        .map(|(source, mut members)| {
            ties += members.windows(2).filter(|pair| pair[0].ordinal == pair[1].ordinal).count() as u64;
            members.sort_by_key(|member| (member.ordinal, member.id));
            (source, members.into_iter().map(|member| { let action = hour.division_action(member.id); (member, action) }).collect())
        })
        .collect();
    let ours: BTreeMap<Source, Vec<(&Member, Action)>> = layer.sources_acting().filter(|(source, _)| layer.admitted(source.0)).map(|(source, members)| (source, members.collect())).collect();
    let admitted_provinces = document.content.counters.bounds.keys().filter(|&&province| layer.admitted(province)).count();
    let mut members = 0u64;
    for source in today.keys().chain(ours.keys()).collect::<std::collections::BTreeSet<_>>() {
        let theirs: Vec<(&Member, Action)> = today.get(source).map(|members| members.iter().map(|(member, action)| (member, *action)).collect()).unwrap_or_default();
        let mine = ours.get(source).cloned().unwrap_or_default();
        members += theirs.len().max(mine.len()) as u64;
        compared.entries += 1 + theirs.len().max(mine.len()) as u64;
        if theirs.len() != mine.len() {
            note(&mut compared, format!("source {source:?}: today {} members {:?}, the layer {} {:?}", theirs.len(), ids(&theirs), mine.len(), ids(&mine)));
            continue;
        }
        for (a, b) in theirs.iter().zip(&mine) {
            if a != b {
                note(&mut compared, format!("source {source:?}: today {a:?}, the layer {b:?}"));
            }
        }
    }
    compared.notes = json!({
        "sources": today.len(), "members": members, "provinces_admitted": admitted_provinces, "ordinal_ties_today": ties,
        "perspective": choice.to_string(),
    });
    Ok(compared)
}

fn ids(members: &[(&Member, Action)]) -> Vec<u64> {
    members.iter().map(|(member, _)| member.id).collect()
}
