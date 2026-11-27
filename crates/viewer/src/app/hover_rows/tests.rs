
use crate::{Config, Options, app::{Playhead, Viewer}, playhead_host::{MapSide, open, views::tests::hourly_steps}};
use viewer_layer_core::{layers::View, test_support::{recorded, sound_recording::sought}, vision};
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn vision_content() -> Arc<vision::Content> {
    use viewer_foundation::visibility::{FogContent, Topology, test_defines};
    let neighbors = BTreeMap::from([(1, vec![2]), (2, vec![1])]);
    Arc::new(vision::Content::new(Ok(FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors }, defines: test_defines() })))
}

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
        for table in viewer_foundation::perspective::VISION_TABLES {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    })
}

#[test]
fn the_hovers_read_the_vision_the_maps_vision_layer_shows_and_are_loading_by_name_until_it_shows_one() {
    let deadline = Instant::now() + Duration::from_secs(120);
    let entry = open(recording("hover vision from the layer"), hourly_steps()).unwrap();
    let side = MapSide::with_products(Arc::clone(&entry), vec![Box::new(vision::Vision::new(vision_content()))], View::default()).unwrap();
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(Mutex::new(None)), None, None);
    viewer.playhead = Some(Playhead { branch: 0, hour: 2, fraction: 0.0 });
    let mut side = viewer_playback::side::SideThread::new(side, std::sync::Arc::new(|| {})).unwrap();
    side.driven(|_| ());
    viewer.playhead_map = Some(side);
    let before = viewer.shown_vision().err();
    assert!(before.as_deref().is_some_and(|why| !why.is_empty()), "no vision before the layer composed, and why by name: {before:?}");
    viewer.playhead_map.as_ref().unwrap().with_side_mut(|side| sought(side, 2));
    let made = viewer_layer_core::vision::seen_made_here();
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
    assert_eq!(viewer_layer_core::vision::seen_made_here(), made, "the frame's thread copied the layer's arrays and made no vision");
    assert_eq!(vision.ours(), 1, "the viewing country the layer resolved");
    let seen = viewer.playhead_map.as_ref().unwrap().with_side(|side| viewer_layer_core::vision::drawn(side.layer_parts()).expect("the vision layer draws").vision().expect("the layer's vision"));
    assert_eq!(vision.differences(&seen), (0, Vec::new()), "made off the frame, it is the layer's vision");
    let again = viewer.shown_vision().unwrap_or_else(|why| panic!("still shown: {why}"));
    assert!(Arc::ptr_eq(&vision, &again), "the same value while nothing the layer shows moves");
}

#[test]
fn a_text_shows_only_for_its_target_at_the_presented_hour() {
    use super::{Family, Hover, Key, Made, What};
    let key = |what: What, sources: [usize; 4], seen: (u32, bool)| Key { seen, ..Key::new(what, sources, (false, false)) };
    let made = key(What::Mines(7), [1, 10, 2, 3], (5, true));
    let mut hover = Hover::new(Family::Naval);
    hover.keep(Made { key: made.clone(), frame: 0, text: Some(Arc::default()), branch: 1, hour: 5, missed: Vec::new() });
    assert!(hover.shown_for(&made, Some((1, 5))).is_some());
    assert!(hover.shown_for(&made, Some((1, 6))).is_none(), "made at another hour");
    assert!(hover.shown_for(&made, Some((2, 5))).is_none(), "made on another branch");
    let settled = key(What::Mines(7), [1, 11, 2, 3], (5, true));
    assert!(hover.shown_for(&settled, Some((1, 5))).is_some(), "a new copy of the same vision within the hour");
    assert!(hover.shown_for(&settled, None).is_none(), "without a presented hour, only its own key");
    assert!(hover.shown_for(&key(What::Mines(7), [1, 11, 2, 3], (6, true)), Some((1, 5))).is_none(), "another country's vision");
    assert!(hover.shown_for(&key(What::Mines(8), [1, 10, 2, 3], (5, true)), Some((1, 5))).is_none(), "another target");
    assert!(hover.shown_for(&key(What::Mines(7), [1, 10, 4, 3], (5, true)), Some((1, 5))).is_none(), "another selection");
}

#[test]
fn texts_made_ahead_wait_for_their_hour() {
    use super::{Family, Hover, Key, Made, What};
    let key = Key::new(What::Mines(7), [1, 10, 2, 3], (false, false));
    let mut hover = Hover::new(Family::Naval);
    for hour in [6, 7, 8] {
        hover.keep(Made { key: key.clone(), frame: hour as usize, text: Some(Arc::default()), branch: 1, hour, missed: Vec::new() });
    }
    for hour in [6, 7, 8] {
        assert!(hover.shown_for(&key, Some((1, hour))).is_some(), "hour {hour}'s text is kept");
    }
    assert!(hover.shown_for(&key, Some((1, 9))).is_none());
}

#[test]
fn played_fast_each_presented_hour_has_its_text_when_presented() {
    use super::next_hour;
    use std::collections::BTreeSet;
    let play = |ahead: bool, lead: i32| {
        let mut made: BTreeSet<i32> = BTreeSet::new();
        let mut making: Option<i32> = None;
        let mut shown = Vec::new();
        for presented in 0..48 {
            if let Some(hour) = making.take() {
                made.insert(hour);
            }
            shown.push(made.contains(&presented));
            let prepared: Vec<i32> = (presented + 1..=presented + lead).collect();
            making = if ahead {
                next_hour(presented, &prepared, |hour| made.contains(&hour))
            } else {
                (!made.contains(&presented)).then_some(presented)
            };
        }
        shown
    };
    for lead in [1, 2, 4] {
        assert!(play(true, lead)[1..].iter().all(|&shown| shown), "lead {lead}: {:?}", play(true, lead));
        assert!(play(false, lead).iter().all(|&shown| !shown), "the presented hour's text always arrives an hour late");
    }
    assert_eq!(next_hour(5, &[], |_| false), Some(5));
    assert_eq!(next_hour(5, &[], |_| true), None);
    assert_eq!(next_hour(5, &[6, 7, 8], |hour| hour == 6), Some(7));
}
