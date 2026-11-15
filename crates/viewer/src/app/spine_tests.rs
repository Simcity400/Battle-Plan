//! With the playhead's map open, the window takes where it is, whether it is ready, the mode on
//! screen and its histories' continuity from the host, and today's pipeline is never started.
use super::{Config, Options, Playhead, ShownHour, Viewer};
use crate::{
    layers::{Budget, Carried, Gpu, Inputs, Layer, Published, View, recorded::record},
    playhead_host::{MapSide, mirror::Mirrors, open, rows::tests::hourly_steps},
};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::branches::Gap;
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_foundation::map::{
    frame::DayNight,
    presentation::{Continuity, UnitRestart},
};
use viewer_world::map::map_mode::{MapMode, Stock};

const BRANCH: BranchId = 1;
const LAST: Hour = 11;

/// A layer whose worker the test holds: it publishes the hour of the last step it was handed, and
/// settles a change of view, only while `open`.
struct Gated {
    open: Arc<AtomicBool>,
    next: Option<Hour>,
    published: Option<Hour>,
    viewed: bool,
}

impl Layer for Gated {
    fn name(&self) -> &'static str {
        "gated"
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
    fn viewed(&mut self, _view: &View, _mirrors: &Mirrors) {
        self.viewed = true;
    }
    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        if self.open.load(Ordering::Acquire) {
            self.published = self.next;
            self.viewed = false;
        }
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried { entities: usize::from(self.viewed), frames: 0 }
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

/// A map of branch [`BRANCH`] drawing only [`Gated`] over twelve recorded hours, its worker open.
fn side(name: &str) -> (MapSide, Arc<AtomicBool>) {
    let units = record(name, LAST as u32 + 1, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    let entry = open(units, hourly_steps()).unwrap();
    let gate = Arc::new(AtomicBool::new(true));
    let layer = Gated { open: Arc::clone(&gate), next: None, published: None, viewed: false };
    let mut side = MapSide::new(entry, vec![Box::new(layer)], View::default()).unwrap();
    side.set_branch(BRANCH);
    (side, gate)
}

fn map(viewer: &mut Viewer) -> &mut MapSide {
    viewer.playhead_map.as_mut().expect("the window's reader")
}

/// Frames at `hour` until `done` holds.
fn frames_at(side: &mut MapSide, hour: Hour, mut done: impl FnMut(&MapSide) -> bool) {
    loop {
        let frame = side.frame(hour).unwrap();
        if done(side) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

/// Frames at `hour` until the map shows it, no reset composing.
fn shown(side: &mut MapSide, hour: Hour) {
    frames_at(side, hour, |side| !side.composing() && side.last_frame().drawn == Some(hour));
}

/// A window over `side` whose clock stands at `hour`, today's pipeline never started.
fn viewer(side: MapSide, hour: Hour) -> Viewer {
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(at(hour));
    viewer.rendered_sequence = 1;
    viewer.playhead_opened = true;
    viewer.playhead_map = Some(side);
    viewer
}

fn at(hour: Hour) -> Playhead {
    Playhead { branch: BRANCH, hour, fraction: 0.0 }
}

/// The overlay laid out over the rows at the head of `hour`, and the counter layers fed the tables
/// made from them before the reader's last draw.
fn whole_at(viewer: &mut Viewer, hour: Hour) {
    viewer.overlay_laid = Some((hour, 0));
    (viewer.counters_given, viewer.counters_drawn) = (Some((hour, 0)), Some((hour, 0)));
}

/// What the end of a drawn frame records: the hour on screen, the layers' while their map is open.
fn drawn(viewer: &mut Viewer) {
    viewer.rendered = viewer.layers_on_screen().map(|(branch, hour)| Playhead { branch, hour, fraction: 0.0 });
}

fn target(hour: Hour, playing: bool) -> dashboard::protocol::DashboardMapView {
    let bounds = dashboard::protocol::ViewBounds { x: 0, y: 0, width: 256, height: 144 };
    dashboard::protocol::DashboardMapView {
        options: Default::default(),
        id: "spine".into(),
        source_id: "spine".into(),
        path: "spine".into(),
        revision: "spine".into(),
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

/// Today's installed hour `hour`, as its pipeline would have left it.
fn installed(hour: Hour) -> ShownHour {
    ShownHour {
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
    }
}

/// With today's pipeline never started, the window is not ready until the host's first picture,
/// then ready at the clock's hour; after a seek it loads until the drawn layer shows the new hour.
#[test]
fn with_todays_pipeline_never_started_the_window_is_ready_at_the_hosts_first_picture_and_loads_after_a_seek() {
    let (side, gate) = side("spine first picture");
    let mut viewer = viewer(side, 2);
    assert!(!viewer.picture_ready(), "no picture before the host's first");
    assert_eq!(viewer.layers_on_screen(), None);
    assert!(viewer.map_state(&target(2, false)).loading);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    assert!(viewer.picture_ready(), "the host's first picture is the window's");
    drawn(&mut viewer);
    assert!(viewer.map_state(&target(2, false)).loading, "no overlay is laid out yet");
    whole_at(&mut viewer, 2);
    let state = viewer.map_state(&target(2, false));
    assert_eq!((state.loading, state.hour, state.branch_id.as_deref()), (false, Some(2), Some("1")), "{state:?}");
    gate.store(false, Ordering::Release);
    viewer.anchor_playback(at(8), false, 0.0, true, Instant::now(), LAST);
    frames_at(map(&mut viewer), 8, |side| !side.composing() && side.applied() == Some(8));
    drawn(&mut viewer);
    let state = viewer.map_state(&target(8, false));
    assert_eq!((state.loading, state.hour), (true, Some(2)), "the seek's hour is applied, but the held layer still shows hour 2: {state:?}");
    gate.store(true, Ordering::Release);
    shown(map(&mut viewer), 8);
    drawn(&mut viewer);
    assert!(viewer.map_state(&target(8, false)).loading, "the overlay is laid out over hour 2's rows");
    whole_at(&mut viewer, 8);
    let state = viewer.map_state(&target(8, false));
    assert_eq!((state.loading, state.hour), (false, Some(8)), "{state:?}");
    assert!(viewer.hours.is_none(), "today's pipeline was never started");
    assert!(viewer.shown.is_none());
}

/// Today's installed hour is 2 and the host draws hour 8: the window is at hour 8 and ready there.
#[test]
fn the_hour_on_screen_is_the_hosts_drawn_hour_while_todays_installed_hour_is_another() {
    let (side, _gate) = side("spine hour on screen");
    let mut viewer = viewer(side, 8);
    viewer.shown = Some(installed(2));
    viewer.rendered = Some(at(2));
    map(&mut viewer).anchor(8, false, 0.0, true);
    shown(map(&mut viewer), 8);
    assert_eq!(viewer.layers_on_screen(), Some((BRANCH, 8)));
    whole_at(&mut viewer, 8);
    let state = viewer.map_state(&target(8, false));
    assert_eq!((state.loading, state.hour), (false, Some(8)), "today's installed hour 2 is not on screen: {state:?}");
}

/// The clock moves to another branch: the reader of the old one is let go, and the next frame opens
/// one at the clock's branch and hour.
#[test]
fn a_branch_change_lets_the_reader_go_and_opens_one_on_the_clocks_branch() {
    let (side, _gate) = side("spine branch");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    let other = Playhead { branch: BRANCH + 1, hour: 5, fraction: 0.0 };
    viewer.anchor_playback(other, false, 0.0, true, Instant::now(), LAST);
    assert!(viewer.playhead_map.is_none(), "the reader of branch {BRANCH} is let go");
    assert!(!viewer.playhead_opened, "the next frame opens a reader at the clock");
    assert_eq!(viewer.playhead.map(|clock| (clock.branch, clock.hour)), Some((BRANCH + 1, 5)));
}

/// The map was reopened over other content (another set of visual mods): the reader made over the
/// old content, whose layers draw its art, is let go, and the next frame opens one over the new.
#[test]
fn a_reader_not_made_over_the_maps_content_is_let_go_and_opens_again_over_it() {
    let (side, _gate) = side("spine content");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    viewer.follow_content(std::ptr::NonNull::dangling().as_ptr());
    assert!(viewer.playhead_map.is_none(), "the reader made over other content is let go");
    assert!(!viewer.playhead_opened, "the next frame opens a reader over the map's content");
}

/// A map opened fresh: its layers show the hour, settled, but its overlay has not been laid out
/// over the rows at the head, nor its counter layers fed the tables made from them. Its picture
/// has no counters, so it is not ready.
#[test]
fn a_map_whose_overlay_is_not_laid_out_is_not_ready() {
    let (side, _gate) = side("spine overlay");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    drawn(&mut viewer);
    assert!(!viewer.presentation_ready(at(2), false), "the layers show hour 2, the overlay nothing");
    viewer.overlay_laid = Some((2, 0));
    assert!(!viewer.presentation_ready(at(2), false), "the counter layers were not fed the tables");
    viewer.counters_given = Some((2, 0));
    assert!(!viewer.presentation_ready(at(2), false), "fed after the reader's last draw: not on screen yet");
    viewer.counters_drawn = viewer.counters_given;
    assert!(viewer.presentation_ready(at(2), false), "every part shows hour 2");
    viewer.overlay_missed = vec![TableId::Countries];
    assert!(!viewer.presentation_ready(at(2), false), "a read the overlay missed is still learned");
}

/// Today's hour is installed, no art decodes and the overlay and counters show hour 8, but a layer
/// held on its worker still shows hour 2: the load is not over, so the content kept for it is not
/// let go, until the layer shows hour 8 too. On the base the load ended with the layer behind,
/// under the spinner.
#[test]
fn the_load_ends_only_once_the_picture_asked_for_is_whole() {
    let (side, gate) = side("spine load complete");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    gate.store(false, Ordering::Release);
    viewer.anchor_playback(at(8), false, 0.0, true, Instant::now(), LAST);
    frames_at(map(&mut viewer), 8, |side| !side.composing() && side.applied() == Some(8));
    drawn(&mut viewer);
    whole_at(&mut viewer, 8);
    viewer.shown = Some(ShownHour { epoch: viewer.epoch, ..installed(8) });
    assert!(!viewer.load_complete(false, true), "the held layer still shows hour 2");
    gate.store(true, Ordering::Release);
    shown(map(&mut viewer), 8);
    drawn(&mut viewer);
    assert!(viewer.load_complete(false, true), "every part shows hour 8");
    assert!(!viewer.load_complete(true, true) && !viewer.load_complete(false, false), "art decoding, or an overlay laid out before it, holds the load");
}

/// The day and night playback took a fresh start since `hour` was last given to it: a fresh one
/// shows the target at once, a running one eases toward it over a frame clock that has not moved.
fn day_night_reset(viewer: &mut Viewer, hour: Hour) -> bool {
    let marker = DayNight { hour_fraction: 0.77, ..DayNight::at_hour(hour) };
    viewer.day_night.update(hour, marker, 0.0, 1.0).hour_fraction == marker.hour_fraction
}

/// A running day and night playback at `hour`.
fn running(viewer: &mut Viewer, hour: Hour) {
    viewer.day_night = Default::default();
    viewer.day_night.update(hour, DayNight::at_hour(hour), 0.0, 1.0);
    viewer.audio_reset = false;
}

/// Playing forward continues; a seek, a step back, a viewing-country change, a gap crossed and a
/// branch change reset, each as today's rule says, with day and night and the audio reset on
/// exactly those pictures.
#[test]
fn the_windows_histories_follow_todays_continuity_over_the_hosts_pictures() {
    let (side, _gate) = side("spine continuity");
    let mut viewer = viewer(side, 2);
    let gaps = [Gap { branch: BRANCH, first_hour: 5, last_hour: 6, reason: "recorder paused".into() }];
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    viewer.presented.reopened = true;
    assert_eq!(viewer.follow_continuity(&gaps), Some((Continuity::Reset, UnitRestart::All)), "a reopened reader's first picture");
    assert!(viewer.audio_reset);
    assert_eq!(viewer.follow_continuity(&gaps), None, "the same picture presents nothing new");
    running(&mut viewer, 2);
    viewer.anchor_playback(at(4), false, 0.0, false, Instant::now(), LAST);
    shown(map(&mut viewer), 4);
    assert_eq!(viewer.follow_continuity(&gaps), Some((Continuity::Continue, UnitRestart::Keep)), "forward on the branch");
    assert!(!viewer.audio_reset);
    assert!(!day_night_reset(&mut viewer, 4));
    running(&mut viewer, 4);
    viewer.anchor_playback(at(3), false, 0.0, true, Instant::now(), LAST);
    shown(map(&mut viewer), 3);
    assert_eq!(viewer.follow_continuity(&gaps), Some((Continuity::Reset, UnitRestart::All)), "the picture composed for the window's seek");
    assert!(viewer.audio_reset);
    running(&mut viewer, 3);
    assert_eq!(viewer.present_continuity((BRANCH, 4, None), false, &gaps), Some((Continuity::Continue, UnitRestart::Keep)));
    assert!(!viewer.audio_reset);
    assert!(!day_night_reset(&mut viewer, 4));
    running(&mut viewer, 4);
    assert_eq!(viewer.present_continuity((BRANCH, 3, None), false, &gaps), Some((Continuity::Reset, UnitRestart::All)), "a step back");
    assert!(viewer.audio_reset);
    running(&mut viewer, 3);
    assert_eq!(viewer.present_continuity((BRANCH, 3, Some("7".into())), false, &gaps), Some((Continuity::Reset, UnitRestart::ViewingCountry)), "another viewing country");
    assert!(viewer.audio_reset);
    assert!(day_night_reset(&mut viewer, 3));
    running(&mut viewer, 3);
    assert_eq!(viewer.present_continuity((BRANCH, 4, Some("7".into())), false, &gaps), Some((Continuity::Continue, UnitRestart::Keep)));
    running(&mut viewer, 4);
    assert_eq!(viewer.present_continuity((BRANCH, 8, Some("7".into())), false, &gaps), Some((Continuity::Reset, UnitRestart::Keep)), "a recorded gap crossed");
    assert!(viewer.audio_reset);
    assert!(day_night_reset(&mut viewer, 8));
    running(&mut viewer, 8);
    assert_eq!(viewer.present_continuity((BRANCH + 1, 8, Some("7".into())), false, &gaps), Some((Continuity::Reset, UnitRestart::All)), "another branch");
    assert!(viewer.audio_reset);
    assert!(day_night_reset(&mut viewer, 8));
}

/// A newly chosen map mode is the mode on screen only once the picture for it is settled.
#[test]
fn the_mode_on_screen_stays_the_previous_one_until_the_picture_for_the_new_one_is_settled() {
    let (side, gate) = side("spine mode");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    assert_eq!(*viewer.shown_mode(), MapMode::default());
    let diplomacy = MapMode::Stock(Stock::Diplomacy);
    viewer.map_mode = diplomacy.clone();
    gate.store(false, Ordering::Release);
    map(&mut viewer).set_view(View { map_mode: diplomacy.clone(), ..View::default() }).unwrap();
    map(&mut viewer).frame(2).unwrap();
    map(&mut viewer).frame(2).unwrap();
    assert_eq!(*viewer.shown_mode(), MapMode::default(), "the layer has not settled the new mode");
    gate.store(true, Ordering::Release);
    frames_at(map(&mut viewer), 2, |side| side.mode_on_screen() == Some(&diplomacy));
    assert_eq!(*viewer.shown_mode(), diplomacy);
}
