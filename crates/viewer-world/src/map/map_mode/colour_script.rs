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
//! with `update_daily` enabled, at each day's first hour; between refreshes the kept
//! borders show as they are. A replay's seek or fresh open acts as a load and starts from empty
//! borders; playing forward carries them across the days.
use super::{Inputs, Style, pack};
use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
use dashboard_text::triggers::{Eval, Evaluator, Temps};
use installed_content_core::scripted_map_modes::{Layer, LayerType};
use replay_data::rows::tables;
use std::cell::RefCell;
use std::rc::Rc;
use viewer_foundation::map::tables::core;

/// One in fixed5.
const FIXED5: i64 = 100_000;

/// Hours in a day: the daily refresh runs at each day's first hour, hour index 0.
#[cfg(test)]
const HOURS_PER_DAY: i32 = 24;

/// The flat gradient byte a layer without `thickness = yes` fills its regions with.
pub(super) const FLAT_GRADIENT: u8 = 244;

pub use map_display_data::landscape::ColourState;
pub(super) use map_display_data::landscape::TargetId;
#[cfg(test)]
pub(super) use map_display_data::landscape::{Border, LayerState};

/// Whether this hour refreshes the mode's colour layers given the previous hour's borders: a
/// first hour (no previous, as after a seek, an open or a selection) always does; otherwise the
/// day's first hour does when the mode updates daily.
#[cfg(test)]
pub(super) fn refreshes(previous: Option<&ColourState>, update_daily: bool, hour: i32) -> bool {
    previous.is_none() || (update_daily && hour.rem_euclid(HOURS_PER_DAY) == 0)
}

/// Actual native refresh calls in this hour's event interval, after the effect's gates.
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
                &mode.top
            } else {
                &mode.bottom
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
    // Alpha gates all the other output reads. In particular a proved zero
    // alpha does not need inherited RGB/highlight values.
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
    let selection = installed_content_core::country_scorers::script_targets(block);
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
        super::resolved::apply_layer(&close(mode, side, layer, inputs).unwrap(), previous)
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
