//! The map's report says the oldest hour on screen: today's installed hour and the playhead's layers'.
use super::{Config, Options, Playhead, ShownHour, Viewer};
use crate::{
    layers::{Budget, Carried, Gpu, Inputs, Layer, Published, View, recorded::record},
    playhead_host::{MapSide, mirror::Mirrors, open, rows::tests::hourly_steps},
};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BRANCH: replay_data::BranchId = 1;

/// A layer whose worker the test holds: it publishes the hour of the last step it was handed only
/// while `open`, and until then shows the hour it published last.
struct Held {
    open: Arc<AtomicBool>,
    next: Option<Hour>,
    published: Option<Hour>,
}

impl Layer for Held {
    fn name(&self) -> &'static str {
        "held"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(TableId::Countries, true)?;
        inputs.index(TableId::Countries, 0)
    }
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.next = mirrors.get(TableId::Countries).and_then(|mirror| mirror.head());
        Ok(())
    }
    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.next = Some(step.last());
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        if self.open.load(Ordering::Acquire) {
            self.published = self.next;
        }
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn published(&self) -> Published {
        match self.published == self.next {
            true => Published::Current,
            false => self.published.map_or(Published::Nothing, Published::At),
        }
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A map drawing only [`Held`] over twelve recorded hours, its worker open.
fn side(name: &str) -> (MapSide, Arc<AtomicBool>) {
    let units = record(name, 12, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    let entry = open(units, hourly_steps()).unwrap();
    let gate = Arc::new(AtomicBool::new(true));
    let layer = Held { open: Arc::clone(&gate), next: None, published: None };
    (MapSide::new(entry, vec![Box::new(layer)], View::default()).unwrap(), gate)
}

/// Frames at `hour` until `done` holds; fails by name within 120 s.
fn frames_at(side: &mut MapSide, hour: Hour, what: &str, mut done: impl FnMut(&MapSide) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let frame = side.frame(hour).unwrap();
        if done(side) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} within 120 s: applied {:?}, drawn {:?}, composing {}", side.applied(), side.last_frame().drawn, side.composing());
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

/// Held at `hour` with its worker open, the map shows it.
fn shown_at(side: &mut MapSide, hour: Hour) {
    side.anchor(hour, false, 0.0, true);
    frames_at(side, hour, &format!("hour {hour} shown"), |side| !side.composing() && side.last_frame().drawn == Some(hour));
}

/// With the worker held, the step to `hour` is taken while the layer still shows the hour before.
fn applied_held(side: &mut MapSide, gate: &AtomicBool, hour: Hour, seek: bool) {
    gate.store(false, Ordering::Release);
    if seek {
        side.anchor(hour, false, 0.0, true);
    }
    frames_at(side, hour, &format!("hour {hour} applied"), |side| !side.composing() && side.applied() == Some(hour));
}

/// A viewer whose frame drew today's installed `hour`, with `side` drawing the playhead's layers.
fn viewer(hour: Hour, side: Option<MapSide>) -> Viewer {
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    let at = Playhead { branch: BRANCH, hour, fraction: 0.0 };
    viewer.rendered = Some(at);
    viewer.playhead = Some(at);
    viewer.rendered_sequence = 1;
    viewer.shown = Some(ShownHour {
        epoch: 0,
        branch: BRANCH,
        hour,
        error: None,
        political_warnings: Vec::new(),
        plan_warnings: Vec::new(),
        building_warnings: Vec::new(),
        model_warnings: Vec::new(),
        political_build: Duration::ZERO,
        limitations: Default::default(),
        snow: Default::default(),
        header: Default::default(),
        borders: None,
    });
    viewer.playhead_map = side;
    viewer
}

fn target(hour: Hour, playing: bool) -> dashboard::protocol::DashboardMapView {
    let bounds = dashboard::protocol::ViewBounds { x: 0, y: 0, width: 256, height: 144 };
    dashboard::protocol::DashboardMapView {
        options: Default::default(),
        id: "map-state".into(),
        source_id: "map-state".into(),
        path: "map-state".into(),
        revision: "map-state".into(),
        branch_id: BRANCH.to_string(),
        playing,
        hours_per_second: 1.0,
        hour,
        fraction: 0.,
        sequence: 1,
        seek_sequence: Some(1),
        bounds,
        clip: bounds,
        visible: true,
    }
}

/// After a seek, today's hour is installed but a drawn layer's worker still shows the hour before:
/// the report is loading at the layer's hour, and stops loading at the frame the layer shows the head.
#[test]
fn after_a_seek_the_report_loads_until_every_drawn_layer_shows_the_head() {
    let (mut side, gate) = side("map state seek");
    shown_at(&mut side, 2);
    applied_held(&mut side, &gate, 8, true);
    let mut viewer = viewer(8, Some(side));
    let state = viewer.map_state(&target(8, false));
    assert!(state.loading, "today's hour 8 is installed, but the held layer shows hour 2: {state:?}");
    assert_eq!(state.hour, Some(2), "the oldest hour on screen is the layer's");
    viewer.playhead_map.as_mut().unwrap().frame(8).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "still held, still loading");
    gate.store(true, Ordering::Release);
    viewer.playhead_map.as_mut().unwrap().frame(8).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "the layers show hour 8, the overlay nothing");
    viewer.overlay_laid = Some((8, 0));
    (viewer.counters_given, viewer.counters_drawn) = (Some((8, 0)), Some((8, 0)));
    let state = viewer.map_state(&target(8, false));
    assert!(!state.loading, "the frame every part shows hour 8 stops the loading: {state:?}");
    assert_eq!(state.hour, Some(8));
}

/// Playing, today's frame is at the clock's hour while a drawn layer's worker is four hours behind:
/// the report is behind by the layers' lag, named by the layer furthest behind.
#[test]
fn playing_the_report_is_behind_by_the_layers_lag_when_it_is_the_larger() {
    let (mut side, gate) = side("map state behind");
    shown_at(&mut side, 6);
    applied_held(&mut side, &gate, 10, false);
    let viewer = viewer(10, Some(side));
    let state = viewer.map_state(&target(10, true));
    assert_eq!(state.behind, Some(4), "today's frame is at hour 10, the held layer at 6: {state:?}");
    assert_eq!(state.behind_reason.as_deref(), Some("the playhead's layers: held"));
    assert_eq!(state.hour, Some(6));
}

/// With the playhead off, the report is today's alone: the same viewer, its layers taken away,
/// reports today's installed hour as ready. A guard of today's behaviour, which it keeps.
#[test]
fn with_the_playhead_off_the_report_is_todays() {
    let (mut side, gate) = side("map state off");
    shown_at(&mut side, 2);
    applied_held(&mut side, &gate, 8, true);
    let mut viewer = viewer(8, Some(side));
    assert!(viewer.map_state(&target(8, false)).loading, "the layers hold the report");
    viewer.playhead_map = None;
    let state = viewer.map_state(&target(8, false));
    assert!(!state.loading, "{state:?}");
    assert_eq!((state.hour, state.fraction, state.behind, state.behind_reason.as_deref()), (Some(8), Some(0.0), None, None));
    let playing = viewer.map_state(&target(8, true));
    assert_eq!((playing.loading, playing.hour, playing.behind, playing.behind_reason.as_deref()), (false, Some(8), Some(0), None));
}

/// A window that is not shown (minimised, covered, or hidden by the desktop host on a dashboard
/// page) runs only eframe's logic pass: its map draws no frame. It lets its playhead go there, so
/// nothing waits on a map that is not drawing: a seek made meanwhile (from the dashboard) is
/// answered by the rows at the head at the hour sought. Before, the map stayed counted while it
/// drew nothing, and the rows took nothing of the new seek until the window was shown again.
#[test]
fn a_window_not_shown_lets_its_map_go_and_the_rows_answer_a_seek_meanwhile() {
    use crate::playhead_host::rows::{Cadence, Recording, Rows};
    let (mut side, _) = side("window not shown");
    shown_at(&mut side, 2);
    let entry = Arc::clone(side.entry());
    let rows = Rows::of(&entry, Recording { branch: BRANCH }).unwrap();
    let reader = rows.reader(&[TableId::Countries], Cadence::PerSecond(4));
    rows.settled();
    let mut viewer = viewer(2, Some(side));
    let context = eframe::egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut input = eframe::egui::RawInput::default();
    input.viewports.entry(eframe::egui::ViewportId::ROOT).or_default().minimized = Some(true);
    let _ = context.run_logic(&input, |context| eframe::App::logic(&mut viewer, context, &mut frame));
    entry.anchor(6, false, 0.0, true);
    let rows_at = reader.wait_through(6, Duration::from_secs(60)).expect("the rows answer the seek at the hour sought");
    assert_eq!(rows_at.hour, 6);
}
