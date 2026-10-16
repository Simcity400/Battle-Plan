//! A scripted map mode's `country`, `state` and `state_controller` layers: each target's `color`
//! callback, run through the shared trigger evaluator, colours its own region.
//!
//! The callback runs in an event scope whose ROOT, THIS and PREV are the viewing country and whose
//! FROM is the target: the country, or the state, whose own FROM is itself, or for a
//! `state_controller` layer the controller of that portion of the state. A callback that holds
//! leaves its colour in the temps `red`, `green`, `blue` and `alpha` (each clamped to 0..1, unset
//! reading 0), `highlighted` (pulsing when non-zero) and, with `thickness = yes`, `thickness`,
//! set to 1 before the callback runs and used unclamped as the region's fill factor. A callback
//! the evaluator cannot decide is refused, counted by its reason.
//!
//! Each target's border is kept between refreshes, as the game keeps it. A refresh first compares
//! the target's province list with the kept one and clears the border when it changed; the colour
//! is then set only when the callback holds with an alpha above 0, so a callback that fails keeps
//! the colour of an unchanged border. The game refreshes when the mode is selected or loaded and,
//! with `update_daily` (its default), at each day's first hour; between refreshes the kept
//! borders show as they are. A replay's seek or fresh open acts as a load and starts from empty
//! borders; playing forward carries them across the days.
use super::{Inputs, ModeLayer, Style, finish, pack};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
use dashboard_text::triggers::{Eval, Evaluator};
use installed_content::scripted_map_modes::{Layer, LayerType};
use replay::rows::tables;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::rc::Rc;
use viewer_foundation::map::tables::core;

/// One in fixed5.
const FIXED5: i64 = 100_000;

/// Hours in a day: the daily refresh runs at each day's first hour, hour index 0.
const HOURS_PER_DAY: i32 = 24;

/// The flat gradient byte a layer without `thickness = yes` fills its regions with.
pub(super) const FLAT_GRADIENT: u8 = 244;

/// Which object a border belongs to: equal colours of two targets stay two regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetId {
    Country(u32),
    State(u32),
    /// A state's portion under one controller.
    Portion {
        state: u32,
        controller: u32,
    },
}

/// One target's kept border: the province list its last refresh saw, and its colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Border {
    pub(super) provinces: Vec<u32>,
    pub(super) style: Option<Style>,
}

/// One colour layer's kept borders and why its last refresh refused what it refused, reported
/// on every hour the borders show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerState {
    pub(super) borders: BTreeMap<TargetId, Border>,
    pub(super) refusals: Vec<String>,
    /// The whole layer is refused: it draws nothing of its own.
    pub(super) refused: bool,
}

/// A scripted mode's kept borders, the bottom layer's then the top's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColourState {
    /// The scripted mode the borders belong to.
    pub mode: String,
    pub layers: [LayerState; 2],
}

/// Whether this hour refreshes the mode's colour layers given the previous hour's borders: a
/// first hour (no previous, as after a seek, an open or a selection) always does; otherwise the
/// day's first hour does when the mode updates daily.
pub(super) fn refreshes(previous: Option<&ColourState>, update_daily: bool, hour: i32) -> bool {
    previous.is_none() || (update_daily && hour.rem_euclid(HOURS_PER_DAY) == 0)
}

/// What one target paints, and the scope its callback runs in.
struct Target {
    id: TargetId,
    object: EventObject,
    /// A `state_controller` portion's controller, its FROM's FROM.
    controller: Option<u32>,
    provinces: Vec<u32>,
}

/// The layer's kept borders after this hour: refreshed from `previous` when `refresh`, else
/// `previous` as it is. The state's refusals are pushed to `warnings` on every hour, refreshed or
/// not; a layer refused outright keeps no borders.
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
    let evaluator = Evaluator::new(world, None);
    let targets = match targets(layer, inputs, &evaluator, viewer) {
        Ok(targets) => targets,
        Err(why) => return refused_outright(format!("{what}'s targets are refused: {why}")),
    };
    let mut kept = previous
        .map(|previous| previous.borders.clone())
        .unwrap_or_default();
    // A target gone this hour, as a country that lost its last controlled province, keeps no
    // border: its old provinces are another target's now.
    kept.retain(|id, _| targets.iter().any(|target| target.id == *id));
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for target in targets {
        // A changed province list clears the border before the callback runs.
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
            .map_err(|why| vec![why])
            .and_then(|scopes| colour(&evaluator, layer, scopes));
        match colour {
            Ok(Some(style)) => border.style = Some(style),
            // A failing callback, or one leaving alpha at 0, keeps what the border holds.
            Ok(None) => {}
            Err(reasons) => {
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

/// The kept borders as a gradient layer: each coloured border its own region.
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
}

/// The callback's colour for one target: `None` when it does not hold or its alpha is 0.
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
    match evaluator.evaluate_seeded(&layer.color, scopes, seeds) {
        Eval::True => {}
        Eval::False => return Ok(None),
        Eval::Unknown(reasons) => return Err(reasons.into_iter().collect()),
    }
    let temp = |name: &[u8]| {
        evaluator
            .temp_value(name)
            .unwrap_or(Ok(0))
            .map_err(|why| vec![why])
    };
    let unit = |name: &[u8]| -> Result<f32, Vec<String>> {
        Ok(temp(name)?.clamp(0, FIXED5) as f32 / FIXED5 as f32)
    };
    let rgba = [
        unit(b"red")?,
        unit(b"green")?,
        unit(b"blue")?,
        unit(b"alpha")?,
    ];
    if rgba[3] <= 0.0 {
        return Ok(None);
    }
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

/// The targets a layer paints: every country with a controlled province, every state, or every
/// portion of a state under one controller, narrowed by the layer's `targets` block.
fn targets(
    layer: &Layer,
    inputs: &Inputs,
    evaluator: &Evaluator<'_, '_>,
    viewer: u32,
) -> Result<Vec<Target>, String> {
    let provinces = core::<tables::Provinces>(inputs.frame).map_err(|e| e.to_string())?;
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(inputs.frame)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let controller = |raw: u32| (raw > 0).then(|| links.get(&raw).copied()).flatten();
    let mut targets = match layer.kind {
        LayerType::Country => {
            let mut by_country: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for province in provinces {
                if let Some(slot) = controller(province.controller) {
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
            let mut by_state: BTreeMap<u32, Vec<(u32, Option<u32>)>> = BTreeMap::new();
            for province in provinces {
                if province.state != 0 {
                    by_state
                        .entry(province.state)
                        .or_default()
                        .push((province.id, controller(province.controller)));
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
                let mut controllers: Vec<u32> =
                    members.iter().filter_map(|(_, slot)| *slot).collect();
                controllers.sort_unstable();
                controllers.dedup();
                match controllers.as_slice() {
                    [] => {}
                    [only] => targets.push(Target {
                        id: TargetId::Portion {
                            state,
                            controller: *only,
                        },
                        object: EventObject::State(state),
                        controller: Some(*only),
                        provinces: ids,
                    }),
                    // The game splits a divided state by its cached controller groups, which
                    // recordings do not hold.
                    _ => {
                        return Err(format!(
                            "state {state} is divided between controllers, whose grouping is not recorded (state_controller)"
                        ));
                    }
                }
            }
            targets
        }
        LayerType::None | LayerType::Game(_) => Vec::new(),
    };
    let Some(block) = &layer.targets else {
        return Ok(targets);
    };
    if layer.kind != LayerType::Country {
        return Err("a targets block on a state layer".into());
    }
    let selection = installed_content::country_scorers::script_targets(block);
    let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
        EventObject::Country(viewer),
    )])));
    let _bound = evaluator.bind_event_scopes(graph);
    let caller = evaluator.event_scopes(0)?;
    let chosen = evaluator.script_target_candidates(&selection, caller)?;
    targets.retain(|target| {
        matches!(target.id, TargetId::Country(slot) if chosen.binary_search(&slot).is_ok())
    });
    Ok(targets)
}
