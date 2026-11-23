
use viewer_layers_counters::naval_counters::{NavalCounters, Waters};
use crate::document::Document;
use viewer_layer_core::{layers::Layer, comparison::Compared, playhead_host::mirror::Mirrors};
use serde_json::json;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_counters_naval::naval_members::NavalMember;
use viewer_foundation::mode_visibility::UnitKinds;

const LEFT_OUT: [&str; 4] = ["ship-model cells (hull, capitals, carrier)", "native task-force operands (supply, status, port tooltip, pride)", "ideology frame", "the air map mode's carrier fleets (the layer is not drawn there)"];

pub fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(NavalCounters::new(Arc::clone(&document.content) as Arc<dyn Waters>))
}

fn naval(layer: &dyn Layer) -> &NavalCounters {
    layer.any().downcast_ref::<NavalCounters>().expect("the naval counters line's layer")
}

pub fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    vec![("task forces", naval(layer).marked() as u64)]
}

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

pub fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = naval(layer);
    let hour = document.hour()?;
    let mut compared = Compared::default();
    let Some((choice, _)) = layer.perspective() else {
        return Err(io::Error::other("the naval counters layer was given no perspective"));
    };
    if layer.hidden() {
        compared.entries = 1;
        if layer.members().next().is_some() || !layer.fleets().is_empty() || *layer.ports() != viewer_counters_naval::naval_members::PortSummary::default() {
            note(&mut compared, format!("the view draws no naval counter, the layer holds {} task forces, {} fleets, ports {:?}", layer.members().count(), layer.fleets().len(), layer.ports()));
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
    let today = tables.counters().naval_members(document.content.ground(), UnitKinds::All, Err("the equipment is not read by the comparison"))?;
    let theirs: BTreeMap<u64, &NavalMember> = today.members.iter().filter(|member| layer.compared(member)).map(|member| (member.fields.id, member)).collect();
    let mine: BTreeMap<u64, &NavalMember> = layer.members().filter(|member| layer.compared(member)).map(|member| (member.fields.id, member)).collect();
    let (mut sources, mut ports) = (0u64, 0u64);
    for id in theirs.keys().chain(mine.keys()).collect::<std::collections::BTreeSet<_>>() {
        compared.entries += 1;
        let (a, b) = (theirs.get(id), mine.get(id));
        if let Some(member) = a {
            sources += u64::from(matches!(member.sea, viewer_counters_naval::naval_members::Sea::Source { .. }));
            ports += u64::from(matches!(member.port, viewer_counters_naval::naval_members::Port::In(_)));
        }
        if a != b {
            note(&mut compared, format!("task force {id}: today {a:?}, the layer {b:?}"));
        }
    }
    compared.entries += today.fleets.len().max(layer.fleets().len()) as u64;
    for fleet in today.fleets.keys().chain(layer.fleets().keys()).collect::<std::collections::BTreeSet<_>>() {
        if today.fleets.get(fleet) != layer.fleets().get(fleet) {
            note(&mut compared, format!("fleet {fleet}: today {:?}, the layer {:?}", today.fleets.get(fleet), layer.fleets().get(fleet)));
        }
    }
    compared.entries += 1 + today.ports.anchors.len() as u64 + today.ports.undecided.len() as u64;
    if today.ports != *layer.ports() {
        note(&mut compared, format!("bases: today {:?}, the layer {:?}", today.ports, layer.ports()));
    }
    compared.entries += 1;
    if today.peace_conference != layer.peace() {
        note(&mut compared, format!("peace conference: today {}, the layer {}", today.peace_conference, layer.peace()));
    }
    let mut at_sea: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    for member in &today.members {
        if let viewer_counters_naval::naval_members::Sea::Source { kind, .. } = member.sea {
            *at_sea.entry((member.fields.province, kind)).or_default() += 1;
        }
    }
    let busiest = at_sea.iter().max_by_key(|&(_, &count)| count).map(|(&(province, kind), &count)| json!({"province": province, "kind": kind, "task_forces": count, "anchor": document.content.counters.anchor(province, kind)}));
    let census = match &*document.frame()?.tables[replay_data::rows::TableId::TaskForces.index()] {
        replay_frame::frame::TableView::Complete(_) => "complete".to_owned(),
        replay_frame::frame::TableView::Truncated { missing, .. } => format!("truncated, {missing} missing"),
        replay_frame::frame::TableView::Unavailable { reason } => format!("unavailable: {reason}"),
        replay_frame::frame::TableView::NotRecorded { .. } => "not recorded".to_owned(),
    };
    compared.notes = json!({
        "task_forces_census": census,
        "busiest_sea_source": busiest,
        "task_forces": theirs.len(), "recorded_task_forces": today.members.len(), "at_sea_sources": sources, "in_port": ports, "fleets": today.fleets.len(),
        "bases": today.ports.anchors.len() + today.ports.unanchored, "undecided_bases": today.ports.undecided.len(),
        "forces_complete": today.forces_complete, "left_out": LEFT_OUT, "perspective": choice.to_string(),
    });
    Ok(compared)
}

pub fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = naval(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    let entries = layer.members().filter(|member| layer.compared(member)).count() as u64 + layer.fleets().len() as u64 + 2;
    let census = |table: replay_data::rows::TableId| mirrors.get(table).map(|mirror| json!({"status": format!("{:?}", mirror.status()), "rows": mirror.rows().count()}));
    let notes = json!({
        "members": layer.members().count(),
        "task_forces": census(replay_data::rows::TableId::TaskForces),
        "ships": census(replay_data::rows::TableId::Ships),
        "motion": census(replay_data::rows::TableId::TaskForceMapMotion),
        "order": census(replay_data::rows::TableId::ProvinceUnitOrder),
    });
    Ok(Compared { entries, differing, first, notes })
}
