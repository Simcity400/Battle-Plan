use super::{Engagements, Index, KINDS};
use crate::{
    layers::{
        Layer, View,
        sounds::tests::{complete, play, put, quick, record, sought, units},
    },
    playhead_host::{MapSide, open},
};
use replay_columnar::state::Cell;
use replay_data::rows::TableId;
use std::sync::Arc;
use viewer_foundation::{
    engaged::Engaged,
    engagement_history::{Covered, History, LAND, NAVAL, Provenance},
};

/// Combat 100 from hour 0, combat 101 from hour 6.
fn combats(at: u32, store: &mut replay_columnar::store::Store) {
    complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]);
    match at {
        0 => put(store, TableId::LandCombats, 0, Some(100), &[("province", Cell::U32(5))]),
        6 => put(store, TableId::LandCombats, 1, Some(101), &[("province", Cell::U32(6))]),
        _ => {}
    }
}

fn layer(side: &MapSide) -> &Engagements {
    side.parts().unwrap().1[0].any().downcast_ref::<Engagements>().unwrap()
}

#[test]
fn an_hour_the_index_has_not_reached_shows_unknown_and_a_covered_one_the_folded_flags() {
    let history = Arc::new(History::default());
    let entry = open(units(record(16, combats), "engagements"), quick()).unwrap();
    let index = Index { history: Arc::clone(&history), branch: 3 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Engagements::new(Some(index), Some(1)))], View::default()).unwrap();
    sought(&mut side, 4);
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!(land.get(&100), Some(&Engaged::Unknown), "never a guess");
    assert_eq!(provenance[LAND], Provenance::Pending);
    {
        let mut covered = Covered::new(0);
        covered.last = 9;
        covered.viewers.push((0, 1));
        covered.points[LAND].insert((100, 0), vec![(0, Engaged::No), (7, Engaged::Yes)]);
        covered.points[LAND].insert((100, 1), vec![(0, Engaged::No)]);
        covered.points[LAND].insert((101, 0), vec![(6, Engaged::No)]);
        covered.points[LAND].insert((101, 1), vec![(6, Engaged::No)]);
        history.branches.lock().unwrap().insert(3, covered);
    }
    let mut seen = Vec::new();
    play(&mut side, 4, |side, frame| {
        let hour = frame.applied.unwrap();
        let (land, _, provenance) = layer(side).flags();
        let published = layer(side).changed().expect("it publishes its changes");
        let mut changed: Vec<(usize, u64)> = [LAND, NAVAL].into_iter().flat_map(|kind| published.of(KINDS[kind]).iter().map(move |&combat| (kind, combat))).collect();
        changed.sort_unstable();
        seen.push((hour, land.clone(), provenance[LAND], changed));
        hour >= 12
    });
    for (hour, _, _, changed) in &seen {
        match hour {
            8 => assert_eq!(changed, &[(LAND, 100), (LAND, 101)], "100 flipped at 7 and 101 began at 6"),
            12 => assert_eq!(changed, &[(LAND, 100), (LAND, 101)], "past the index's last hour every combat is restated"),
            _ => {}
        }
    }
    for (hour, land, provenance, _) in seen {
        if hour <= 9 {
            assert_eq!(provenance, Provenance::Indexed, "hour {hour}");
            assert_eq!(land[&100], if hour >= 7 { Engaged::Yes } else { Engaged::No }, "hour {hour}");
            assert_eq!(land.get(&101).copied(), (hour >= 6).then_some(Engaged::No), "hour {hour}");
        } else {
            assert_eq!(provenance, Provenance::Pending, "hour {hour}: past what the index folded");
            assert!(land.values().all(|flag| *flag == Engaged::Unknown));
        }
    }
}
