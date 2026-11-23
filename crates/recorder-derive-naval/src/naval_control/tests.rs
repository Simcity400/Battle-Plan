use super::*;
use recorder_derive_framework::{
    forces::testing::{put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, CountryRow, FactionMemberRow, Recorded, SubjectRow, WarRelationRow};

fn feed(store: &mut Store, hour: i32, currents: Option<&[(u32, u32, i64)]>, war: bool) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    if let Some(currents) = currents {
        let rows: Vec<NavalSupremacyRow> = currents.iter().map(|&(region, country, current)| NavalSupremacyRow { region, country, current, ..Default::default() }).collect();
        put(store, hour, TableId::NavalSupremacy, &rows, Status::Complete);
    }
    let ids = [101, 102, 103, 104];
    put(store, hour, TableId::Countries, &ids.map(|id| CountryRow { id, canonical_slot: id - 100, ..Default::default() }), Status::Complete);
    put(store, hour, TableId::CountryLinks, &ids.map(|id| CountryLinkRow { id, canonical_slot: id - 100 }), Status::Complete);
    put(store, hour, TableId::FactionMembers, &[FactionMemberRow { country: 101, faction: 9, ordinal: 0 }, FactionMemberRow { country: 102, faction: 9, ordinal: 1 }], Status::Complete);
    put::<SubjectRow>(store, hour, TableId::Subjects, &[], Status::Complete);
    let wars: &[WarRelationRow] = if war { &[WarRelationRow { first_country: 101, second_country: 103, ..Default::default() }] } else { &[] };
    put(store, hour, TableId::WarRelations, wars, Status::Complete);
    put::<replay_data::rows::RelationTupleRow>(store, hour, TableId::RelationTuples, &[], Status::Complete);
    put::<AirCountryRelationRow>(store, hour, TableId::AirCountryRelations, &[], Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(NavalControl::default())]);
    writers.start_run();
    writers
}

const DAY0: &[(u32, u32, i64)] = &[(7, 101, 10), (7, 102, 5), (7, 103, 7), (8, 104, 0)];
const MOVED: &[(u32, u32, i64)] = &[(7, 101, 10), (7, 102, 6), (7, 103, 7), (8, 104, 0)];
const UNLINKED: &[(u32, u32, i64)] = &[(7, 101, 10), (7, 102, 6), (7, 103, 7), (8, 104, 0), (9, 105, 4)];

const HOURS: &[(i32, Option<&[(u32, u32, i64)]>, bool)] = &[(0, Some(DAY0), true), (1, Some(MOVED), true), (2, None, false), (3, Some(UNLINKED), false), (24, None, false), (25, Some(UNLINKED), false)];

fn table(store: &Store) -> Vec<((u32, u32), Vec<Option<i64>>)> {
    let view = store.view(TableId::NavalRegionControl.index());
    let mut out: Vec<_> = view.rows().map(|slot| {
        let key = view.key(slot).unwrap();
        (((key >> 32) as u32, key as u32), (0..5).map(|column| view.number(column, slot)).collect())
    }).collect();
    out.sort_unstable();
    out
}

fn known(values: [i64; 5]) -> Vec<Option<i64>> {
    values.map(Some).to_vec()
}

#[test]
fn each_viewer_holds_the_rules_operands_where_the_currents_concern_it() {
    let mut store = store();
    let mut writers = writers();
    let every = NavalRegionControlRow::EVERY_VIEWER;
    for (at, &(hour, currents, war)) in HOURS.iter().enumerate() {
        feed(&mut store, hour, currents, war);
        writers.hour(1, hour, at == 0, &mut store).unwrap();
        let view = store.view(TableId::NavalRegionControl.index());
        let withheld = match view.status() {
            Status::Unavailable { reason } => Some(String::from_utf8_lossy(view.string(reason).unwrap_or_default()).into_owned()),
            _ => None,
        };
        assert_eq!(withheld.is_some(), hour == 24, "hour {hour}: {withheld:?}");
        if hour == 24 {
            continue;
        }
        let (second, enemy_of_1, enemy_of_3) = match hour {
            0 => (5, 7, 10),
            1 => (6, 7, 10),
            _ => (6, 0, 0),
        };
        let mut expected = vec![
            ((1, 7), known([10 + second, enemy_of_1, 10 + second, 0, 10])),
            ((2, 7), known([10 + second, 0, 10 + second, 0, second])),
            ((3, 7), known([7, enemy_of_3, 7, 0, 7])),
        ];
        if hour >= 3 {
            expected.push(((every, 9), vec![None; 5]));
        }
        assert_eq!(table(&store), expected, "hour {hour}");
    }
    let view = store.view(TableId::NavalRegionControl.index());
    let slot = view.slot(u128::from(every) << 32 | 9).unwrap();
    for column in [5, 6] {
        let text = view.text(column, slot).and_then(|id| view.string(id)).unwrap();
        assert_eq!(text, b"naval dominance raw country link 105 is missing");
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (at, &(hour, currents, war)) in HOURS.iter().enumerate() {
            feed(&mut store, hour, currents, war);
            writers.hour(1, hour, at == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::NavalRegionControl));
            if resume == Some(at) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(NavalControl::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in 0..HOURS.len() - 1 {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {}", HOURS[resume].0);
    }
}

fn feed_reserved(store: &mut Store, hour: i32, naming: Status) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    put(store, hour, TableId::NavalSupremacy, &[NavalSupremacyRow { region: 7, country: 101, current: 10, ..Default::default() }, NavalSupremacyRow { region: 7, country: 102, current: 5, ..Default::default() }], Status::Complete);
    let ids = [101, 102];
    put(store, hour, TableId::Countries, &ids.map(|id| CountryRow { id, canonical_slot: id - 100, ..Default::default() }), Status::Complete);
    put(store, hour, TableId::CountryLinks, &[CountryLinkRow { id: 100, canonical_slot: 0 }, CountryLinkRow { id: 101, canonical_slot: 1 }, CountryLinkRow { id: 102, canonical_slot: 2 }], Status::Complete);
    put(store, hour, TableId::FactionMembers, &[FactionMemberRow { country: 101, faction: 9, ordinal: 0 }, FactionMemberRow { country: 102, faction: 9, ordinal: 1 }], Status::Complete);
    put::<SubjectRow>(store, hour, TableId::Subjects, &[], Status::Complete);
    put::<WarRelationRow>(store, hour, TableId::WarRelations, &[], Status::Complete);
    put::<replay_data::rows::RelationTupleRow>(store, hour, TableId::RelationTuples, &[], Status::Complete);
    put::<AirCountryRelationRow>(store, hour, TableId::AirCountryRelations, &[], Status::Complete);
    let named = [ReservedCountryNamingRow { faction: Recorded::Value(Some(9)), ..Default::default() }];
    put(store, hour, TableId::ReservedCountryNaming, &named, naming);
    store.settle();
}

#[test]
fn a_reserved_input_refused_or_back_without_a_row_moving_recomputes_the_regions() {
    let mut store = store();
    let mut writers = writers();
    let refused = Status::Unavailable { reason: 0 };
    let mut seen = Vec::new();
    for (at, (hour, naming)) in [(0, Status::Complete), (1, refused), (2, Status::Complete)].into_iter().enumerate() {
        feed_reserved(&mut store, hour, naming);
        writers.hour(1, hour, at == 0, &mut store).unwrap();
        assert_eq!(store.view(TableId::NavalRegionControl.index()).status(), Status::Complete, "hour {hour}");
        let reserved = table(&store).into_iter().find(|(key, _)| *key == (0, 7)).map(|(_, values)| values[..2].to_vec());
        seen.push((hour, reserved));
    }
    assert_eq!(seen, [(0, Some(vec![Some(15), Some(0)])), (1, Some(vec![None, None])), (2, Some(vec![Some(15), Some(0)]))]);
}
