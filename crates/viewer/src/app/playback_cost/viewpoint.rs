use super::super::{Playhead, Viewer};
use super::toggle;
use super::{render_state, wait_until};
use crate::Options;
use crate::playback_cost::distribution;
use eframe::egui;
use replay_data::rows::tables;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_foundation::perspective::Country;

pub(crate) struct Switches {
    pub countries: Vec<String>,
    pub rounds: u32,
    pub play_switches: u32,
    pub play_speed: f64,
    pub hour_steps: u32,
    pub refresh_hz: f64,
    pub settle_frames: u32,
    pub timeout: Duration,
}

#[derive(Clone)]
pub(in crate::app) struct Target {
    pub(crate) slot: u32,
    pub(crate) tag: String,
}

pub(in crate::app) struct Seen {
    pub(crate) parts: Vec<(String, bool, bool)>,
    pub(crate) whole: bool,
    pub(crate) ready: bool,
}

impl Seen {
    pub(crate) fn settled(&self) -> bool {
        self.ready && self.parts.iter().all(|(_, good, _)| *good)
    }
}

pub(in crate::app) fn seen(viewer: &Viewer, target: Option<&Target>, head: Option<Playhead>, playing: bool, layers: &mut Vec<crate::playhead_host::CaughtUp>) -> Seen {
    let whole = viewer.drew_frame && head.is_some_and(|head| viewer.presentation_ready(head, playing));
    let ready = if playing { viewer.drew_frame } else { whole };
    let mut parts = Vec::new();
    let Some(side) = viewer.playhead_map.as_ref() else {
        return Seen { parts: vec![("map".into(), false, false)], whole, ready };
    };
    if let Some(target) = target {
        let view = matches!(&side.view().perspective.country, Country::Slot { slot, .. } if *slot == target.slot);
        parts.push(("view".into(), view, false));
        let vision = viewer.shown_vision().is_ok_and(|vision| vision.ours() == target.slot);
        parts.push(("vision shown".into(), vision, vision));
        let overlay = viewer.overlay_tables.as_ref().is_some_and(|tables| tables.ours() == target.slot);
        parts.push(("overlay".into(), overlay, overlay));
    }
    if let Some(read) = side.try_with_side(crate::playhead_host::MapSide::caught_up) {
        *layers = read;
    }
    for layer in layers.iter() {
        let viewing = match (target, layer.viewer) {
            (Some(target), Some(held)) => Some(held == target.slot),
            _ => None,
        };
        let current = if playing { !layer.withheld } else { layer.caught };
        parts.push((layer.name.to_owned(), current && viewing != Some(false), viewing == Some(true)));
    }
    Seen { parts, whole, ready }
}

fn waiting(viewer: &Viewer, seen: &Seen) -> String {
    let behind: Vec<&str> = seen.parts.iter().filter(|(_, good, _)| !good).map(|(name, ..)| name.as_str()).collect();
    let layers = viewer.playhead_map.as_ref().map(|side| side.with_side(crate::playhead_host::MapSide::shown_states));
    format!("whole {}, behind {behind:?}, layers {layers:?}, error {:?}", seen.ready, viewer.reported_error())
}

struct Run<'a> {
    viewer: &'a mut Viewer,
    ctx: egui::Context,
    state: eframe::egui_wgpu::RenderState,
    service: Arc<dashboard::Service>,
    started: Instant,
    switches: &'a Switches,
    layers: Vec<crate::playhead_host::CaughtUp>,
}

impl Run<'_> {
    fn frame(&mut self, playing: bool) -> io::Result<f64> {
        let now = Instant::now();
        if playing
            && let (Some(side), Some(head)) = (self.viewer.playhead_map.as_ref(), self.viewer.playhead)
        {
            self.viewer.playhead = Some(Playhead { hour: side.head(now), fraction: 0.0, ..head });
        }
        let seconds = now.duration_since(self.started).as_secs_f64();
        let pixels = self.viewer.options.size;
        toggle::frame(self.viewer, &self.ctx, &self.state, &self.service, pixels, seconds)?;
        Ok(now.elapsed().as_secs_f64() * 1e3)
    }

    fn until_settled(&mut self, name: &str, target: Option<&Target>, head: Option<Playhead>, playing: bool, requested: Instant) -> io::Result<Value> {
        let (mut first_changed, mut settled) = (None, None);
        let mut since: BTreeMap<String, f64> = BTreeMap::new();
        let mut caught = Value::Null;
        let (mut work, mut frames, mut unsettled_after, mut after) = (Vec::new(), Vec::new(), 0u32, 0u32);
        let mut behind_after: BTreeMap<String, u32> = BTreeMap::new();
        for index in 0u64.. {
            wait_until(crate::playback_cost::frame_due(requested, index as f64 / self.switches.refresh_hz)?);
            let at_ms = requested.elapsed().as_secs_f64() * 1e3;
            let work_ms = self.frame(playing)?;
            let seen = seen(self.viewer, target, head.or(self.viewer.playhead), playing, &mut self.layers);
            if settled.is_some() {
                unsettled_after += u32::from(!seen.settled());
                for part in seen.parts.iter().filter(|(_, good, _)| !good).map(|(part, ..)| part.clone()).chain((!seen.ready).then(|| "whole".to_owned())) {
                    *behind_after.entry(part).or_default() += 1;
                }
                after += 1;
                if after >= self.switches.settle_frames {
                    break;
                }
                continue;
            }
            work.push(work_ms);
            if first_changed.is_none() && seen.parts.iter().any(|(_, _, new)| *new) {
                first_changed = Some(at_ms);
            }
            for (part, good, _) in &seen.parts {
                if *good {
                    since.entry(part.clone()).or_insert(at_ms);
                } else {
                    since.remove(part);
                }
            }
            let behind: Vec<&str> = seen.parts.iter().filter(|(_, good, _)| !good).map(|(part, ..)| part.as_str()).collect();
            frames.push(json!({"frame": index, "at_ms": at_ms, "work_ms": work_ms, "whole": seen.whole, "behind": behind}));
            if seen.settled() {
                settled = Some(at_ms);
                caught = json!(since);
            } else if requested.elapsed() > self.switches.timeout {
                return Err(io::Error::other(format!("{name}: not settled in {} s: {}", self.switches.timeout.as_secs(), waiting(self.viewer, &seen))));
            }
        }
        let latest = since.values().copied().fold(None, |held: Option<f64>, ms| Some(held.map_or(ms, |held| held.max(ms))));
        let last = latest.map(|ms| json!({"parts": since.iter().filter(|(_, at)| **at == ms).map(|(part, _)| part).collect::<Vec<_>>(), "ms": ms}));
        Ok(json!({
            "name": name,
            "playing": playing,
            "hour": self.viewer.playhead.map(|head| head.hour),
            "ms": {"first_changed": first_changed, "settled": settled},
            "frames_to_settle": frames.len(),
            "settled_last": last,
            "parts_caught_ms": caught,
            "work_ms": distribution(work),
            "unsettled_frames_after": unsettled_after,
            "behind_after": behind_after,
            "frames": frames,
        }))
    }
}

fn slots(frame: &replay_frame::frame::DecodedFrame, tags: &[String]) -> io::Result<Vec<Target>> {
    let countries = frame.complete::<tables::Countries>().map_err(|why| io::Error::other(format!("Countries: {why}")))?;
    tags.iter()
        .map(|tag| {
            countries
                .iter()
                .rev()
                .find(|row| row.tag == tag.as_bytes())
                .map(|row| Target { slot: row.canonical_slot, tag: tag.clone() })
                .ok_or_else(|| io::Error::other(format!("Countries holds no country tagged {tag} at the hour")))
        })
        .collect()
}

fn summary(records: &[Value], playing: bool) -> Value {
    let of = |key: &str| records.iter().filter(|record| record["playing"] == playing).filter_map(|record| record["ms"][key].as_f64()).collect::<Vec<_>>();
    let mut last: BTreeMap<String, u32> = BTreeMap::new();
    for record in records.iter().filter(|record| record["playing"] == playing) {
        for part in record["settled_last"]["parts"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            *last.entry(part.to_owned()).or_default() += 1;
        }
    }
    json!({"switches": of("settled").len(), "first_changed_ms": distribution(of("first_changed")), "settled_ms": distribution(of("settled")), "settled_last": last})
}

pub(crate) fn measure(config: dashboard_core::config::Config, mut options: Options, path: PathBuf, switches: &Switches) -> io::Result<Value> {
    viewer_runtime::worker_priority::frame_priority();
    shader_pipeline::gpu::stats::mark_window_thread();
    let state = render_state()?;
    let info = state.adapter.get_info();
    options.window_dump = false;
    options.headless_dump = false;
    let service = dashboard::Service::new(Arc::new(|_| true))?;
    service.configure_content(&config, options.allow_unverified)?;
    let revision = dashboard_core::protocol::replay_revision(&path)?;
    let source = service.open_map(&path, Some(&revision)).map_err(io::Error::other)?;
    let mut viewer = toggle::viewer(&config, &options, &state, &service)?;
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    let opened = Instant::now();
    viewer.open(path, &ctx, Some(source.key().revision.clone()), None);
    let mut run = Run { viewer: &mut viewer, ctx, state, service, started: opened, switches, layers: Vec::new() };
    let open = run.until_settled("open", None, None, false, opened)?;
    println!("open: settled at {:?} ms", open["ms"]["settled"]);
    let head = run.viewer.playhead.expect("an opened map has a playhead");
    let frame = run.viewer.overlay_frame.clone().ok_or_else(|| io::Error::other("the opened map holds no rows at the head"))?;
    let targets = slots(&frame, &switches.countries)?;
    let mut ours = run.viewer.shown_vision().map_err(io::Error::other)?.ours();
    let mut records = Vec::new();
    let mut next = targets.iter().cycle();
    let mut switch = |run: &mut Run<'_>, playing: bool, records: &mut Vec<Value>| -> io::Result<()> {
        let target = next.by_ref().find(|target| target.slot != ours).expect("two countries differ").clone();
        let name = format!("{}-{}{}", records.len() + 1, target.tag, if playing { "-playing" } else { "" });
        let requested = Instant::now();
        run.viewer.perspective.country = Country::Slot { slot: target.slot, tag: target.tag.clone() };
        let held = (!playing).then(|| Playhead { fraction: 0.0, ..run.viewer.playhead.expect("the map has a playhead") });
        let mut record = run.until_settled(&name, Some(&target), held, playing, requested)?;
        record["from_slot"] = json!(ours);
        record["to"] = json!({"tag": target.tag, "slot": target.slot});
        println!("{name}: first changed {:?} ms, settled {:?} ms, last {}", record["ms"]["first_changed"], record["ms"]["settled"], record["settled_last"]);
        records.push(record);
        ours = target.slot;
        Ok(())
    };
    for _ in 0..switches.rounds * targets.len() as u32 {
        switch(&mut run, false, &mut records)?;
    }
    let last = run.viewer.document.as_ref().and_then(|document| document.playback.range(head.branch)).map_or(head.hour, |(_, last)| last);
    let mut steps = Vec::new();
    for _ in 0..switches.hour_steps {
        let at = run.viewer.playhead.expect("the map has a playhead");
        if at.hour >= last {
            break;
        }
        let target = Playhead { hour: at.hour + 1, fraction: 0.0, ..at };
        let (requested, speed) = (Instant::now(), run.viewer.speed);
        run.viewer.anchor_playback(target, false, speed, false, None, requested, last);
        let record = run.until_settled(&format!("hour-{}", target.hour), None, Some(target), false, requested)?;
        println!("hour step to {}: settled {:?} ms", target.hour, record["ms"]["settled"]);
        steps.push(record);
    }
    if switches.play_switches > 0 {
        let at = run.viewer.playhead.expect("the map has a playhead");
        run.viewer.anchor_playback(at, true, switches.play_speed, false, None, Instant::now(), last);
        let playing_from = Instant::now();
        while playing_from.elapsed() < Duration::from_secs(1) {
            wait_until(crate::playback_cost::frame_due(Instant::now(), 1.0 / switches.refresh_hz)?);
            run.frame(true)?;
        }
        for _ in 0..switches.play_switches {
            switch(&mut run, true, &mut records)?;
        }
    }
    let step_ms: Vec<f64> = steps.iter().filter_map(|step| step["ms"]["settled"].as_f64()).collect();
    drop(run);
    drop(viewer);
    drop(source);
    Ok(json!({
        "schema": 1,
        "mode": "viewpoint-switch",
        "countries": targets.iter().map(|target| json!({"tag": target.tag, "slot": target.slot})).collect::<Vec<_>>(),
        "hour": head.hour,
        "refresh_hz": switches.refresh_hz,
        "play_speed": switches.play_speed,
        "adapter": {"name": info.name, "backend": format!("{:?}", info.backend), "driver": info.driver},
        "viewport": options.size,
        "camera": options.camera.map(|camera| format!("{camera:?}")),
        "summary": {"held": summary(&records, false), "playing": summary(&records, true), "hour_step_settled_ms": distribution(step_ms)},
        "open": open,
        "switches": records,
        "hour_steps": steps,
        "scope": "Each switch runs from the view's perspective set to the new country's slot (what a Ctrl+click or the host's MapOptions does in the viewer) to ms.first_changed, the first frame any part shows the new country (the vision shown, the overlay's tables or a layer's held viewing country), and to ms.settled, the first frame wholly of it: the side drawing the new perspective, the vision shown, the overlay's tables and every layer holding a viewing country on the new one, and every drawn layer caught up (held) or publishing a product (playing); held, the picture is also whole at the hour (presentation_ready), while playing that is recorded per frame (whole) but not asked, since the playing picture stands behind the moving head for reasons of its own. Layers are read without waiting for the side, so a layer-held country may lead the presented frame by a publication. parts_caught_ms is each part's start of the run of correct frames up to the settled one; settled_last names the latest; behind_after counts the parts not correct in the settle frames after it. hour_steps are single held hour steps timed the same way, the cost a switch is compared with. The host's own path to the viewer (the dashboard and the desktop shell) is not in it.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_reports_scene_build_error() {
        let mut viewer = Viewer::create(
            crate::Config::default(),
            Options::default(),
            None,
            Arc::new(std::sync::Mutex::new(None)),
            None,
            None,
        );
        let seen = Seen {
            parts: vec![("map".into(), false, false)],
            whole: false,
            ready: false,
        };
        assert!(waiting(&viewer, &seen).ends_with("error None"));

        viewer.scene_error = Some("scene build failed".into());
        assert!(waiting(&viewer, &seen).ends_with("error Some(\"scene build failed\")"));

        viewer.error = Some("seek failed".into());
        assert!(waiting(&viewer, &seen).ends_with("error Some(\"seek failed\")"));

        viewer.error = None;
        viewer.scene_error = None;
        viewer.map_error = Some("map failed".into());
        assert!(waiting(&viewer, &seen).ends_with("error Some(\"map failed\")"));
    }
}
