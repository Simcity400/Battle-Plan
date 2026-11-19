//! The window's marker hovers, combat window and engaged byte check take the engaged index from the
//! engagements layer while the playhead's map is open, with today's pipeline never started; today's
//! pipeline's only with no playhead map.

use crate::{
    Config, Options,
    app::{Playhead, Viewer},
    layers::{
        View,
        engagements::{Engagements, Index},
        sounds::tests::{complete, quick, record, sought, units},
    },
    playhead_host::{MapSide, open},
};
use replay_data::rows::TableId;
use std::sync::{Arc, Mutex};
use viewer_foundation::engagement_history::History;

#[test]
fn the_marker_hovers_the_combat_window_and_the_byte_check_take_the_engaged_index_from_the_engagements_layer() {
    let history = Arc::new(History::default());
    let recording = record(8, |at, store| complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]));
    let entry = open(units(recording, "engaged index from the layer"), quick()).unwrap();
    let layer = Engagements::new(Some(Index { history: Arc::clone(&history), branch: 3 }), Some(1));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    sought(&mut side, 4);
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(Playhead { branch: 3, hour: 4, fraction: 0.0 });
    let mut side = crate::playhead_host::side::SideThread::new(side).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    let taken = viewer.layer_engaged();
    assert!(taken.is_some_and(|taken| Arc::ptr_eq(&taken, &history)), "the marker hovers and the combat window read the engagements layer's index");
    let checked = viewer.layer_history();
    assert!(checked.is_some_and(|checked| Arc::ptr_eq(&checked, &history)), "the engaged byte check reads the same index");
    viewer.playhead_map = None;
    let today = viewer.layer_engaged();
    assert!(today.is_none(), "no playhead map: today's pipeline's index, none without the pipeline");
}
