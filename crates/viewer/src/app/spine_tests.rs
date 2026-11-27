use super::{Config, Options, Playhead, Viewer};
use viewer_layer_core::{layers::{Carried, Settling, Inputs, Layer, Published, View}, test_support::recorded::record};
use crate::playhead_host::{MapSide, mirror::Mirrors, open, views::tests::hourly_steps};
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
    time::Instant,
};
use viewer_gpu::frame::DayNight;
use viewer_runtime::presentation::{Continuity, UnitRestart};
use viewer_map_modes::map_mode::{MapMode, Stock};

const BRANCH: BranchId = 1;
const LAST: Hour = 11;

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
    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        if self.open.load(Ordering::Acquire) {
            self.published = self.next;
            self.viewed = false;
        }
        Ok(())
    }
    fn carried(&self) -> Carried {
        Carried { entities: usize::from(self.viewed) }
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

fn side(name: &str) -> (MapSide, Arc<AtomicBool>) {
    let units = record(name, LAST as u32 + 1, |at, store| {
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    });
    let entry = open(units, hourly_steps()).unwrap();
    let gate = Arc::new(AtomicBool::new(true));
    let layer = Gated { open: Arc::clone(&gate), next: None, published: None, viewed: false };
    let mut side = MapSide::with_products(entry, vec![Box::new(layer)], View::default()).unwrap();
    side.set_branch(BRANCH);
    (side, gate)
}

fn map(viewer: &mut Viewer) -> &mut viewer_playback::side::SideThread {
    viewer.playhead_map.as_mut().expect("the window's reader")
}

fn frames_at(side: &mut viewer_playback::side::SideThread, hour: Hour, mut done: impl FnMut(&MapSide) -> bool) {
    side.with_side_mut(|side| {
        viewer_layer_core::test_support::recorded::frames_at(side, hour, &format!("hour {hour}"), |side, _| done(side));
    });
    side.driven(|_| ());
}

fn shown(side: &mut viewer_playback::side::SideThread, hour: Hour) {
    frames_at(side, hour, |side| !side.composing() && side.last_frame().drawn == Some(hour));
}

fn viewer(side: MapSide, hour: Hour) -> Viewer {
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(at(hour));
    viewer.rendered_sequence = 1;
    viewer.playhead_opened = true;
    let mut side = viewer_playback::side::SideThread::new(side).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    viewer
}

fn at(hour: Hour) -> Playhead {
    Playhead { branch: BRANCH, hour, fraction: 0.0 }
}

fn whole_at(viewer: &mut Viewer, hour: Hour) {
    viewer.overlay_hour = Some(hour);
}

fn drawn(viewer: &mut Viewer) {
    viewer.rendered = viewer.layers_on_screen().map(|(branch, hour)| Playhead { branch, hour, fraction: 0.0 });
}

fn target(hour: Hour, playing: bool) -> dashboard_core::protocol::DashboardMapView {
    let bounds = dashboard_core::protocol::ViewBounds { x: 0, y: 0, width: 256, height: 144 };
    dashboard_core::protocol::DashboardMapView {
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
        composed_at: 0.0,
        clock_id: "page-1".into(),
        bounds,
        clip: bounds,
        visible: true,
        focus_province: None,
        battle: None,
    }
}

#[test]
fn the_window_is_ready_at_the_hosts_first_picture_and_loads_after_a_seek() {
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
    viewer.anchor_playback(at(8), false, 0.0, true, None, Instant::now(), LAST);
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
}

#[test]
fn the_hour_on_screen_is_the_hosts_drawn_hour() {
    let (side, _gate) = side("spine hour on screen");
    let mut viewer = viewer(side, 8);
    viewer.rendered = Some(at(2));
    map(&mut viewer).anchor(8, false, 0.0, true);
    shown(map(&mut viewer), 8);
    assert_eq!(viewer.layers_on_screen(), Some((BRANCH, 8)));
    whole_at(&mut viewer, 8);
    let state = viewer.map_state(&target(8, false));
    assert_eq!((state.loading, state.hour), (false, Some(8)), "{state:?}");
}

#[test]
fn a_branch_change_lets_the_reader_go_and_opens_one_on_the_clocks_branch() {
    let (side, _gate) = side("spine branch");
    let entry = Arc::downgrade(side.entry());
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    viewer.head_rows.end_frame(viewer.playhead_map.as_ref().map(|side| side.views()));
    let other = Playhead { branch: BRANCH + 1, hour: 5, fraction: 0.0 };
    viewer.anchor_playback(other, false, 0.0, true, None, Instant::now(), LAST);
    assert!(viewer.playhead_map.is_none(), "the reader of branch {BRANCH} is let go");
    assert!(entry.upgrade().is_none(), "the map's click reader releases the closed playhead");
    assert!(!viewer.playhead_opened, "the next frame opens a reader at the clock");
    assert_eq!(viewer.playhead.map(|clock| (clock.branch, clock.hour)), Some((BRANCH + 1, 5)));
}

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

#[test]
fn a_map_whose_overlay_is_not_laid_out_is_not_ready() {
    let (side, _gate) = side("spine overlay");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    drawn(&mut viewer);
    assert!(!viewer.presentation_ready(at(2), false), "the layers show hour 2, the overlay nothing");
    viewer.overlay_hour = Some(2);
    assert!(viewer.presentation_ready(at(2), false), "every part shows hour 2");
    viewer.overlay_missed = vec![TableId::Countries];
    assert!(!viewer.presentation_ready(at(2), false), "a read the overlay missed is still learned");
}

#[test]
fn the_load_ends_only_once_the_picture_asked_for_is_whole() {
    let (side, gate) = side("spine load complete");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    gate.store(false, Ordering::Release);
    viewer.anchor_playback(at(8), false, 0.0, true, None, Instant::now(), LAST);
    frames_at(map(&mut viewer), 8, |side| !side.composing() && side.applied() == Some(8));
    drawn(&mut viewer);
    whole_at(&mut viewer, 8);
    assert!(!viewer.load_complete(false, true), "the held layer still shows hour 2");
    gate.store(true, Ordering::Release);
    shown(map(&mut viewer), 8);
    drawn(&mut viewer);
    assert!(viewer.load_complete(false, true), "every part shows hour 8");
    assert!(!viewer.load_complete(true, true) && !viewer.load_complete(false, false), "art decoding, or an overlay laid out before it, holds the load");
}

fn day_night_reset(viewer: &mut Viewer, hour: Hour) -> bool {
    let marker = DayNight { hour_fraction: 0.77, ..DayNight::at_hour(hour) };
    viewer.day_night.update(hour, marker, 0.0, 1.0).hour_fraction == marker.hour_fraction
}

fn running(viewer: &mut Viewer, hour: Hour) {
    viewer.day_night = Default::default();
    viewer.day_night.update(hour, DayNight::at_hour(hour), 0.0, 1.0);
    viewer.audio_reset = false;
}

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
    viewer.anchor_playback(at(4), false, 0.0, false, None, Instant::now(), LAST);
    shown(map(&mut viewer), 4);
    assert_eq!(viewer.follow_continuity(&gaps), Some((Continuity::Continue, UnitRestart::Keep)), "forward on the branch");
    assert!(!viewer.audio_reset);
    assert!(!day_night_reset(&mut viewer, 4));
    running(&mut viewer, 4);
    viewer.anchor_playback(at(3), false, 0.0, true, None, Instant::now(), LAST);
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

#[test]
fn the_mode_on_screen_stays_the_previous_one_until_the_picture_for_the_new_one_is_settled() {
    let (side, gate) = side("spine mode");
    let mut viewer = viewer(side, 2);
    map(&mut viewer).anchor(2, false, 0.0, true);
    shown(map(&mut viewer), 2);
    drawn(&mut viewer);
    whole_at(&mut viewer, 2);
    assert_eq!(*viewer.shown_mode(), MapMode::default());
    assert!(viewer.picture_whole(at(2), false));
    let diplomacy = MapMode::Stock(Stock::Diplomacy);
    viewer.map_mode = diplomacy.clone();
    assert!(!viewer.picture_whole(at(2), false), "picture is not whole until the asked mode is on screen");
    assert!(viewer.map_state(&target(2, false)).loading, "map state stays loading while the old mode is still shown");
    gate.store(false, Ordering::Release);
    let at_view = View { map_mode: diplomacy.clone(), ..View::default() };
    map(&mut viewer).with_side_mut(|side| {
        side.set_view(at_view).unwrap();
        side.frame(2).unwrap();
        side.frame(2).unwrap();
    });
    map(&mut viewer).driven(|_| ());
    assert_eq!(*viewer.shown_mode(), MapMode::default(), "the layer has not settled the new mode");
    assert!(!viewer.picture_whole(at(2), false));
    gate.store(true, Ordering::Release);
    frames_at(map(&mut viewer), 2, |side| side.mode_on_screen() == Some(&diplomacy));
    assert_eq!(*viewer.shown_mode(), diplomacy);
    assert!(viewer.picture_whole(at(2), false));
    assert!(!viewer.map_state(&target(2, false)).loading);
    let resistance = MapMode::Stock(Stock::Resistance);
    let compliance = MapMode::Stock(Stock::Compliance);
    let mut resistance_overlay = viewer.counter_view.clone();
    resistance_overlay.set_mode(&resistance);
    viewer.overlay_view = Some(resistance_overlay);
    viewer.map_mode = compliance.clone();
    let compliance_view = View { map_mode: compliance.clone(), ..View::default() };
    map(&mut viewer).with_side_mut(|side| side.set_view(compliance_view).unwrap());
    frames_at(map(&mut viewer), 2, |side| side.mode_on_screen() == Some(&compliance));
    assert!(!viewer.picture_whole(at(2), false), "picture stays incomplete while the installed overlay still has the previous mode");
    assert!(viewer.map_state(&target(2, false)).loading, "map state stays loading until the new mode's overlay installs");
    let mut compliance_overlay = viewer.counter_view.clone();
    compliance_overlay.set_mode(&compliance);
    viewer.overlay_view = Some(compliance_overlay);
    assert!(viewer.picture_whole(at(2), false));
    assert!(!viewer.map_state(&target(2, false)).loading);
}

#[test]
fn overlay_continuity_matches_destination_across_lagging_layers_and_rejects_stale_overlays() {
    let gaps = [Gap { branch: BRANCH, first_hour: 5, last_hour: 6, reason: "recorder paused".into() }];
    assert!(
        !super::overlay_matches_continuity((0, BRANCH, 8, 7), (BRANCH, 4, Some("7")), Some(0), &gaps),
        "destination overlay arriving while layers lag before the gap stays stale on Frame 1"
    );
    assert!(
        super::overlay_matches_continuity((0, BRANCH, 8, 7), (BRANCH, 8, Some("7")), Some(0), &gaps),
        "destination overlay matches once layers catch up across the gap on Frame 2"
    );
    assert!(
        !super::overlay_matches_continuity((0, BRANCH, 4, 7), (BRANCH, 8, Some("7")), Some(0), &gaps),
        "retained pre-gap overlay is not promoted across the gap"
    );
    assert!(
        !super::overlay_matches_continuity((0, BRANCH, 8, 7), (BRANCH, 8, Some("7")), Some(1), &gaps),
        "retained pre-seek overlay is rejected once overlay generation advances"
    );
    assert!(
        super::overlay_matches_continuity((1, BRANCH, 8, 7), (BRANCH, 8, Some("7")), Some(1), &gaps),
        "destination overlay from the current generation matches after seek"
    );
    assert!(
        !super::overlay_matches_continuity((0, BRANCH, 8, 7), (BRANCH, 8, Some("9")), Some(1), &gaps),
        "retained overlay from another viewing country is rejected"
    );
    assert!(
        !super::overlay_matches_continuity((0, BRANCH, 8, 7), (BRANCH + 1, 8, Some("7")), Some(0), &gaps),
        "retained overlay from another branch is rejected"
    );
}

#[test]
fn a_new_playing_origin_seeks_the_attached_map_side_to_the_new_hour() {
    let (side, _) = side("spine origin switch");
    let mut viewer = viewer(side, 2);
    let now = viewer.last_step;
    let sample = |origin, composed_at| {
        Some(crate::page_clock::ClockSample {
            origin,
            composed_at,
        })
    };
    viewer.anchor_playback(
        at(2),
        true,
        1_440.0,
        true,
        sample("page-A", 10_000.0),
        now,
        LAST,
    );
    shown(map(&mut viewer), 2);
    let before_epoch = map(&mut viewer).entry().playhead().epoch();
    viewer.anchor_playback(
        at(9),
        true,
        1_440.0,
        false,
        sample("page-B", 9_500.0),
        now + std::time::Duration::from_millis(500),
        LAST,
    );
    assert_eq!(viewer.playhead, Some(at(9)));
    assert!(
        map(&mut viewer).entry().playhead().epoch() > before_epoch,
        "a new playing page origin must issue a seek to the attached map side"
    );
    shown(map(&mut viewer), 9);
    assert_eq!(map(&mut viewer).applied(), Some(9));
}
