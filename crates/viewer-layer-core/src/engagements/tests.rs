use super::{Engagements, Index};
use crate::{layers::{Layer, View}, test_support::sound_recording::{complete, play, put, quick, record, sought, units}, playhead_host::{MapSide, open}};
use replay_columnar::state::Cell;
use replay_data::rows::TableId;
use std::sync::Arc;
use viewer_engagements::{
    engaged::Engaged,
    engagement_history::{Covered, History, LAND, Provenance},
};

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
    let entry = open(units(record(16, combats), "engagements index"), quick()).unwrap();
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
        covered.fold(false).record(LAND, 0, &[((100, 0), Engaged::No), ((100, 1), Engaged::No)], &[]);
        covered.fold(false).record(LAND, 6, &[((101, 0), Engaged::No), ((101, 1), Engaged::No)], &[]);
        covered.fold(false).record(LAND, 7, &[((100, 0), Engaged::Yes)], &[]);
        history.insert_branch(3, covered);
    }
    let mut seen = Vec::new();
    play(&mut side, 4, |side, frame| {
        let hour = frame.applied.unwrap();
        let (land, _, provenance) = layer(side).flags();
        seen.push((hour, land.clone(), provenance[LAND]));
        hour >= 12
    });
    assert!(seen.iter().any(|(_, _, provenance)| *provenance == Provenance::Indexed), "the index answered an hour");
    for (hour, land, provenance) in seen {
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


#[test]
fn a_still_publication_waits_for_no_fold_and_the_next_reads_it_once_folded() {
    let history = Arc::new(History::default());
    let entry = open(units(record(16, combats), "engagements still"), quick()).unwrap();
    let index = Index { history: Arc::clone(&history), branch: 3 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Engagements::new(Some(index), Some(1)))], View::default()).unwrap();
    sought(&mut side, 4);
    let waited = layer(&side).fold_waits.get();
    side.frame_still(4).unwrap();
    assert_eq!(layer(&side).fold_waits.get(), waited, "a still publication enters no wait for the fold");
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!((provenance[LAND], land.get(&100)), (Provenance::Pending, Some(&Engaged::Unknown)), "pending by name, never a guess");
    {
        let mut covered = Covered::new(0);
        covered.last = 9;
        covered.viewers.push((0, 1));
        covered.fold(false).record(LAND, 0, &[((100, 0), Engaged::No), ((100, 1), Engaged::No)], &[]);
        history.insert_branch(3, covered);
    }
    side.frame_still(4).unwrap();
    assert_eq!(layer(&side).fold_waits.get(), waited, "nor does the one after the fold");
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!((provenance[LAND], land.get(&100)), (Provenance::Indexed, Some(&Engaged::No)), "the publication after the fold reads it");
}

fn passing(at: u32, store: &mut replay_columnar::store::Store) {
    complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]);
    match at {
        0 => put(store, TableId::LandCombats, 0, Some(100), &[("province", Cell::U32(5))]),
        6 => put(store, TableId::LandCombats, 1, Some(101), &[("province", Cell::U32(6))]),
        10 => store.table(TableId::LandCombats.index()).unwrap().delete(1).unwrap(),
        _ => {}
    }
}

fn folded() -> Arc<History> {
    let history = Arc::new(History::default());
    let mut covered = Covered::new(0);
    covered.last = 15;
    covered.viewers.push((0, 1));
    covered.fold(false).record(LAND, 0, &[((100, 0), Engaged::No), ((100, 1), Engaged::No)], &[]);
    covered.fold(false).record(LAND, 6, &[((101, 0), Engaged::No), ((101, 1), Engaged::No)], &[]);
    covered.fold(false).record(LAND, 7, &[((100, 0), Engaged::Yes)], &[]);
    history.insert_branch(0, covered);
    history
}

#[test]
fn a_step_reads_the_flags_its_hours_and_rows_changed_and_no_others() {
    let history = folded();
    let entry = open(units(record(16, passing), "engagements step"), quick()).unwrap();
    let index = Index { history: Arc::clone(&history), branch: 0 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Engagements::new(Some(index), Some(1)))], View::default()).unwrap();
    sought(&mut side, 0);
    let mut read: Vec<(replay_data::Hour, usize)> = Vec::new();
    let mut marked = layer(&side).marked();
    play(&mut side, 0, |side, frame| {
        let now = layer(side).marked();
        read.push((frame.applied.unwrap(), now - marked));
        marked = now;
        frame.applied.unwrap() >= 12
    });
    for &(hour, count) in &read {
        let expected = match hour {
            4 => 0,
            8 => 2,
            12 => 0,
            _ => continue,
        };
        assert_eq!(count, expected, "hour {hour} of {read:?}");
    }
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!(provenance[LAND], Provenance::Indexed);
    assert_eq!(land, &std::collections::BTreeMap::from([(100, Engaged::Yes)]), "101's row is gone");
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    let (whole, _, whole_provenance) = fresh.flags();
    assert_eq!((layer(&side).flags().0, provenance), (whole, whole_provenance));
}

#[test]
fn a_whole_settle_whose_fold_cannot_answer_leaves_the_flags_unknown() {
    let history = Arc::new(History::default());
    history.cancelled.store(true, std::sync::atomic::Ordering::Release);
    let entry = open(units(record(16, combats), "engagements fold cancelled"), quick()).unwrap();
    let index = Index { history: Arc::clone(&history), branch: 3 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Engagements::new(Some(index), Some(1)))], View::default()).unwrap();
    sought(&mut side, 4);
    side.settle_all().expect("a fold that cannot answer never refuses the settle");
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!(provenance[LAND], Provenance::Pending);
    assert_eq!(land.get(&100), Some(&Engaged::Unknown), "never a guess");
}

#[test]
fn a_whole_settle_reads_the_flags_the_fold_covers() {
    let history = folded();
    let entry = open(units(record(16, passing), "engagements fold covers"), quick()).unwrap();
    let index = Index { history: Arc::clone(&history), branch: 0 };
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Engagements::new(Some(index), Some(1)))], View::default()).unwrap();
    sought(&mut side, 8);
    side.settle_all().expect("the fold covers hour 8");
    let (land, _, provenance) = layer(&side).flags();
    assert_eq!(provenance[LAND], Provenance::Indexed);
    assert_eq!(land.get(&100), Some(&Engaged::Yes), "100 is engaged from hour 7");
}
