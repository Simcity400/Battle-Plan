use super::{Inputs, pack};
#[cfg(test)]
use map_display_data::landscape::ColourState;
use map_display_data::landscape::Style;
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
use dashboard_text::triggers::{Eval, Evaluator, Temps};
use content_reader_map::scripted_map_modes::{Layer, LayerType};
use replay_data::rows::tables;
use std::cell::RefCell;
use std::rc::Rc;
use viewer_foundation::tables::core;

const FIXED5: i64 = 100_000;

#[cfg(test)]
const HOURS_PER_DAY: i32 = 24;

pub(super) const FLAT_GRADIENT: u8 = 244;

pub(super) use map_display_data::landscape::TargetId;
#[cfg(test)]
pub(super) use map_display_data::landscape::{Border, LayerState};

#[cfg(test)]
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

pub(super) fn refresh_interval(
    mode: &str,
    inputs: &Inputs,
) -> Option<map_display_data::identities::RefreshInterval> {
    use map_display_data::{
        Availability,
        identities::{EventKey, RefreshInterval, RefreshOccurrence},
    };
    use replay_data::rows::{MapEventFamily, MapEventPayload};
    forced_refresh(mode, inputs).ok()?;
    let [interval] = core::<tables::MapEvents>(inputs.frame).ok()? else {
        return None;
    };
    let occurrences: Vec<_> = interval
        .events
        .iter()
        .filter_map(|event| {
            let MapEventPayload::ScriptedMapRefresh(declaration) = event.payload else {
                return None;
            };
            Some(RefreshOccurrence {
                declaration,
                key: EventKey {
                    branch: inputs.frame.branch,
                    epoch: interval.epoch,
                    sequence: event.sequence,
                    hour: event.hour,
                    kind: MapEventFamily::ScriptedMapRefresh.index() as u8,
                },
            })
        })
        .collect();
    Some(RefreshInterval {
        branch: inputs.frame.branch,
        epoch: interval.epoch,
        from_hour: interval.from_hour,
        to_hour: interval.to_hour,
        first: interval.first_sequence,
        next: interval.next_sequence,
        status: if occurrences.is_empty() {
            Availability::KnownEmpty {}
        } else {
            Availability::Known {}
        },
        occurrences,
    })
}

mod fold;
mod routes;
mod targets;
pub(super) use fold::Fold;

pub(super) fn close(
    mode: &str,
    side: &str,
    layer: &Layer,
    inputs: &Inputs,
) -> std::io::Result<map_display_data::landscape::LayerProgram> {
    let declared = inputs.content.scripted.get(mode).is_some_and(|mode| {
        std::ptr::eq(
            layer,
            if side == "top" {
                &raw const mode.top
            } else {
                &raw const mode.bottom
            },
        )
    });
    if declared {
        inputs
            .content
            .scripts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .close(mode, side, layer, inputs)
    } else {
        Fold::default().close(mode, side, layer, inputs)
    }
}

pub(super) fn close_lazy<'a, 'w>(
    mode: &str,
    side: &str,
    layer: &Layer,
    inputs: &Inputs<'a, 'w>,
    world: &dyn Fn() -> &'a dashboard_text::triggers::World<'w>,
) -> std::io::Result<map_display_data::landscape::LayerProgram> {
    inputs
        .content
        .scripts
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .close_lazy(mode, side, layer, inputs, world)
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

fn choose(
    layer: &Layer,
    evaluator: &Evaluator<'_, '_>,
    viewer: u32,
) -> Result<Option<Vec<u32>>, String> {
    let Some(block) = layer.targets.as_ref().filter(|block| !block.is_empty()) else {
        return Ok(None);
    };
    let selection = content_reader_politics::country_scorers::script_targets(block);
    let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
        EventObject::Country(viewer),
    )])));
    let _bound = evaluator.bind_event_scopes(graph);
    let caller = evaluator.event_scopes(0).map_err(|why| why.to_string())?;
    Some(if layer.kind == LayerType::Country {
        evaluator
            .script_target_candidates(&selection, caller)
            .map_err(|why| why.to_string())
    } else {
        evaluator
            .script_state_target_candidates(&selection, caller)
            .map_err(|why| why.to_string())
    })
    .transpose()
}

#[cfg(test)]
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
        map_display_data::landscape::apply_layer(&close(mode, side, layer, inputs).unwrap(), previous)
    } else {
        previous.cloned().unwrap_or_default()
    };
    let mut expected_warnings = Vec::new();
    let expected = super::colour_oracle::borders(
        mode,
        side,
        layer,
        inputs,
        previous,
        refresh,
        &mut expected_warnings,
    );
    assert_eq!(state, expected);
    assert_eq!(state.refusals, expected_warnings);
    warnings.extend(state.refusals.iter().cloned());
    state
}
