//! The map's report says the hour on screen: the one the playhead's layers draw.
use super::{Config, Options, Playhead, Viewer};
use crate::{
    layers::{Carried, Settling, Inputs, Layer, Published, View, recorded::record},
    playhead_host::{Entry, MapSide, mirror::Mirrors, open, views::tests::hourly_steps, side::SideThread},
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

/// A layer that owes the GPU an install after every step it applies, and takes it when its
/// upload runs. Nothing but the install stands between the hour it applied and the hour it shows.
struct Owing {
    owes: bool,
    installs: u32,
}

impl Owing {
    /// What its upload does, without a device: the test's stand-in for [`Layer::upload`].
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

/// A map drawing only [`Held`] over twelve recorded hours, its worker open.
fn side(name: &str) -> (MapSide, Arc<AtomicBool>) {
    let units = record(name, 12, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    joins(&open(units, hourly_steps()).unwrap())
}

/// Another [`Held`] map of `entry`, as a reopened widget's reader is: the playhead outlives the
/// reader that closed.
fn joins(entry: &Arc<Entry>) -> (MapSide, Arc<AtomicBool>) {
    let gate = Arc::new(AtomicBool::new(true));
    let layer = Held { open: Arc::clone(&gate), next: None, published: None };
    (MapSide::new(Arc::clone(entry), vec![Box::new(layer)], View::default()).unwrap(), gate)
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

/// A viewer whose frame drew `hour`, with `side` drawing the playhead's layers.
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
        composed_at: 0.0,
        bounds,
        clip: bounds,
        visible: true,
    }
}

/// After a seek, a drawn layer's worker still shows the hour before: the report is loading at the
/// layer's hour, and stops loading at the frame the layer shows the head.
#[test]
fn after_a_seek_the_report_loads_until_every_drawn_layer_shows_the_head() {
    let (mut side, gate) = side("map state seek");
    shown_at(&mut side, 2);
    applied_held(&mut side, &gate, 8, true);
    let mut viewer = viewer(8, side);
    let state = viewer.map_state(&target(8, false));
    assert!(state.loading, "the head is hour 8, but the held layer shows hour 2: {state:?}");
    assert_eq!(state.hour, Some(2), "the hour on screen is the layer's");
    viewer.playhead_map.as_mut().unwrap().driven(|side| side.frame(8)).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "still held, still loading");
    gate.store(true, Ordering::Release);
    viewer.playhead_map.as_mut().unwrap().driven(|side| side.frame(8)).unwrap();
    assert!(viewer.map_state(&target(8, false)).loading, "the layers show hour 8, the overlay nothing");
    viewer.overlay_hour = Some(8);
    let state = viewer.map_state(&target(8, false));
    assert!(!state.loading, "the frame every part shows hour 8 stops the loading: {state:?}");
    assert_eq!(state.hour, Some(8));
}

/// Playing, the clock is at hour 10 while a drawn layer's worker is four hours behind: the report
/// is behind by the layers' lag, named by the layer furthest behind.
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

/// A window that is not shown (minimised, covered, or hidden by the desktop host on a dashboard
/// page) runs only eframe's logic pass: its map draws no frame. It lets its playhead go there, so
/// nothing waits on a map that is not drawing: a seek made meanwhile (from the dashboard) is
/// answered by the rows at the head at the hour sought. Before, the map stayed counted while it
/// drew nothing, and the rows took nothing of the new seek until the window was shown again.
#[test]
fn a_window_not_shown_lets_its_map_go_and_the_rows_answer_a_seek_meanwhile() {
    use crate::playhead_host::views::{Need, Pace, apart::Apart};
    let (mut side, _) = side("window not shown");
    shown_at(&mut side, 2);
    let entry = Arc::clone(side.entry());
    // A reader of this playhead that is not the map: the dashboard's panels have one of these.
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
    let rows_at = reader.wait(|frame| frame.hour == 6, Duration::from_secs(60)).expect("the rows answer the seek at the hour sought");
    assert_eq!(rows_at.hour, 6);
}

/// A seek the page asks for while the map's reader is closed or still being made reaches no
/// reader, and the playhead outlives the reader: its clock stands where it stood. The page raises
/// its seek counter once, so no later anchor is a seek of its own. The seek is owed until a reader
/// takes it, and play that follows runs from the hour sought, backwards and forwards.
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
        // The widget is scrolled out of the page: its reader closes, its playhead lives on.
        viewer.close_playhead();
        // The page seeks while paused, and plays a second later: no reader took either anchor.
        viewer.anchor_playback(head(sought), false, 1.0, true, None, at(10), 11);
        viewer.anchor_playback(head(sought), true, 1.0, false, None, at(1_010), 11);
        // The reader comes back and the page sends its next anchor, the same as its last.
        let (back, _) = joins(&entry);
        viewer.playhead_map = Some(SideThread::new(back).unwrap());
        viewer.anchor_playback(head(sought), true, 1.0, false, None, at(1_020), 11);
        assert_eq!(entry.playhead().head(Instant::now()), sought, "the map seeks to {sought}, never plays on from {stood}");
        assert!(!entry.paused(), "and plays from the hour sought");
        // Paused, the page sends no further anchor: the reader that opens takes the seek itself.
        viewer.anchor_playback(head(sought), false, 1.0, true, None, at(1_030), 11);
        viewer.close_playhead();
        viewer.anchor_playback(head(stood), false, 1.0, true, None, at(1_040), 11);
        let (back, _) = joins(&entry);
        viewer.side_opened(SideThread::new(back).unwrap());
        assert_eq!(entry.playhead().head(Instant::now()), stood, "the reader that opens takes the seek owed");
        assert!(entry.paused(), "and stands there");
        // A speed change, a resume, a pause, a second seek while the seek's reset is still to be
        // taken, and a step forward, which keeps the page's seek counter, each leave the hour exact.
        // Before, a paused step left the playhead at the hour before while the page said the next.
        for (hour, playing, speed, seek) in [(stood, false, 24.0, false), (stood, true, 24.0, false), (stood, false, 1.0, false), (sought, false, 1.0, true), (sought + 1, false, 1.0, false)] {
            viewer.anchor_playback(head(hour), playing, speed, seek, None, at(1_050), 11);
            assert_eq!(entry.playhead().head(Instant::now()), hour, "{hour} at {speed} playing {playing}");
        }
    }
}

/// Playing, a picture of an hour the page's clock has not reached answers nothing: the map says
/// the hour it shows and that it is still loading, and no lag at all, never that the target asked
/// for is shown by a frame exactly up to date.
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
    // A frame whose interval is spent before its first ask: the budget's install ration, while it
    // stood, left every layer's install for a later frame and held the hour the picture showed
    // with it. The publication's products reach the GPU in the window that takes them.
    let units = record("install-on-the-publication", 12, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    let entry = open(units, hourly_steps()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Owing { owes: false, installs: 0 })], View::default()).unwrap();
    shown_at(&mut side, 2);
    viewer_foundation::map::display_refresh::pace_at(1e12);
    let mut installed = 0;
    for hour in 3..=8 {
        frames_at(&mut side, hour, &format!("hour {hour} applied"), |side| !side.composing() && side.applied() == Some(hour));
        let _budget = viewer_foundation::map::frame_budget::begin(true);
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
    viewer_foundation::map::display_refresh::pace_at(120.0);
    assert_eq!(installed, 6, "every publication's install ran in the window that took it");
}
