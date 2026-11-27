use viewer_layer_core::test_support::host_recording::{Sum, record, units, busy, quick};
use super::SideThread;
use viewer_layer_core::layers::{Layer, View};
use crate::map_open::MapSide;
use viewer_layer_core::playhead_host::{Head, open};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn side(name: &str, layer: Box<dyn Layer>) -> SideThread {
    let (recording, _) = record(1, 8, busy);
    let entry = open(units(recording, name), quick()).unwrap();
    let side = SideThread::new(MapSide::with_products(entry, vec![layer], View::default()).unwrap(), Arc::new(|| {})).unwrap();
    side.anchor(0, false, 1.0, true);
    side
}

#[test]
fn the_frames_window_never_waits_while_the_sides_thread_prepares_a_step() {
    let composes = Arc::new(Mutex::new(0));
    let armed = Arc::new(AtomicBool::new(false));
    let (mut layer, release, began) = Sum::held_settle(&composes);
    layer.settle_armed = Some(Arc::clone(&armed));
    let mut side = side("window never waits", Box::new(layer));
    side.driven(|side| viewer_layer_core::test_support::recorded::frames_at(side, 0, "composed at hour 0", |side, frame| frame.swapped.is_some() && !side.composing()));
    armed.store(true, Ordering::Release);
    side.ask(View::default(), Head::Paced(7), None, None);
    began.recv().expect("the layer entered settle while holding the side");
    for _ in 0..100 {
        assert!(side.window().is_none(), "no window while the side's thread holds the side");
        assert!(side.products().missed.is_some(), "the frame is told what the side's thread was doing, rather than waiting for it");
    }
    release.send(()).unwrap();
    side.wait_window(|side| side.applied() == Some(7)).unwrap();
    assert_eq!(side.products().missed, None, "the window is given again once settle finishes");
}

#[test]
fn a_view_asked_between_two_steps_is_taken_at_the_next_one() {
    let composes = Arc::new(Mutex::new(0));
    let mut side = side("view between steps", Box::new(Sum::new(&composes)));
    let high = View { camera_height: 4.0, ..View::default() };
    side.ask(View::default(), Head::Paced(4), None, None);
    side.wait_window(|side| side.applied() == Some(4)).unwrap();
    side.ask(high, Head::Paced(6), None, None);
    side.wait_window(|side| side.view().camera_height == 4.0).unwrap();
    let heights = side.with_side(|side| side.parts().unwrap().1[0].any().downcast_ref::<Sum>().unwrap().viewed.clone());
    assert!(heights.contains(&4.0), "the camera the view asked for reached the layer: {heights:?}");
    assert_eq!(side.view().camera_height, 4.0, "and the products say the view its picture stands at");
}

#[test]
fn a_stepped_frame_is_told_when_the_side_stops_in_its_step() {
    let composes = Arc::new(Mutex::new(0));
    let (layer, release, began) = Sum::settle_stops(&composes);
    let mut side = side("stepped failure", Box::new(layer));
    side.ask(View::default(), Head::Stepped(5), None, None);
    began.recv().expect("the side's thread settles the step");
    let _ = release.send(());
    assert!(side.stepped_window(5).is_none(), "no window once the side has stopped");
    let why = side.failure().expect("the frame is told why it has no window");
    assert!(why.contains("the test layer settles no further"), "named by the layer that stopped it: {why}");
}

#[test]
fn a_stepped_frame_refuses_by_name_rather_than_wait_for_a_picture_nothing_will_make() {
    let composes = Arc::new(Mutex::new(0));
    let mut side = side("stepped bound", Box::new(Sum::new(&composes).times_out_at(6)));
    side.ask(View::default(), Head::Stepped(3), None, None);
    drop(side.stepped_window(3).expect("the picture of the hour asked"));
    let short = (Duration::ZERO, Duration::ZERO);
    assert!(side.whole_at(6, short.0, short.1).is_none(), "no window for an hour nothing stands at");
    let why = side.take_refusal().expect("refused by name");
    assert!(why.contains("hour 6") && why.contains("Stepped(3)") && why.contains("whole at Some(3)"), "says what it waited for: {why}");
    assert!(side.failure().is_none(), "the side does not stop for a capture's refusal");
    drop(side.stepped_window(3).expect("the side still answers the frames after"));
    side.ask(View::default(), Head::Stepped(6), None, None);
    assert!(side.stepped_window(6).is_none(), "no window for an hour not made whole");
    let why = side.take_refusal().expect("refused by name");
    assert!(why.contains("times out once"), "says why: {why}");
    assert!(side.failure().is_none(), "the side does not stop for a capture's refusal");
    side.ask(View::default(), Head::Stepped(6), None, None);
    drop(side.stepped_window(6).expect("the refused hour asked again is made whole"));
    assert!(side.take_refusal().is_none());
}

#[test]
fn a_seek_while_catching_up_leaves_the_rows_driven_at_the_stepped_hour() {
    let composes = Arc::new(Mutex::new(0));
    let (layer, release, began) = Sum::held_settle(&composes);
    let mut side = side("seek while catching up", Box::new(layer));
    side.ask(View::default(), Head::Stepped(6), None, None);
    began.recv().expect("the side's thread settles within its catch-up");
    let before = side.entry().playhead().epoch();
    side.entry().anchor(2, false, 1.0, true);
    assert_ne!(side.entry().playhead().epoch(), before, "the seek is in force");
    let _ = release.send(());
    let mut window = side.stepped_window(6).expect("the picture of the hour asked");
    window.take(Default::default());
    drop(window);
    assert_eq!(side.applied(), Some(6), "the side stands at the stepped hour, so its views do");
}

#[test]
fn a_stepped_frames_window_is_the_picture_of_exactly_its_hour() {
    let composes = Arc::new(Mutex::new(0));
    let mut side = side("stepped picture", Box::new(Sum::new(&composes)));
    for hour in [3, 5, 6] {
        side.ask(View::default(), Head::Stepped(hour), None, None);
        let mut window = side.stepped_window(hour).expect("the picture of the hour asked");
        window.take(Default::default());
        drop(window);
        assert_eq!(side.applied(), Some(hour), "every step to the hour is applied");
        assert_eq!(side.last_frame().drawn, Some(hour), "and every layer shows it");
    }
    let high = View { camera_height: 4.0, ..View::default() };
    side.ask(high, Head::Stepped(6), None, None);
    let mut window = side.stepped_window(6).expect("the picture of the view asked");
    window.take(Default::default());
    drop(window);
    assert_eq!(side.view().camera_height, 4.0, "the picture stands at the view asked");
}

#[derive(Default)]
struct Answering {
    answer: Option<(replay_data::Hour, bool)>,
    joined: Option<(replay_data::Hour, bool)>,
}

struct Answers(Arc<Mutex<Answering>>);

impl viewer_layer_core::playhead_host::Overlaying<crate::map_open::Products> for Answers {
    fn ask(&mut self, _: crate::overlay::Ask) {}
    fn ask_step(&mut self, _: viewer_layer_core::playhead_host::OverlayStep<'_>, _: bool, _: bool, _: Arc<viewer_unit_reads::counter_move::MovedHours>) -> bool {
        false
    }
    fn join(&mut self, _: &[Box<dyn Layer>], _: &viewer_foundation::perspective::Choice, _: Option<replay_data::BranchId>, _: viewer_layer_core::layers::Settling) -> bool {
        let mut held = self.0.lock().unwrap();
        held.joined = held.answer;
        true
    }
    fn stepped(&mut self) {}
    fn stands_at(&self, hour: replay_data::Hour) -> bool {
        self.hour().is_some_and(|held| held >= hour)
    }
    fn take(&mut self) -> Option<crate::overlay::Made> {
        None
    }
    fn reset(&mut self) {}
    fn generation(&self) -> u64 {
        0
    }
    fn hour(&self) -> Option<replay_data::Hour> {
        self.0.lock().unwrap().joined.map(|(hour, _)| hour)
    }
    fn loading(&self) -> &[String] {
        &[]
    }
    fn missed(&self) -> &[replay_data::rows::TableId] {
        &[]
    }
    fn wants_turn(&mut self) -> bool {
        let held = self.0.lock().unwrap();
        held.answer != held.joined
    }
    fn refused(&self) -> bool {
        self.0.lock().unwrap().joined.is_some_and(|(_, refused)| refused)
    }
}

fn still(name: &str, overlay: Option<Answers>) -> (SideThread, Arc<AtomicUsize>) {
    let composes = Arc::new(Mutex::new(0));
    let (recording, _) = record(1, 8, busy);
    let entry = open(units(recording, name), quick()).unwrap();
    let mut side = MapSide::with_products(entry, vec![Box::new(Sum::new(&composes))], View::default()).unwrap();
    if let Some(overlay) = overlay {
        side.set_overlay(Box::new(overlay));
    }
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    let side = SideThread::new(side, Arc::new(move || {
        woken.fetch_add(1, Ordering::AcqRel);
    })).unwrap();
    side.anchor(0, false, 1.0, true);
    (side, wakes)
}

fn settled(side: &mut SideThread, hour: replay_data::Hour, mode: &viewer_map_modes::map_mode::MapMode) {
    side.wait_window(|side| side.applied() == Some(hour) && side.mode_on_screen() == Some(mode) && side.on_screen().is_some_and(|on| on.settled && on.drawn == Some(hour))).unwrap();
}

fn rests(side: &mut SideThread, wakes: &AtomicUsize, view: &View, why: &str) -> usize {
    let (made, woken) = (side.with_side(|side| side.last_frame().made), wakes.load(Ordering::Acquire));
    for _ in 0..3 {
        side.ask(view.clone(), Head::Clock, None, None);
        side.looked(8);
    }
    assert_eq!(side.with_side(|side| side.last_frame().made), made, "{why}: the side made no turn while nothing changed");
    assert_eq!(wakes.load(Ordering::Acquire), woken, "{why}: and gave the window no frame");
    woken
}

#[test]
fn a_still_map_is_given_a_frame_for_each_change_and_none_while_nothing_changes() {
    use viewer_map_modes::map_mode::{MapMode, Stock};
    let (mut side, wakes) = still("still map", None);
    let view = View::default();
    side.ask(view.clone(), Head::Clock, None, None);
    settled(&mut side, 0, &MapMode::default());
    let woken = rests(&mut side, &wakes, &view, "the first picture");
    assert!(woken > 0, "the first picture reached the window through the side's wakes");
    let factions = View { map_mode: MapMode::Stock(Stock::Factions), ..View::default() };
    side.ask(factions.clone(), Head::Clock, None, None);
    settled(&mut side, 0, &factions.map_mode);
    let changed = rests(&mut side, &wakes, &factions, "a paused mode change");
    assert!(changed > woken, "a paused mode change woke the window");
    side.anchor(5, false, 1.0, true);
    settled(&mut side, 5, &factions.map_mode);
    let sought = rests(&mut side, &wakes, &factions, "a paused seek");
    assert!(sought > changed, "a paused seek woke the window with no new ask from it");
}

#[test]
fn a_late_overlay_answer_and_a_refusal_each_give_one_frame_and_then_none() {
    use viewer_map_modes::map_mode::MapMode;
    let answering = Arc::new(Mutex::new(Answering::default()));
    let (mut side, wakes) = still("late overlay", Some(Answers(Arc::clone(&answering))));
    let view = View::default();
    side.ask(view.clone(), Head::Clock, None, None);
    settled(&mut side, 0, &MapMode::default());
    let woken = rests(&mut side, &wakes, &view, "no overlay answer yet");
    answering.lock().unwrap().answer = Some((0, false));
    side.wait_window(|side| side.products().overlay_hour == Some(0)).unwrap();
    assert_eq!(rests(&mut side, &wakes, &view, "the late answer, shown"), woken + 1, "the late answer gave the window one frame");
    assert!(side.products().overlay_coming());
    answering.lock().unwrap().answer = Some((0, true));
    side.wait_window(|side| side.products().overlay_refused).unwrap();
    assert_eq!(rests(&mut side, &wakes, &view, "the refusal, shown"), woken + 2, "the refusal gave the window one frame");
    assert!(!side.products().overlay_coming(), "a refused overlay is not waited for: the layers make the picture whole");
    assert!(side.products().overlay_withdrawn, "and the overlay of the answer before is withdrawn, not drawn as this one");
}
