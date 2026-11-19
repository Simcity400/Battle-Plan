//! The hovers read the vision the playhead's map's vision layer shows, with today's pipeline never
//! started: loading by name until the layer shows one, then its value, the same value while nothing
//! the layer shows moves.

use crate::{
    Config, Options,
    app::{Playhead, Viewer},
    layers::{View, recorded, sounds::tests::sought, vision},
    playhead_host::{MapSide, open, views::tests::hourly_steps},
};
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn vision_content() -> Arc<vision::Content> {
    use viewer_foundation::map::visibility::{FogContent, Topology, test_defines};
    let neighbors = BTreeMap::from([(1, vec![2]), (2, vec![1])]);
    Arc::new(vision::Content::new(Ok(FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors }, defines: test_defines() })))
}

/// Countries 101 to 103 in slots 1 to 3, the local player 101, every vision table whole each hour.
fn recording(name: &str) -> crate::playhead_host::units::Units {
    recorded::record(name, 8, |at, store| {
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, recorded::column(table, field), cell).unwrap();
        let insert = |store: &mut Store, table: TableId, slot: u32, key: u128| store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
        if at == 0 {
            for (slot, raw) in [101u32, 102, 103].into_iter().enumerate() {
                for table in [TableId::CountryLinks, TableId::Countries] {
                    insert(store, table, slot as u32, u128::from(raw));
                    write(store, table, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                }
            }
            insert(store, TableId::LocalPlayer, 0, 0);
            write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
        }
        for table in viewer_foundation::map::perspective::VISION_TABLES {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    })
}

#[test]
fn the_hovers_read_the_vision_the_maps_vision_layer_shows_and_are_loading_by_name_until_it_shows_one() {
    let deadline = Instant::now() + Duration::from_secs(120);
    let entry = open(recording("hover vision from the layer"), hourly_steps()).unwrap();
    let side = MapSide::new(Arc::clone(&entry), vec![Box::new(vision::Vision::new(vision_content()))], View::default()).unwrap();
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(Playhead { branch: 0, hour: 2, fraction: 0.0 });
    let mut side = crate::playhead_host::side::SideThread::new(side).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    let before = viewer.shown_vision().err();
    assert!(before.as_deref().is_some_and(|why| !why.is_empty()), "no vision before the layer composed, and why by name: {before:?}");
    viewer.playhead_map.as_ref().unwrap().with_side_mut(|side| sought(side, 2));
    let made = crate::layers::vision::seen_made_here();
    let vision = loop {
        let side = viewer.playhead_map.as_mut().unwrap();
        side.driven(|side| {
            side.frame(2)?;
            side.settle_all()
        })
        .unwrap();
        if let Ok(vision) = viewer.shown_vision() {
            break vision;
        }
        assert!(Instant::now() < deadline, "the vision layer shows a vision at hour 2 within 120 s: {:?}", viewer.shown_vision().err());
        std::thread::yield_now();
    };
    assert_eq!(crate::layers::vision::seen_made_here(), made, "the frame's thread copied the layer's arrays and made no vision");
    assert_eq!(vision.ours(), 1, "the viewing country the layer resolved");
    let seen = viewer.playhead_map.as_ref().unwrap().with_side(|side| crate::layers::vision::drawn(side).expect("the vision layer draws").vision().expect("the layer's vision"));
    assert_eq!(vision.differences(&seen), (0, Vec::new()), "made off the frame, it is the layer's vision");
    let again = viewer.shown_vision().unwrap_or_else(|why| panic!("still shown: {why}"));
    assert!(Arc::ptr_eq(&vision, &again), "the same value while nothing the layer shows moves");
}
