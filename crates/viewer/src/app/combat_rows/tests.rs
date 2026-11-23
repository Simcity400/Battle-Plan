
use crate::{Config, Options, app::{Playhead, Viewer}, playhead_host::{MapSide, open}};
use viewer_layer_core::{layers::View, engagements::{Engagements, Index}, test_support::sound_recording::{complete, quick, record, sought, units}};
use replay_data::rows::TableId;
use std::sync::{Arc, Mutex};
use viewer_engagements::engagement_history::History;

#[test]
fn the_marker_hovers_and_the_byte_check_take_the_engaged_index_from_the_engagements_layer() {
    let history = Arc::new(History::default());
    let recording = record(8, |at, store| complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]));
    let entry = open(units(recording, "engaged index from the layer"), quick()).unwrap();
    let layer = Engagements::new(Some(Index { history: Arc::clone(&history), branch: 3 }), Some(1));
    let mut side = MapSide::with_products(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    sought(&mut side, 4);
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(Playhead { branch: 3, hour: 4, fraction: 0.0 });
    let mut side = viewer_playback::side::SideThread::new(side).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    let taken = viewer.layer_engaged();
    assert!(taken.is_some_and(|taken| Arc::ptr_eq(&taken, &history)), "the marker hovers read the engagements layer's index");
    let checked = viewer.layer_history();
    assert!(checked.is_some_and(|checked| Arc::ptr_eq(&checked, &history)), "the engaged byte check reads the same index");
    viewer.playhead_map = None;
    let today = viewer.layer_engaged();
    assert!(today.is_none(), "no playhead map: today's pipeline's index, none without the pipeline");
}

#[test]
fn the_combat_window_stands_at_the_hour_the_map_shows_not_the_hour_the_side_applied() {
    let recording = record(8, |at, store| complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]));
    let entry = open(units(recording, "combat window hour"), quick()).unwrap();
    let layer = Engagements::new(Some(Index { history: Arc::new(History::default()), branch: 3 }), Some(1));
    let mut side = MapSide::with_products(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    side.set_branch(3);
    sought(&mut side, 4);
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    let mut side = viewer_playback::side::SideThread::new(side).unwrap();
    side.driven(|_| ());
    assert_eq!((side.applied(), side.branch()), (Some(4), Some(3)), "the side applied hour 4");
    let (views, branch) = (side.views().clone(), 3);
    viewer.playhead_map = Some(side);
    views.show_through(Some(3));
    let on = viewer.combat_on_screen();
    assert_eq!(on, Some(viewer_combat::OnScreen { branch, hour: 3, seek: views.seek() }), "the map still shows hour 3: the window stands there, not at hour 4");
    views.show_through(None);
    assert_eq!(viewer.combat_on_screen().map(|on| on.hour), Some(4), "a saved frame: the side's own hour");
}
