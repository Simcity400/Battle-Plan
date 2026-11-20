//! A viewing-country switch timed in the perf map log (`BATTLE_PLAN_PERF_MAP_LOG`): where it began (a
//! Ctrl+click here, or the host's options), each frame's parts not yet the new country's, and its
//! first changed and settled frames, judged as the `--viewpoint-switch` harness judges them.
use super::Viewer;
use super::playback_cost::viewpoint::{Target, seen};
use crate::playhead_host::CaughtUp;
use serde_json::json;
use std::time::{Duration, Instant};

/// A switch traced for longer than this is given up and marked so.
const GIVE_UP: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct SwitchTrace {
    switch: Option<Switch>,
    layers: Vec<CaughtUp>,
    names: Vec<&'static str>,
}

struct Switch {
    target: Target,
    started: Instant,
    first_changed: bool,
    /// The step whose compose was last marked.
    composed: Option<u64>,
}

impl Viewer {
    /// The viewing country asked to become `slot`, from `from`; a switch to the slot already traced
    /// goes on.
    pub(super) fn trace_switch(&mut self, slot: u32, tag: &str, from: &'static str) {
        if !crate::perf_log::enabled() {
            return;
        }
        crate::perf_log::mark(from, json!({"slot": slot, "tag": tag}));
        if self.switch_trace.switch.as_ref().is_some_and(|switch| switch.target.slot == slot) {
            return;
        }
        self.switch_trace.switch = Some(Switch { target: Target { slot, tag: tag.to_owned() }, started: Instant::now(), first_changed: false, composed: None });
    }

    /// After a frame: what of the traced switch it shows.
    pub(super) fn trace_switch_frame(&mut self) {
        let Some(switch) = self.switch_trace.switch.as_ref() else {
            return;
        };
        let mut layers = std::mem::take(&mut self.switch_trace.layers);
        let seen = seen(self, Some(&switch.target), self.playhead, self.playing, &mut layers);
        self.switch_trace.layers = layers;
        let Some(switch) = self.switch_trace.switch.as_mut() else {
            return;
        };
        let ms = switch.started.elapsed().as_secs_f64() * 1e3;
        if let Some(side) = self.playhead_map.as_ref()
            && let Some(names) = side.try_with_side(|side| side.names().to_vec())
            && !names.is_empty()
        {
            self.switch_trace.names = names;
        }
        if let Some(side) = self.playhead_map.as_ref()
            && let Some(swapped) = side.last_frame().swapped.clone()
            && switch.composed != Some(side.last_frame().made)
        {
            switch.composed = Some(side.last_frame().made);
            let mut spent: Vec<(&str, f64)> = self.switch_trace.names.iter().copied().zip(swapped.iter().map(|&ns| ns as f64 / 1e6)).filter(|(_, ms)| *ms > 0.0).collect();
            spent.sort_by(|a, b| b.1.total_cmp(&a.1));
            crate::perf_log::mark("switch_composed", json!({"slot": switch.target.slot, "ms": ms, "layers_ms": spent}));
        }
        let behind: Vec<&str> = seen.parts.iter().filter(|(_, good, _)| !good).map(|(part, ..)| part.as_str()).collect();
        let new = seen.parts.iter().any(|(_, _, new)| *new);
        crate::perf_log::mark("switch_frame", json!({"slot": switch.target.slot, "ms": ms, "whole": seen.whole, "drawn": self.drew_frame, "new": new, "behind": behind}));
        if new && !switch.first_changed {
            switch.first_changed = true;
            crate::perf_log::mark("switch_first_changed", json!({"slot": switch.target.slot, "ms": ms}));
        }
        if seen.settled() {
            crate::perf_log::mark("switch_settled", json!({"slot": switch.target.slot, "ms": ms}));
            self.switch_trace.switch = None;
        } else if switch.started.elapsed() > GIVE_UP {
            crate::perf_log::mark("switch_given_up", json!({"slot": switch.target.slot, "ms": ms, "behind": behind}));
            self.switch_trace.switch = None;
        }
    }
}
