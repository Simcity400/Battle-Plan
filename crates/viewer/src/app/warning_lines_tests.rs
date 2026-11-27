use super::{Config, MAP_LINES_LOADING, Options, Viewer};
use viewer_layer_core::{layers::View, test_support::recorded::record};
use viewer_layers_infrastructure::strikes::Strikes;
use crate::playhead_host::{MapSide, open, views::tests::hourly_steps};
use viewer_playback::limitations;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use viewer_models::strike_effects;

#[test]
fn before_the_head_reaches_an_hour_the_lines_are_the_maps_whole_list() {
    let units = record("warning lines", 12, |at, store| {
        let status = if at >= 2 { Status::Truncated { missing: 3 } } else { Status::Complete };
        store.status(TableId::NuclearStrikes.index(), status, true, Some(at as i32), true).unwrap();
    });
    let side = MapSide::with_products(open(units, hourly_steps()).unwrap(), vec![Box::new(Strikes::new(Arc::new(|_| None)))], View::default()).unwrap();
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    let mut side = viewer_playback::side::SideThread::new(side, Arc::new(|| {})).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    let [lines, _] = viewer.hour_lines().expect("the open map's lines, though it shows no hour");
    assert_eq!(lines, [MAP_LINES_LOADING], "loading by name before the head reaches an hour");
    let side = viewer.playhead_map.as_mut().unwrap();
    side.anchor(4, false, 0.0, true);
    let deadline = Instant::now() + Duration::from_secs(120);
    while side.composing() || side.applied() != Some(4) || side.with_side(|side| side.parts().is_some_and(|(_, layers)| layers[0].carried().entities > 0)) {
        let frame = side.driven(|side| side.frame(4)).unwrap();
        assert!(Instant::now() < deadline, "hour 4 applied and settled within 120 s: applied {:?}, composing {}", side.applied(), side.composing());
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    let [lines, _] = viewer.hour_lines().expect("the open map's lines");
    let side = viewer.playhead_map.as_ref().unwrap();
    assert_eq!(lines, side.with_side(|side| limitations::lines(side)).unwrap(), "the map's whole list at the head, nothing appended");
    assert!(lines.iter().any(|line| line.starts_with("Table ")), "the tables' notes at the head: {lines:?}");
    let note = format!("Field Nuclear impact effects: {}", strike_effects::truncated(3));
    assert!(lines.contains(&note), "the strikes layer's own note, in today's words: {lines:?}");
    assert!(!lines.iter().any(|line| line.contains("not given by the playhead's layers")), "no general line in place of the notes: {lines:?}");
}

#[test]
fn a_refused_vision_is_named_on_the_map_without_diagnostics() {
    use viewer_layer_core::vision::{Content, Vision};
    let units = record("refused vision", 4, |_, _| {});
    let vision = Vision::new(Arc::new(Content::new(Err("no fog content".into()))));
    let side = MapSide::with_products(open(units, hourly_steps()).unwrap(), vec![Box::new(vision)], View::default()).unwrap();
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    assert!(!viewer.diagnostics, "the diagnostics are off");
    let mut side = viewer_playback::side::SideThread::new(side, Arc::new(|| {})).unwrap();
    side.anchor(2, false, 0.0, true);
    let deadline = Instant::now() + Duration::from_secs(120);
    while side.products().vision_refused.is_none() || side.on_screen().is_none() {
        let frame = side.driven(|side| side.frame(2)).unwrap();
        assert!(Instant::now() < deadline, "the vision refused within 120 s: applied {:?}, composing {}", side.applied(), side.composing());
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    let why = side.products().vision_refused.clone().unwrap();
    viewer.playhead_map = Some(side);
    let line = viewer.vision_refusal_line().expect("the refusal is named on the map");
    assert!(line.contains(&why) && line.starts_with(limitations::NOT_GIVEN), "{line}");
}
