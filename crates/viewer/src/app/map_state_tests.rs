use super::{Config, Options, Playhead, Viewer};
use viewer_layer_core::{layers::{Carried, Settling, Inputs, Layer, Published, View}, test_support::recorded::record};
use crate::playhead_host::{Entry, MapSide, mirror::Mirrors, open, views::tests::hourly_steps};
use viewer_playback::side::SideThread;
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
    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        if self.open.load(Ordering::Acquire) {
            self.published = self.next;
        }
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

struct Owing {
    owes: bool,
    installs: u32,
}

impl Owing {
    fn install(&mut self) -> io::Result<()> {
        (self.owes, self.installs) = (false, self.installs + 1);
        Ok(())
    }
}

impl Layer for Owing {
    fn name(&self) -> &'static str {
        "owing"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(TableId::Countries, true)?;
        inputs.index(TableId::Countries, 0)
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.owes = false;
        Ok(())
    }
    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {
        self.owes = true;
    }
    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        Ok(())
    }
    fn uploaded(&self) -> bool {
        !self.owes
    }
    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn carried(&self) -> Carried {
        Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

fn side(name: &str) -> (MapSide, Arc<AtomicBool>) {
    let units = record(name, 12, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    joins(&open(units, hourly_steps()).unwrap())
}

fn joins(entry: &Arc<Entry>) -> (MapSide, Arc<AtomicBool>) {
    let gate = Arc::new(AtomicBool::new(true));
    let layer = Held { open: Arc::clone(&gate), next: None, published: None };
    (MapSide::with_products(Arc::clone(entry), vec![Box::new(layer)], View::default()).unwrap(), gate)
}

fn frames_at(side: &mut MapSide, hour: Hour, what: &str, mut done: impl FnMut(&MapSide) -> bool) {
    viewer_layer_core::test_support::recorded::frames_at(side, hour, what, |side, _| done(side));
}

fn shown_at(side: &mut MapSide, hour: Hour) {
    side.anchor(hour, false, 0.0, true);
    frames_at(side, hour, &format!("hour {hour} shown"), |side| !side.composing() && side.last_frame().drawn == Some(hour));
}

fn applied_held(side: &mut MapSide, gate: &AtomicBool, hour: Hour, seek: bool) {
    gate.store(false, Ordering::Release);
    if seek {
        side.anchor(hour, false, 0.0, true);
    }
    frames_at(side, hour, &format!("hour {hour} applied"), |side| !side.composing() && side.applied() == Some(hour));
}

fn viewer(hour: Hour, side: MapSide) -> Viewer {
    let mut side = SideThread::new(side).unwrap();
    side.driven(|_| ());
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    let at = Playhead { branch: BRANCH, hour, fraction: 0.0 };
    viewer.rendered = Some(at);
    viewer.playhead = Some(at);
    viewer.rendered_sequence = 1;
    viewer.playhead_map = Some(side);
    viewer
}

fn target(hour: Hour, playing: bool) -> dashboard_core::protocol::DashboardMapView {
    let bounds = dashboard_core::protocol::ViewBounds { x: 0, y: 0, width: 256, height: 144 };
    dashboard_core::protocol::DashboardMapView {
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
        composed_at: 0.0,
        clock_id: "page-1".into(),
        bounds,
        clip: bounds,
        visible: true,
    }
}

#[test]
fn after_a_seek_the_report_loads_until_every_drawn_layer_shows_the_head() {
    let (mut side, gate) = side("map state seek");
    shown_at(&mut side, 2);
    applied_held(&mut side, &gate, 8, true);
    let mut viewer = viewer(8, side);
    let state = viewer.map_state(&target(8, false));
    assert_eq!(state.clock_id, "page-1");
    assert!(state.loading, "the head is hour 8, but the held layer shows hour 2: {state:?}");
    assert_eq!(state.hour, Some(2), "the hour on screen is the layer's");
    viewer.playhead_map.as_mut().unwrap().driven(|side| side.frame(8)).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "still held, still loading");
    gate.store(true, Ordering::Release);
    viewer.playhead_map.as_mut().unwrap().driven(|side| side.frame(8)).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "the layers show hour 8, the overlay nothing");
    viewer.overlay_hour = Some(8);
    let state = viewer.map_state(&target(8, false));
    assert_eq!(state.clock_id, "page-1");
    assert!(!state.loading, "the frame every part shows hour 8 stops the loading: {state:?}");
    assert_eq!(state.hour, Some(8));
    viewer.error = Some("synthetic map error".into());
    let failed = viewer.map_state(&target(8, false));
    assert_eq!(failed.clock_id, "page-1");
    assert_eq!(failed.error.as_deref(), Some("synthetic map error"));
}

#[test]
fn playing_the_report_is_behind_by_the_layers_lag() {
    let (mut side, gate) = side("map state behind");
    shown_at(&mut side, 6);
    applied_held(&mut side, &gate, 10, false);
    let viewer = viewer(10, side);
    let state = viewer.map_state(&target(10, true));
    assert_eq!(state.behind, Some(4), "the clock is at hour 10, the held layer at 6: {state:?}");
    assert_eq!(state.behind_reason.as_deref(), Some("the playhead's layers: held"));
    assert_eq!(state.hour, Some(6));
}

#[test]
fn a_window_not_shown_lets_its_map_go_and_the_rows_answer_a_seek_meanwhile() {
    use crate::playhead_host::views::{Need, Pace, apart::Apart};
    let (mut side, _) = side("window not shown");
    shown_at(&mut side, 2);
    let entry = Arc::clone(side.entry());
    let mut apart = Apart::new(&entry, BRANCH).unwrap();
    let reader = apart.store().handle().reader_at("not the map", &[Need::every(TableId::Countries)], Pace::Step);
    let mut viewer = viewer(2, side);
    let context = eframe::egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut input = eframe::egui::RawInput::default();
    input.viewports.entry(eframe::egui::ViewportId::ROOT).or_default().minimized = Some(true);
    let _ = context.run_logic(&input, |context| eframe::App::logic(&mut viewer, context, &mut frame));
    entry.anchor(6, false, 0.0, true);
    assert!(apart.drive(6), "the rows answer the seek at the hour sought");
    let rows_at = reader.frame().expect("the rows answer the seek at the hour sought");
    assert_eq!(rows_at.hour, 6);
}

#[test]
fn a_seek_no_reader_took_is_owed_until_one_does_and_play_runs_from_it() {
    for (stood, sought) in [(8, 2), (2, 8)] {
        let (side, _) = side(&format!("seek then play from {stood}"));
        let entry = Arc::clone(side.entry());
        let now = Instant::now();
        let mut viewer = viewer(stood, side);
        viewer.last_step = now;
        let head = |hour| Playhead { branch: BRANCH, hour, fraction: 0.0 };
        let at = |ms| now + Duration::from_millis(ms);
        viewer.anchor_playback(head(stood), false, 1.0, true, None, now, 11);
        assert_eq!(entry.playhead().head(now), stood, "the map stands where it played to");
        viewer.close_playhead();
        viewer.anchor_playback(head(sought), false, 1.0, true, None, at(10), 11);
        viewer.anchor_playback(head(sought), true, 1.0, false, None, at(1_010), 11);
        let (back, _) = joins(&entry);
        viewer.playhead_map = Some(SideThread::new(back).unwrap());
        viewer.anchor_playback(head(sought), true, 1.0, false, None, at(1_020), 11);
        assert_eq!(entry.playhead().head(Instant::now()), sought, "the map seeks to {sought}, never plays on from {stood}");
        assert!(!entry.paused(), "and plays from the hour sought");
        viewer.anchor_playback(head(sought), false, 1.0, true, None, at(1_030), 11);
        viewer.close_playhead();
        viewer.anchor_playback(head(stood), false, 1.0, true, None, at(1_040), 11);
        let (back, _) = joins(&entry);
        viewer.side_opened(SideThread::new(back).unwrap());
        assert_eq!(entry.playhead().head(Instant::now()), stood, "the reader that opens takes the seek owed");
        assert!(entry.paused(), "and stands there");
        for (hour, playing, speed, seek) in [(stood, false, 24.0, false), (stood, true, 24.0, false), (stood, false, 1.0, false), (sought, false, 1.0, true), (sought + 1, false, 1.0, false)] {
            viewer.anchor_playback(head(hour), playing, speed, seek, None, at(1_050), 11);
            assert_eq!(entry.playhead().head(Instant::now()), hour, "{hour} at {speed} playing {playing}");
        }
    }
}

#[test]
fn a_picture_beyond_the_clock_is_not_reported_as_the_target_shown() {
    let (mut side, _) = side("map state beyond");
    shown_at(&mut side, 10);
    let mut viewer = viewer(4, side);
    viewer.overlay_hour = Some(10);
    let state = viewer.map_state(&target(4, true));
    assert_eq!(state.hour, Some(10), "the hour on screen is said as it is");
    assert!(state.loading, "the clock is at hour 4 and the picture at hour 10: {state:?}");
    assert_eq!(state.behind, None, "a picture the clock has not reached trails it by no hours");
}

#[test]
fn a_layers_install_runs_on_the_frame_that_takes_its_publication_whatever_the_frames_clock_says() {
    let units = record("install-on-the-publication", 12, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    let entry = open(units, hourly_steps()).unwrap();
    let mut side = MapSide::with_products(Arc::clone(&entry), vec![Box::new(Owing { owes: false, installs: 0 })], View::default()).unwrap();
    shown_at(&mut side, 2);
    viewer_runtime::display_refresh::pace_at(1e12);
    let mut installed = 0;
    for hour in 3..=8 {
        frames_at(&mut side, hour, &format!("hour {hour} applied"), |side| !side.composing() && side.applied() == Some(hour));
        let _budget = viewer_runtime::frame_budget::begin(true);
        side.upload_each(|layer| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Owing>()) {
            Some(owing) => {
                installed += 1;
                owing.install()
            }
            None => Ok(()),
        })
        .unwrap();
        drop(_budget);
        assert_eq!(side.last_frame().drawn, Some(hour), "the hour the picture shows is the hour the publication applied");
        assert!(!side.last_frame().layers[0].upload_carried, "nothing of the publication is left for a later frame");
    }
    viewer_runtime::display_refresh::pace_at(120.0);
    assert_eq!(installed, 6, "every publication's install ran in the window that took it");
}
