use super::{Engagements, Index};
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
    engagement_history::{Covered, History, LAND, Provenance},
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
        history.branches.lock().unwrap().insert(3, covered);
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


/// Combat 100 from hour 0, 101 from hour 6 and gone at hour 10.
fn passing(at: u32, store: &mut replay_columnar::store::Store) {
    complete(store, at, &[TableId::LandCombats, TableId::NavalCombats, TableId::NavalCombatDetails]);
    match at {
        0 => put(store, TableId::LandCombats, 0, Some(100), &[("province", Cell::U32(5))]),
        6 => put(store, TableId::LandCombats, 1, Some(101), &[("province", Cell::U32(6))]),
        10 => store.table(TableId::LandCombats.index()).unwrap().delete(1).unwrap(),
        _ => {}
    }
}

/// The index of [`passing`]'s combats: 100 is engaged from hour 7, 101 is not, through hour 15.
fn folded() -> Arc<History> {
    let history = Arc::new(History::default());
    let mut covered = Covered::new(0);
    covered.last = 15;
    covered.viewers.push((0, 1));
    covered.fold(false).record(LAND, 0, &[((100, 0), Engaged::No), ((100, 1), Engaged::No)], &[]);
    covered.fold(false).record(LAND, 6, &[((101, 0), Engaged::No), ((101, 1), Engaged::No)], &[]);
    covered.fold(false).record(LAND, 7, &[((100, 0), Engaged::Yes)], &[]);
    history.branches.lock().unwrap().insert(0, covered);
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
    // Hours 1 to 4 change no flag and no row: the step reads none. The step over hour 6 reads the
    // combat that began, the one over hour 7 the combat whose flag the index flipped, and the one
    // over hour 10 the combat whose row went (dropped, never read).
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
    // The flags stepped to equal the flags composed whole from the same mirrors.
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    let (whole, _, whole_provenance) = fresh.flags();
    assert_eq!((layer(&side).flags().0, provenance), (whole, whole_provenance));
}

/// A whole settle waits for the background fold, but a fold that cannot answer (cancelled at
/// shutdown, or one the head has outrun) leaves the flags unknown and never refuses the settle: a
/// background pass must not stop the window, and the layers that read the flags name the gap.
#[test]
fn a_whole_settle_whose_fold_cannot_answer_leaves_the_flags_unknown() {
    let history = Arc::new(History::default());
    // A cancelled fold answers at once, so the wait's limit is never spent.
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

/// The same settle, once the fold covers the hour: it waits for nothing and reads the folded flags.
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
