//! The counters layer's line of the proof tool (`playback_cost::proof`): the window's layer, made on
//! its worker ([`Fed`]), from the loaded recording, and how the product it hands the window compares
//! with today's division counters of the same hour and with the counters composed anew. The proof's
//! comparisons follow a whole settle, which waits for the product of its hour.

use super::{Counters, PHASES, Source, fed::Fed};

/// [`PHASES`] as the nanosecond counters' names.
const PHASE_NS: [&str; PHASES.len()] = ["ns groups", "ns orders", "ns plans", "ns arrays", "ns members", "ns whole members", "ns strength", "ns orders only", "ns outside", "ns memberships", "ns rows"];
use crate::{
    document::Document,
    layers::{Budget, Layer},
    playback_cost::proof::Compared,
    playhead_host::mirror::Mirrors,
};
use serde_json::json;
use std::{collections::BTreeMap, io};
use viewer_counters::map::counters::Member;
use viewer_foundation::map::counter_action::Action;

pub(crate) fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(Fed::new(Counters::of_document(document)))
}

fn counters(layer: &dyn Layer) -> &Fed {
    layer.any().downcast_ref::<Fed>().expect("the counters line's layer")
}

pub(crate) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = counters(layer);
    let mut marked = vec![("divisions", layer.marked() as u64)];
    for (name, entities, ns) in layer.phases() {
        marked.push((name, entities));
        marked.push((PHASE_NS[PHASES.iter().position(|&held| held == name).expect("a phase")], ns));
    }
    marked
}

/// The layer's sources against today's division counters at the document's hour, under the
/// perspective the layer was given. Each member's type tint and action are left out.
pub(crate) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
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

/// The product the window was handed against the counters composed anew on this thread from the
/// same mirrors and upstream layers, each admitted source with its members and their actions.
pub(crate) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &crate::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = counters(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    fresh.settle_with(mirrors, upstream, &mut Budget::unlimited())?;
    fresh.publish(&mut Budget::unlimited());
    let mut compared = Compared::default();
    if layer.refused() != fresh.refused() {
        compared.entries = 1;
        compared.differing = 1;
        compared.first = Some(format!("the product refuses {:?}, composed anew {:?}", layer.refused(), fresh.refused()));
        return Ok(compared);
    }
    let product = layer.stacks().cloned().unwrap_or_default();
    let sources: std::collections::BTreeSet<&Source> = product.keys().chain(fresh.published.keys()).filter(|source| layer.admitted(source.0)).collect();
    for source in sources {
        let (mine, theirs) = (product.get(source), fresh.published.get(source));
        compared.entries += 1 + mine.map_or(0, |members| members.len()).max(theirs.map_or(0, |members| members.len())) as u64;
        if mine.map(|members| &members[..]) != theirs.map(|members| &members[..]) {
            compared.differing += 1;
            compared.first.get_or_insert_with(|| format!("source {source:?}: the product {mine:?}, composed anew {theirs:?}"));
        }
    }
    compared.entries = compared.entries.max(1);
    Ok(compared)
}
