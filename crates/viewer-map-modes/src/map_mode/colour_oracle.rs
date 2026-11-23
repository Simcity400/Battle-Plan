use super::{Inputs, finish, pack};
use map_display_data::landscape::{ModeLayer, Style};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
use dashboard_text::triggers::{Eval, Evaluator, Temps};
use content_reader_map::scripted_map_modes::{Layer, LayerType};
use replay_data::rows::tables;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use viewer_foundation::tables::core;

const FIXED5: i64 = 100_000;

const HOURS_PER_DAY: i32 = 24;

pub(super) const FLAT_GRADIENT: u8 = 244;

use map_display_data::landscape::{Border, ColourState, LayerState, TargetId};

pub(super) fn refreshes(previous: Option<&ColourState>, update_daily: bool, hour: i32) -> bool {
    previous.is_none() || (update_daily && hour.rem_euclid(HOURS_PER_DAY) == 0)
}

pub(super) fn forced_refresh(mode: &str, inputs: &Inputs) -> Result<bool, String> {
    use replay_data::rows::{MapEventFamily, MapEventPayload};
    let Some(index) = inputs
        .content
        .scripted
        .modes
        .iter()
        .position(|entry| entry.name == mode)
    else {
        return Ok(false);
    };
    let rows = core::<tables::MapEvents>(inputs.frame).map_err(|error| {
        format!("Scripted map mode {mode}: forced refresh events unavailable: {error}")
    })?;
    let [interval] = rows else {
        return Err(format!(
            "Scripted map mode {mode}: forced refresh interval is absent"
        ));
    };
    interval.validate().map_err(|why| {
        format!("Scripted map mode {mode}: forced refresh interval is malformed: {why}")
    })?;
    if interval.to_hour != inputs.frame.hour {
        return Err(format!(
            "Scripted map mode {mode}: forced refresh interval is not this hour's"
        ));
    }
    if !interval.health[MapEventFamily::ScriptedMapRefresh.index()].complete {
        return Err(format!(
            "Scripted map mode {mode}: forced refresh interval is incomplete"
        ));
    }
    if interval.events.iter().any(|event| matches!(event.payload, MapEventPayload::ScriptedMapRefresh(ordinal) if ordinal as usize >= inputs.content.scripted.modes.len())) {
        return Err(format!("Scripted map mode {mode}: a forced refresh declaration ordinal is outside the recorded content"));
    }
    Ok(interval.events.iter().any(|event| matches!(event.payload, MapEventPayload::ScriptedMapRefresh(ordinal) if ordinal as usize == index)))
}

struct Target {
    id: TargetId,
    object: EventObject,
    controller: Option<u32>,
    provinces: Vec<u32>,
}

pub(super) fn borders(
    mode: &str,
    side: &str,
    layer: &Layer,
    inputs: &Inputs,
    previous: Option<&LayerState>,
    refresh: bool,
    warnings: &mut Vec<String>,
) -> LayerState {
    let state = if refresh {
        refreshed(mode, side, layer, inputs, previous)
    } else {
        previous.cloned().unwrap_or_default()
    };
    warnings.extend(state.refusals.iter().cloned());
    state
}

fn refreshed(
    mode: &str,
    side: &str,
    layer: &Layer,
    inputs: &Inputs,
    previous: Option<&LayerState>,
) -> LayerState {
    let what = format!("Scripted map mode {mode}: its {side} layer");
    let refused_outright = |why: String| LayerState {
        borders: BTreeMap::new(),
        refusals: vec![why],
        refused: true,
    };
    let Some(world) = inputs.world else {
        return refused_outright(format!(
            "{what}'s colour script has no recorded hour to run on"
        ));
    };
    let Some(viewer) = inputs.selected else {
        return refused_outright(format!("{what}'s colour script needs a viewing country"));
    };
    let temps = match forced_refresh(mode, inputs) {
        Ok(false) => Temps::Fresh,
        Ok(true) | Err(_) => Temps::Inherited,
    };
    let evaluator = Evaluator::new(world, None).with_temps(temps);
    let (selected, failures) =
        evaluator.capture_colour_inputs(|| targets(layer, inputs, &evaluator, viewer));
    if !failures.is_empty() {
        return LayerState {
            borders: BTreeMap::new(),
            refusals: failures
                .into_iter()
                .map(|why| format!("{what}'s target selection is refused for 1 layer: {why}"))
                .collect(),
            refused: true,
        };
    }
    let targets = match selected {
        Ok(targets) => targets,
        Err(why) => return refused_outright(format!("{what}'s targets are refused: {why}")),
    };
    let mut kept = previous
        .map(|previous| previous.borders.clone())
        .unwrap_or_default();
    kept.retain(|id, _| targets.iter().any(|target| target.id == *id));
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for target in targets {
        let border = kept.entry(target.id).or_insert_with(|| Border {
            provinces: target.provinces.clone(),
            style: None,
        });
        if border.provinces != target.provinces {
            *border = Border {
                provinces: target.provinces.clone(),
                style: None,
            };
        }
        let mut scopes = vec![
            EventScope {
                from: Link::Scope(1),
                ..EventScope::of(EventObject::Country(viewer))
            },
            EventScope {
                from: if target.controller.is_some() {
                    Link::Scope(2)
                } else {
                    Link::Itself
                },
                ..EventScope::of(target.object)
            },
        ];
        if let Some(controller) = target.controller {
            scopes.push(EventScope::of(EventObject::Country(controller)));
        }
        let graph = Rc::new(RefCell::new(EventScopes::new(scopes)));
        let _bound = evaluator.bind_event_scopes(graph);
        let colour = evaluator
            .event_scopes(0)
            .map_err(|why| vec![why.to_string()])
            .and_then(|scopes| colour(&evaluator, layer, scopes));
        match colour {
            Ok(Some(style)) => border.style = Some(style),
            Ok(None) => {}
            Err(reasons) => {
                border.style = None;
                for reason in reasons {
                    *refused.entry(reason).or_default() += 1;
                }
            }
        }
    }
    LayerState {
        borders: kept,
        refusals: refused
            .into_iter()
            .map(|(reason, count)| {
                format!("{what}'s colour script is refused for {count} targets: {reason}")
            })
            .collect(),
        refused: false,
    }
}

pub(super) fn layer(
    layer: &Layer,
    borders: &BTreeMap<TargetId, Border>,
    inputs: &Inputs,
) -> ModeLayer {
    let defines = &inputs.content.defines;
    let len = super::region_table_len(
        inputs.map,
        borders
            .values()
            .flat_map(|border| border.provinces.iter().copied()),
    );
    let mut region_of = vec![0; len];
    let mut styles = BTreeMap::new();
    for (index, border) in borders.values().enumerate() {
        let Some(style) = border.style else {
            continue;
        };
        let key = index as u32 + 1;
        for &province in &border.provinces {
            if let Some(slot) = region_of.get_mut(province as usize) {
                *slot = key;
            }
        }
        styles.insert(key, style);
    }
    finish(
        region_of,
        styles,
        layer.thickness.then_some(defines.scripted_thickness),
        (!layer.thickness).then_some(FLAT_GRADIENT),
        defines.country_channel,
    )
    .unwrap()
}

fn colour(
    evaluator: &Evaluator<'_, '_>,
    layer: &Layer,
    scopes: dashboard_text::triggers::Scopes,
) -> Result<Option<Style>, Vec<String>> {
    let seeds: &[(&[u8], i64)] = if layer.thickness {
        &[(b"thickness", FIXED5)]
    } else {
        &[]
    };
    match evaluator.evaluate_colour(&layer.color, scopes, seeds) {
        Eval::True => {}
        Eval::False => return Ok(None),
        Eval::Unknown(reasons) => {
            return Err(reasons.into_iter().map(|why| why.to_string()).collect());
        }
    }
    let temp = |name: &[u8]| {
        evaluator
            .temp_value(name)
            .unwrap_or(Ok(0))
            .map_err(|why| vec![why.to_string()])
    };
    let unit = |name: &[u8]| -> Result<f32, Vec<String>> {
        Ok(temp(name)?.clamp(0, FIXED5) as f32 / FIXED5 as f32)
    };
    let alpha = unit(b"alpha")?;
    if alpha <= 0.0 {
        return Ok(None);
    }
    let rgba = [unit(b"red")?, unit(b"green")?, unit(b"blue")?, alpha];
    Ok(Some(Style {
        color: pack(rgba),
        pulse: temp(b"highlighted")? != 0,
        fill: if layer.thickness {
            temp(b"thickness")? as f32 / FIXED5 as f32
        } else {
            1.0
        },
    }))
}

fn targets(
    layer: &Layer,
    inputs: &Inputs,
    evaluator: &Evaluator<'_, '_>,
    viewer: u32,
) -> Result<Vec<Target>, String> {
    let chosen = if let Some(block) = layer.targets.as_ref().filter(|block| !block.is_empty()) {
        let selection = content_reader_politics::country_scorers::script_targets(block);
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(viewer),
        )])));
        let _bound = evaluator.bind_event_scopes(graph);
        let caller = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
        Some(if layer.kind == LayerType::Country {
            evaluator
                .script_target_candidates(&selection, caller)
                .map_err(|why| why.to_string())?
        } else {
            evaluator
                .script_state_target_candidates(&selection, caller)
                .map_err(|why| why.to_string())?
        })
    } else {
        None
    };
    let provinces = core::<tables::Provinces>(inputs.frame).map_err(|e| e.to_string())?;
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(inputs.frame)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let controller = |raw: u32| (raw > 0).then(|| links.get(&raw).copied()).flatten();
    let targets = match layer.kind {
        LayerType::Country => {
            let mut by_country: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for province in provinces {
                if let Some(slot) = controller(province.controller) {
                    if chosen
                        .as_ref()
                        .is_some_and(|chosen| chosen.binary_search(&slot).is_err())
                    {
                        continue;
                    }
                    by_country.entry(slot).or_default().push(province.id);
                }
            }
            by_country
                .into_iter()
                .map(|(slot, provinces)| Target {
                    id: TargetId::Country(slot),
                    object: EventObject::Country(slot),
                    controller: None,
                    provinces,
                })
                .collect::<Vec<_>>()
        }
        LayerType::State | LayerType::StateController => {
            let mut by_state: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
            for province in provinces {
                if province.state != 0 {
                    if chosen
                        .as_ref()
                        .is_some_and(|chosen| chosen.binary_search(&province.state).is_err())
                    {
                        continue;
                    }
                    by_state
                        .entry(province.state)
                        .or_default()
                        .push((province.id, province.controller));
                }
            }
            let mut targets = Vec::new();
            for (state, members) in by_state {
                let ids = members.iter().map(|(id, _)| *id).collect();
                if layer.kind == LayerType::State {
                    targets.push(Target {
                        id: TargetId::State(state),
                        object: EventObject::State(state),
                        controller: None,
                        provinces: ids,
                    });
                    continue;
                }
                let row = core::<tables::States>(inputs.frame)
                    .map_err(|why| why.to_string())?
                    .iter()
                    .find(|row| row.id == state)
                    .ok_or_else(|| format!("States lacks state {state}"))?;
                let single = row
                    .single_controller
                    .require("States.single_controller")?
                    .ok_or_else(|| {
                        format!("state {state} cached controller grouping unavailable")
                    })?;
                if single != 0 {
                    let slot = if row.controller == 0 {
                        0
                    } else {
                        controller(row.controller)
                            .ok_or_else(|| format!("state {state} controller is unresolved"))?
                    };
                    targets.push(Target {
                        id: TargetId::Portion {
                            state,
                            controller: slot,
                        },
                        object: EventObject::State(state),
                        controller: Some(slot),
                        provinces: ids,
                    });
                } else {
                    let mut groups = BTreeMap::<u32, Vec<u32>>::new();
                    for (id, raw) in members {
                        if (raw as i32) > 0 {
                            let slot = controller(raw)
                                .ok_or_else(|| format!("province {id} controller is unresolved"))?;
                            groups.entry(slot).or_default().push(id);
                        }
                    }
                    for (slot, provinces) in groups {
                        targets.push(Target {
                            id: TargetId::Portion {
                                state,
                                controller: slot,
                            },
                            object: EventObject::State(state),
                            controller: Some(slot),
                            provinces,
                        });
                    }
                }
            }
            targets
        }
        LayerType::None | LayerType::Game(_) => Vec::new(),
    };
    Ok(targets)
}
