//! Maps beside the harness's own, as a dashboard draws them, set by the environment:
//! `BATTLE_PLAN_EXTRA_MAPS=<n>` more readers of the map's playhead, each opened off the frame thread as
//! the window opens one and drawn every frame at the map's view and hour once it arrives;
//! `BATTLE_PLAN_EXTRA_RECORDING=<file>` one map of another recording, on a playhead of its own playing on
//! its clock; `BATTLE_PLAN_MAP_CHURN=<frames>` the extra maps closed and opened again every so many
//! frames; `BATTLE_PLAN_HIDE=extra|map:<from>:<frames>` the first extra map scrolled out, or the harness's
//! own map covered, for so many frames from frame `from`, then shown again;
//! `BATTLE_PLAN_AS_DASHBOARD_MAP=1` the harness's own map drawn as a dashboard map. What they cost is the
//! report's `extra_maps`.

use crate::{
    document::Document,
    playback_cost::distribution,
    playhead_host::{self, Entry, Head, MapSide, Opening, side::SideThread},
};
use replay_data::Hour;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Instant};
use viewer_foundation::profiling::current_thread_cpu_time;

fn number(name: &str) -> io::Result<Option<u64>> {
    match std::env::var(name) {
        Ok(text) if !text.trim().is_empty() => text.trim().parse().map(Some).map_err(|_| io::Error::other(format!("{name}: {text} is not a number"))),
        _ => Ok(None),
    }
}

/// Whether the harness's map is drawn as a dashboard map.
pub(super) fn as_dashboard_map() -> bool {
    std::env::var("BATTLE_PLAN_AS_DASHBOARD_MAP").or_else(|_| std::env::var("HOI4_AS_DASHBOARD_MAP")).is_ok_and(|value| value == "1")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hidden {
    Extra,
    Map,
}

/// A map hidden for a while and shown again: when it was shown, when its reader arrived, when it
/// drew its first picture, and what the playhead held while it was hidden.
#[derive(Default)]
struct Shown {
    at: Option<u64>,
    arrived: Option<u64>,
    pictured: Option<u64>,
    /// Per hidden frame: the playhead's bytes, its ring's steps and bytes, its lanes.
    hidden: Vec<(usize, usize, usize, usize)>,
    /// Per frame after the first picture: the hours its layers are behind the head.
    lag: Vec<f64>,
}

/// An extra map drawn, and the reader opening whose first picture it has not drawn yet.
struct Drawn {
    side: SideThread,
    opened: Option<usize>,
    /// The hidden map shown again, until its first picture and its lag after.
    watched: bool,
}

struct Pending {
    opening: Opening,
    started: u64,
    /// Whether it is the hidden extra map shown again.
    shown: bool,
}

#[derive(Default)]
pub(super) struct Extras {
    wanted: usize,
    churn: Option<u64>,
    hide: Option<(Hidden, u64, u64)>,
    entry: Option<Arc<Entry>>,
    sides: Vec<Drawn>,
    pending: Vec<Pending>,
    other: Option<SideThread>,
    /// Per frame: the extra maps' frames on the window thread, the other timeline's, the map's step
    /// built by the stepper (most a step).
    draw_ms: Vec<f64>,
    /// The extra maps' frames, the window thread's own CPU, milliseconds.
    draw_cpu_ms: Vec<f64>,
    other_ms: Vec<f64>,
    step_ms: Vec<f64>,
    events: Vec<(u64, &'static str, f64)>,
    /// Per frame with an opening started, waiting or arriving: the frame thread's CPU and wall
    /// time on it (start, poll, install), microseconds.
    opening_work: Vec<(u64, f64, f64)>,
    /// Per reader opened: frames from its start to its arrival, and to its first picture.
    opened: Vec<(u64, Option<u64>, Option<u64>)>,
    /// The map's own frames: (frame, lag in hours of what its layers show, resets, steps).
    map: Vec<(u64, Option<f64>, u32, u32)>,
    shown: Shown,
    held: Vec<Value>,
}

impl Extras {
    /// The extras the environment asks for beside `main`, the harness's map of `document`.
    pub(super) fn open(document: &Document, main: Option<&SideThread>, speed: f64) -> io::Result<Self> {
        let wanted = number("BATTLE_PLAN_EXTRA_MAPS")?.unwrap_or(0) as usize;
        let churn = number("BATTLE_PLAN_MAP_CHURN")?.filter(|&frames| frames > 0);
        let hide = match std::env::var("BATTLE_PLAN_HIDE").or_else(|_| std::env::var("HOI4_HIDE")) {
            Ok(text) if !text.trim().is_empty() => {
                let parts: Vec<&str> = text.trim().split(':').collect();
                let which = match parts.first() {
                    Some(&"extra") => Hidden::Extra,
                    Some(&"map") => Hidden::Map,
                    _ => return Err(io::Error::other(format!("BATTLE_PLAN_HIDE takes extra|map:<from>:<frames>, not {text}"))),
                };
                let at = |index: usize| parts.get(index).and_then(|part| part.parse().ok()).ok_or_else(|| io::Error::other(format!("BATTLE_PLAN_HIDE: {text} names no frames")));
                Some((which, at(1)?, at(2)?))
            }
            _ => None,
        };
        if (wanted > 0 || hide.is_some()) && main.is_none() {
            return Err(io::Error::other("BATTLE_PLAN_EXTRA_MAPS and BATTLE_PLAN_HIDE need the map's playhead, which is off or refused"));
        }
        if hide.is_some_and(|(which, _, _)| which == Hidden::Extra) && wanted == 0 {
            return Err(io::Error::other("BATTLE_PLAN_HIDE=extra needs an extra map: set BATTLE_PLAN_EXTRA_MAPS"));
        }
        let mut extras = Self { wanted, churn, hide, entry: main.map(|side| Arc::clone(side.entry())), ..Self::default() };
        extras.start(document, 0, false);
        if let Ok(path) = std::env::var("BATTLE_PLAN_EXTRA_RECORDING").or_else(|_| std::env::var("HOI4_EXTRA_RECORDING"))
            && !path.trim().is_empty()
        {
            let (units, _) = playhead_host::units::at_path(std::path::Path::new(path.trim()), None)?;
            let first = units.timeline.first();
            let entry = playhead_host::open(units, playhead_host::settings())?;
            entry.set_capture(document.capture_mode());
            let side = SideThread::new(MapSide::new(Arc::clone(&entry), Self::layers(document), main.map(|side| side.view().clone()).unwrap_or_default())?)?;
            entry.anchor(first, true, speed, true);
            extras.other = Some(side);
        }
        Ok(extras)
    }

    fn active(&self) -> bool {
        self.wanted > 0 || self.other.is_some() || self.hide.is_some()
    }

    /// The window's layers for another recording's map.
    fn layers(document: &Document) -> Vec<Box<dyn crate::layers::Layer>> {
        let content = Arc::new(crate::layers::political::Content::new(Arc::clone(&document.content.map), Arc::clone(&document.border_meshes)));
        let place_in = Arc::clone(&document.content);
        vec![
            Box::new(crate::layers::political::Political::new(content)),
            Box::new(crate::layers::sounds::SoundsFed::new(crate::layers::sounds::Sounds::new(document.content.audio.as_ref().map(|source| Arc::clone(&source.content))))),
            Box::new(crate::layers::strikes::Strikes::new(Arc::new(move |province| place_in.products.impact_place(province)))),
        ]
    }

    /// Starts the readers wanted that are neither drawn nor being made, as a dashboard map opens
    /// one: on a worker, joining the playhead already anchored at its clock.
    fn start(&mut self, document: &Document, index: u64, shown: bool) {
        while self.sides.len() + self.pending.len() < self.wanted {
            match Opening::start(document, None) {
                Ok(opening) => self.pending.push(Pending { opening, started: index, shown }),
                Err(error) => {
                    eprintln!("An extra map's playhead is refused: {error}");
                    break;
                }
            }
        }
    }

    /// Whether the harness's own map is covered this frame.
    pub(super) fn main_hidden(&self, index: u64) -> bool {
        self.hide.is_some_and(|(which, from, frames)| which == Hidden::Map && (from..from + frames).contains(&index))
    }

    /// The playhead's clock, for frames whose map is covered.
    pub(super) fn head(&self, now: Instant) -> Option<Hour> {
        self.entry.as_ref().map(|entry| entry.playhead().head(now))
    }

    /// One frame of every extra map, after the harness's map drew at `head` (none while covered).
    pub(super) fn frame(&mut self, document: &Document, main: Option<&SideThread>, index: u64, head: Head) -> Value {
        if !self.active() {
            return Value::Null;
        }
        let _cost = replay_frame::cpu::scope("frame.extra_maps", Some("other"));
        let (cpu, wall) = (current_thread_cpu_time(), Instant::now());
        let mut event = None;
        let mut worked = !self.pending.is_empty();
        if let Some(every) = self.churn
            && index > 0
            && index.is_multiple_of(every)
        {
            let what = if self.sides.is_empty() && self.pending.is_empty() {
                self.start(document, index, false);
                "opened"
            } else {
                // Let go off the frame thread, as the window's dropper does.
                let closed = (std::mem::take(&mut self.sides), std::mem::take(&mut self.pending).into_iter().map(|pending| pending.opening).collect::<Vec<_>>());
                std::thread::spawn(move || drop(closed));
                "closed"
            };
            event = Some(what);
            worked = true;
        }
        if let Some((Hidden::Extra, from, frames)) = self.hide {
            if index == from && !self.sides.is_empty() {
                let closed = self.sides.remove(0);
                std::thread::spawn(move || drop(closed));
                self.wanted -= 1;
                event = Some("hidden");
            } else if index == from + frames {
                self.wanted += 1;
                self.start(document, index, true);
                self.shown.at = Some(index);
                event = Some("shown");
                worked = true;
            }
        }
        let mut arrived = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            match self.pending[at].opening.poll() {
                Some(made) => {
                    let pending = self.pending.remove(at);
                    if let Ok(side) = made.and_then(SideThread::new) {
                        if pending.shown {
                            self.shown.arrived = Some(index);
                        }
                        self.sides.push(Drawn { side, opened: Some(self.opened.len()), watched: pending.shown });
                        arrived.push(pending.started);
                        self.opened.push((pending.started, Some(index - pending.started), None));
                    }
                }
                None => at += 1,
            }
        }
        if worked || !arrived.is_empty() {
            let cpu_us = current_thread_cpu_time().zip(cpu).map_or(f64::NAN, |(now, then)| now.saturating_sub(then).as_secs_f64() * 1e6);
            self.opening_work.push((index, cpu_us, wall.elapsed().as_secs_f64() * 1e6));
        }
        let view = main.or(self.sides.first().map(|drawn| &drawn.side)).map(|side| side.view().clone()).unwrap_or_default();
        let (started, draw_cpu) = (Instant::now(), current_thread_cpu_time());
        for drawn in &mut self.sides {
            drawn.side.ask(view.clone(), head, None, None);
            if let Some(mut window) = window_for(&mut drawn.side, head) {
                window.take(Default::default());
            }
        }
        let draw = started.elapsed().as_secs_f64() * 1e3;
        let draw_cpu = current_thread_cpu_time().zip(draw_cpu).map(|(now, then)| now.saturating_sub(then).as_secs_f64() * 1e3);
        for drawn in &mut self.sides {
            // An extra map has no scene to upload to, so its first picture is its layers composed
            // and drawing; its lag is what it has applied behind the head.
            if !drawn.side.draws("political") {
                continue;
            }
            let Some(shown) = drawn.side.applied() else { continue };
            if let Some(at) = drawn.opened.take() {
                self.opened[at].2 = Some(index - self.opened[at].0);
            }
            if drawn.watched {
                self.shown.pictured.get_or_insert(index);
                self.shown.lag.push(f64::from(drawn.side.last_frame().head) - f64::from(shown));
                drawn.watched = self.shown.lag.len() < 120;
            }
        }
        let other = self.other.as_mut().map(|side| {
            let started = Instant::now();
            side.ask(view.clone(), Head::Clock, None, None);
            if let Some(mut window) = side.window() {
                window.take(Default::default());
            }
            started.elapsed().as_secs_f64() * 1e3
        });
        self.draw_ms.push(draw);
        self.draw_cpu_ms.extend(draw_cpu);
        self.other_ms.extend(other);
        if let Some(what) = event {
            let ms = wall.elapsed().as_secs_f64() * 1e3 - draw;
            self.events.push((index, what, ms));
        }
        self.track_hidden(main, index);
        if let Some(side) = main {
            let frame = side.last_frame();
            if frame.steps > 0 {
                self.step_ms.push(frame.step_most_ns as f64 / 1e6);
            }
            let lag = frame.drawn.map(|drawn| f64::from(frame.head) - f64::from(drawn));
            self.map.push((index, lag, frame.resets, frame.steps));
        }
        if index.is_multiple_of(60)
            && let Some(entry) = &self.entry
        {
            self.held.push(self.held_now(entry, main));
        }
        json!({"extra_draw_ms": draw, "extra_draw_cpu_ms": draw_cpu, "other_ms": other, "event": event, "main_drawn": main.is_some()})
    }

    /// While a map is hidden, what the playhead holds; once the covered map is shown again, when
    /// its reader arrives, when it first draws, and its lag after.
    fn track_hidden(&mut self, main: Option<&SideThread>, index: u64) {
        let Some((which, from, frames)) = self.hide else { return };
        if (from..from + frames).contains(&index)
            && let Some(entry) = &self.entry
        {
            let (now, _, _) = entry.playhead().held();
            self.shown.hidden.push((now.total, now.ring_steps, now.ring_bytes, now.lanes));
        }
        if which == Hidden::Map && index >= from + frames {
            self.shown.at.get_or_insert(from + frames);
            if let Some(side) = main {
                self.shown.arrived.get_or_insert(index);
                if side.draws("political")
                    && let Some(drawn) = side.last_frame().drawn
                    && self.shown.lag.len() < 120
                {
                    self.shown.pictured.get_or_insert(index);
                    self.shown.lag.push(f64::from(side.last_frame().head) - f64::from(drawn));
                }
            }
        }
    }

    fn held_now(&self, entry: &Entry, main: Option<&SideThread>) -> Value {
        let (now, _, _) = entry.playhead().held();
        let side = |thread: &SideThread| {
            thread.with_side(|side| json!({
                "mirrors": side.held().mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>(), "layers": side.held().layers.iter().map(|(_, bytes)| bytes).sum::<usize>(),
                "applied": side.applied(), "composing": side.composing(), "draws_political": side.draws("political"),
                "refused": side.refusals().map(|(name, why)| format!("{name}: {why}")).collect::<Vec<_>>(),
            }))
        };
        json!({
            "playhead": now.total, "playhead_lanes": now.lanes, "ring_steps": now.ring_steps, "ring_bytes": now.ring_bytes,
            "map": main.map(side), "extras": self.sides.iter().map(|drawn| side(&drawn.side)).collect::<Vec<_>>(),
            "other_playhead": self.other.as_ref().map(|other| other.entry().playhead().held().0.total),
            "other": self.other.as_ref().map(side),
        })
    }

    /// What the extra maps cost over the run, and how the map's own frames fared around each
    /// opening, closing, hiding and showing.
    pub(super) fn summary(&self) -> Value {
        if !self.active() {
            return Value::Null;
        }
        let near = |index: u64| self.events.iter().any(|&(at, _, _)| index >= at && index < at + 10);
        let lag = |inside: bool| distribution(self.map.iter().filter(|(index, ..)| near(*index) == inside).filter_map(|(_, lag, ..)| *lag).collect());
        let (peak, counters) = self.entry.as_ref().map(|entry| {
            let (_, peak, counters) = entry.playhead().held();
            (peak.total, counters.stalls)
        }).unzip();
        let hidden = &self.shown.hidden;
        json!({
            "maps_of_the_playhead": 1 + self.wanted, "churn_frames": self.churn, "other_timeline": self.other.is_some(),
            "extra_draw_ms": distribution(self.draw_ms.clone()),
            "extra_draw_cpu_ms": distribution(self.draw_cpu_ms.clone()),
            "other_draw_ms": distribution(self.other_ms.clone()),
            "step_most_ms": distribution(self.step_ms.clone()),
            "events": self.events.iter().map(|(index, what, ms)| json!({"frame": index, "what": what, "ms": ms})).collect::<Vec<_>>(),
            "opening_work_cpu_us": distribution(self.opening_work.iter().map(|work| work.1).filter(|us| us.is_finite()).collect()),
            "opening_work_wall_us": distribution(self.opening_work.iter().map(|work| work.2).collect()),
            "opening_frames": self.opening_work.len(),
            "opened": self.opened.iter().map(|(started, arrived, pictured)| json!({"started": started, "frames_to_arrive": arrived, "frames_to_picture": pictured})).collect::<Vec<_>>(),
            "map_lag_hours_within_10_frames_of_an_event": lag(true),
            "map_lag_hours_elsewhere": lag(false),
            "map_resets_within_10_frames_of_an_event": self.map.iter().filter(|(index, ..)| near(*index)).map(|(_, _, resets, _)| resets).sum::<u32>(),
            "map_resets": self.map.iter().map(|(_, _, resets, _)| resets).sum::<u32>(),
            "map_frames_without_a_step_within_10_frames_of_an_event": self.map.iter().filter(|(index, _, _, steps)| near(*index) && *steps == 0).count(),
            "map_frames_without_a_step_elsewhere": self.map.iter().filter(|(index, _, _, steps)| !near(*index) && *steps == 0).count(),
            "hide": self.hide.map(|(which, from, frames)| json!({
                "what": if which == Hidden::Map { "the harness's map covered" } else { "the first extra map scrolled out" },
                "from": from, "frames": frames, "shown_at": self.shown.at,
                "frames_to_reader": self.shown.arrived.zip(self.shown.at).map(|(arrived, at)| arrived - at),
                "frames_to_picture": self.shown.pictured.zip(self.shown.at).map(|(pictured, at)| pictured - at),
                "lag_hours_after": distribution(self.shown.lag.clone()),
                "hidden_playhead_bytes": distribution(hidden.iter().map(|held| held.0 as f64).collect()),
                "hidden_ring_steps": distribution(hidden.iter().map(|held| held.1 as f64).collect()),
                "hidden_ring_bytes": distribution(hidden.iter().map(|held| held.2 as f64).collect()),
                "hidden_lanes": distribution(hidden.iter().map(|held| held.3 as f64).collect()),
            })),
            "held": self.held, "playhead_peak": peak, "stalls": counters,
            "scope": "extra_draw_ms: every extra map's frame (mirror apply, layers' apply and settle) on the window thread, after the map's own; no extra map uploads (it has no scene). opening_work: the frame thread's time starting, polling and installing readers made on workers, in frames where one was. step_most_ms: the stepper's wall time for the longest step the map took each frame. held: bytes every 60 frames; hidden_*: every frame a map was hidden (the playhead's report is refreshed at its own interval).",
        })
    }
}

/// An extra map's window for `head`: a stepped head waits for the picture of its hour, as the
/// harness's own map does.
fn window_for(side: &mut SideThread, head: Head) -> Option<crate::playhead_host::side::Window<'_>> {
    match head {
        Head::Stepped(hour) => side.stepped_window(hour),
        _ => side.window(),
    }
}
