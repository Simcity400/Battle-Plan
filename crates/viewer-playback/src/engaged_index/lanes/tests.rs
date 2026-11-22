use super::{PieceStart, fold};
use crate::{
    engaged_index::Pass,
    layers::sounds::tests::{record_runs, units},
};
use replay_columnar::{
    manifest::Status,
    state::Cell,
    store::{Store, Value},
};
use replay_data::{
    Hour,
    rows::{
        CountryLinkRow, CountryRow, FactionMemberRow, Field, LandCombatRow, LandCombatSideRow, List, LocalPlayerRow, Opaque, RowCodec, TableId, TableRows, TableSchema, tables,
    },
};
use replay_frame::frame::{DecodedFrame, Metrics, TableView};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use viewer_engagements::{engaged::Engaged, engagement_history::{Covered, History}};
use viewer_foundation::perspective::Country;

fn hour_rows(hour: Hour) -> Vec<TableRows> {
    let links = (1..=3).map(|id| CountryLinkRow { id, canonical_slot: id }).collect();
    let countries = (1..=3).map(|id| CountryRow { id, canonical_slot: id, tag: NAMES[id as usize - 1].to_vec(), ..Default::default() }).collect();
    let factions = if hour >= 8 { vec![FactionMemberRow { country: 1, faction: 9, ordinal: 0 }, FactionMemberRow { country: 3, faction: 9, ordinal: 1 }] } else { Vec::new() };
    let combat = |id: u64, attackers: Vec<u64>, defenders: Vec<u64>| LandCombatRow { id, attacker_classes: vec![0; attackers.len()], defender_classes: vec![0; defenders.len()], attackers, defenders, ..Default::default() };
    let side = |combat: u64, attacker: u32, defender: u32| LandCombatSideRow { combat, attacker_primary: vec![attacker], defender_primary: vec![defender], indicator: (combat % 2) as u8, ..Default::default() };
    let (mut combats, mut sides) = (Vec::new(), Vec::new());
    if (2..12).contains(&hour) {
        combats.push(combat(100, vec![11], vec![21]));
        sides.push(side(100, 1, 2));
    }
    if hour >= 4 {
        combats.push(combat(101, if hour >= 10 { vec![31, 32] } else { vec![31] }, vec![22]));
        sides.push(side(101, 3, 2));
    }
    if hour >= 14 {
        combats.push(combat(102, vec![23], vec![12]));
        sides.push(side(102, 2, 1));
    }
    vec![
        TableRows::CountryLinks(links),
        TableRows::Countries(countries),
        TableRows::LocalPlayer(vec![LocalPlayerRow { country: 1, observer_country: 0 }]),
        TableRows::FactionMembers(factions),
        TableRows::LandCombats(combats),
        TableRows::LandCombatSides(sides),
    ]
}

const NAMES: [&[u8]; 3] = [b"C1", b"C2", b"C3"];

const EMPTY: [TableId; 9] = [
    TableId::Players,
    TableId::HumanControllers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatParticipants,
    TableId::NavalCombatShips,
    TableId::TaskForces,
];

fn write<T: TableSchema>(store: &mut Store, rows: &[T::Row], held: &mut BTreeMap<u128, u32>) {
    let mut changes = store.table(T::ID.index()).unwrap();
    let keys: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (&key, &slot) in held.iter() {
        if !keys.contains(&key) {
            changes.delete(slot).unwrap();
        }
    }
    held.retain(|key, _| keys.contains(key));
    for row in rows {
        let key = row.key();
        let slot = match held.get(&key) {
            Some(&slot) => slot,
            None => {
                let slot = (0..).find(|slot| !held.values().any(|held| held == slot)).unwrap();
                changes.insert_at(slot, key).unwrap();
                held.insert(key, slot);
                slot
            }
        };
        row.visit_wire(&mut |column, field| {
            let mut bytes = Vec::new();
            let cell = match field {
                Field::U8(value) => Cell::U8(value),
                Field::I8(value) => Cell::I8(value),
                Field::U16(value) => Cell::U16(value),
                Field::I16(value) => Cell::I16(value),
                Field::U32(value) => Cell::U32(value),
                Field::I32(value) => Cell::I32(value),
                Field::U64(value) => Cell::U64(value),
                Field::I64(value) => Cell::I64(value),
                Field::Bool(value) => Cell::Bool(value),
                Field::OptionalI32(value) => Cell::OptionalI32(value),
                Field::OptionalU32(value) => Cell::OptionalU32(value),
                Field::OptionalI64(value) => Cell::OptionalI64(value),
                Field::OptionalU64(value) => Cell::OptionalU64(value),
                Field::Bytes(text) => Cell::Bytes(NAMES.iter().position(|name| *name == text).expect("a named string") as u32),
                Field::U32List(List::Native(items)) => Cell::U32List(items),
                Field::U64List(List::Native(items)) => Cell::U64List(items),
                Field::Opaque(Opaque::Native(value)) => {
                    value.encode_value(&mut bytes)?;
                    Cell::Opaque(&bytes)
                }
                Field::NotRecorded => return changes.put_value(slot, column, Value::Unrecorded),
                _ => panic!("no cell for column {column}"),
            };
            changes.write(slot, column, cell)
        })
        .unwrap();
    }
}

fn written(at: Hour, store: &mut Store, held: &mut HashMap<TableId, BTreeMap<u128, u32>>) {
    for table in EMPTY {
        store.status(table.index(), Status::Complete, true, Some(at), true).unwrap();
    }
    for rows in hour_rows(at) {
        let table = rows.table();
        store.status(table.index(), Status::Complete, true, Some(at), true).unwrap();
        let held = held.entry(table).or_default();
        match rows {
            TableRows::CountryLinks(rows) => write::<tables::CountryLinks>(store, &rows, held),
            TableRows::Countries(rows) => write::<tables::Countries>(store, &rows, held),
            TableRows::LocalPlayer(rows) => write::<tables::LocalPlayer>(store, &rows, held),
            TableRows::FactionMembers(rows) => write::<tables::FactionMembers>(store, &rows, held),
            TableRows::LandCombats(rows) => write::<tables::LandCombats>(store, &rows, held),
            TableRows::LandCombatSides(rows) => write::<tables::LandCombatSides>(store, &rows, held),
            _ => unreachable!("the sequence writes no other table"),
        }
    }
}

fn frame(branch: u64, hour: Hour) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch,
        hour,
        tables: std::array::from_fn(|_| TableView::Unavailable { reason: "not read".into() }.into()),
        metrics: Metrics::default(),
    };
    for table in EMPTY {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    for rows in hour_rows(hour) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
    }
    frame
}

const RUNS: [(Hour, u32); 2] = [(0, 20), (30, 12)];

fn covered(history: &History) -> Vec<(u64, String)> {
    let branches = history.branches.lock().unwrap();
    branches.iter().map(|(&branch, covered)| (branch, describe(covered))).collect()
}

fn describe(covered: &Covered) -> String {
    let [perspective, recording] = covered.folds();
    format!("{}..{} folded {perspective:?} recording {recording:?} viewers {:?} players {:?}", covered.first, covered.last, covered.viewers, covered.players)
}

fn by_frames(country: Country) -> Arc<History> {
    let shared = Arc::new(History::default());
    let mut pass = Pass::new(Arc::clone(&shared), country);
    for (at, &(first, hours)) in RUNS.iter().enumerate() {
        for hour in first..first + hours as Hour {
            pass.fold_hour(3, hour, Some(&frame(3, hour)), None, at > 0 && hour == first);
        }
    }
    shared
}

fn by_lanes(country: Country) -> Arc<History> {
    let mut held = HashMap::new();
    let recording = record_runs(&RUNS, &NAMES, |at, store| {
        if RUNS.iter().any(|&(first, _)| first == at) {
            held.clear();
        }
        written(at, store, &mut held)
    });
    let units = units(recording, "engaged");
    let shared = Arc::new(History::default());
    let started = |at: usize| PieceStart { branch: 3, load: false, restart: at > 0 };
    fold(&units.pages, &units.timeline, &started, None, country, Arc::clone(&shared), None, |_, _, _| {}).unwrap();
    shared
}

#[test]
fn the_fold_fed_hour_by_hour_from_lanes_equals_the_fold_fed_from_frames() {
    let (lanes, frames) = (by_lanes(Country::Recorded), by_frames(Country::Recorded));
    assert_eq!(covered(&lanes), covered(&frames));
    let lookup = |history: &History, hour: Hour, combat: u64| history.lookup(0, 3, hour, 1, &[combat]).map(|flags| flags[&combat]);
    assert_eq!(lookup(&lanes, 3, 100), Ok(Engaged::Yes), "a combat starting after an hour of whole tables rebuilds at once");
    assert_eq!(lookup(&lanes, 5, 101), Ok(Engaged::No), "a combat started under complete tables rebuilds");
    assert_eq!(lookup(&lanes, 9, 101), Ok(Engaged::No), "a relation change alone refreshes nothing");
    assert_eq!(lookup(&lanes, 10, 101), Ok(Engaged::Yes), "a unit joining refreshes the side");
    assert_eq!(lookup(&lanes, 15, 102), Ok(Engaged::Yes), "a new combat in a reused slot is its own");
    assert_eq!(lookup(&lanes, 31, 101), Ok(Engaged::Unknown), "across the gap every side is unknown until it rebuilds");
}

#[test]
fn a_perspective_change_folds_again_for_the_new_viewer_as_today() {
    let (lanes, frames) = (by_lanes(Country::Tag("C3".into())), by_frames(Country::Tag("C3".into())));
    assert_eq!(covered(&lanes), covered(&frames));
    let recorded = by_lanes(Country::Recorded);
    assert_ne!(covered(&lanes), covered(&recorded), "another viewer's flags are its own fold");
}

#[test]
fn each_timeline_of_a_recording_loaded_back_in_time_folds_only_its_own_pieces() {
    use crate::playhead_host::units::{
        of,
        tests::{reloaded, segment_of, tag},
    };
    let base = 4_000;
    let replay = reloaded(base);
    for (branch, last) in [(2, base + 8), (1, base + 5)] {
        let units = of(&replay, branch).unwrap();
        let (shared, mut read) = (Arc::new(History::default()), Vec::new());
        let started = |_: usize| PieceStart { branch, load: false, restart: false };
        fold(&units.pages, &units.timeline, &started, Some(&replay.campaign_id), Country::Recorded, Arc::clone(&shared), None, |hour, store, _| {
            read.push((hour, store.table::<tables::Countries>().and_then(|rows| rows.first()).map(|row| row.tag.clone())));
        })
        .unwrap();
        let wanted: Vec<(Hour, Option<Vec<u8>>)> = (base..=last).map(|hour| (hour, Some(tag(segment_of(base, branch, hour))))).collect();
        assert_eq!(read, wanted, "the hours timeline {branch}'s fold reads, and the segment each is read from");
        assert_eq!(covered(&shared).iter().map(|(held, _)| *held).collect::<Vec<_>>(), [branch], "timeline {branch} folds into its own branch");
    }
    let index = |branch| super::shared(&replay, &of(&replay, branch).unwrap(), branch, &Country::Recorded).unwrap();
    let (forked, again, parent) = (index(2), index(2), index(1));
    assert!(Arc::ptr_eq(&forked, &again), "a second ask for timeline 2's index shares its fold");
    assert!(!Arc::ptr_eq(&forked, &parent), "timeline 1 has an index of its own");
}

#[test]
fn naval_combat_sides_of_an_earlier_format_decode_at_its_own_format() {
    use crate::lane_rows::{Kept, Table};
    use crate::layers::sounds::tests::record_runs_at;
    use playhead::pages::Pages;
    use replay_data::{
        rows::{NavalCombatDetailsRow, NavalCombatSideDetails, NavalCommanderInputs, Recorded},
        value::Value as _,
    };
    const FORMAT: u16 = 97;
    let commander = NavalCommanderInputs { commander: 0x1234_5678_9abc_def0, name: vec![0xff, 0, 8], ..Default::default() };
    let side = |engaged: u8| NavalCombatSideDetails { commander_inputs: Some(commander.clone()), engaged, units: vec![4, u64::MAX, 2], ..Default::default() };
    let row_at = |engaged: u8| NavalCombatDetailsRow { combat: 7, sides: [side(engaged), side(engaged)], ..Default::default() };
    let mut needle = Vec::new();
    commander.encode(&mut needle).unwrap();
    let old_commander = &needle[..needle.len() - 4];
    let sides_column = (0..64).find(|&column| TableId::NavalCombatDetails.field_name(column) == Some("sides")).unwrap();
    let old_sides = |row: &NavalCombatDetailsRow| {
        let mut current = Vec::new();
        row.visit_wire(&mut |column, field| {
            if column == sides_column
                && let Field::Opaque(Opaque::Native(value)) = field
            {
                value.encode_value(&mut current)?;
            }
            Ok(())
        })
        .unwrap();
        let mut old = Vec::new();
        let mut rest = &current[..];
        while let Some(at) = rest.windows(needle.len()).position(|window| window == needle) {
            old.extend_from_slice(&rest[..at]);
            old.extend_from_slice(old_commander);
            rest = &rest[at + needle.len()..];
        }
        old.extend_from_slice(rest);
        assert_eq!(old.len(), current.len() - 8, "both sides' commanders lose their timed traits");
        old
    };
    let recording = record_runs_at(FORMAT, &[(0, 4)], &[], |hour, store| {
        store.status(TableId::NavalCombatDetails.index(), Status::Complete, true, Some(hour), true).unwrap();
        if hour != 0 && hour != 3 {
            return;
        }
        let row = row_at(if hour == 0 { 1 } else { 0 });
        let mut changes = store.table(TableId::NavalCombatDetails.index()).unwrap();
        if hour == 0 {
            changes.insert_at(0, row.key()).unwrap();
        }
        let old = old_sides(&row);
        changes.write(0, sides_column, Cell::Opaque(&old)).unwrap();
    });
    assert_eq!(recording.format(), FORMAT);
    let pages: Arc<dyn Pages> = recording;
    let mut table = Table::<tables::NavalCombatDetails>::open(&pages, 0, Some(&["sides"]), "The test").unwrap();
    table.seek(2).unwrap();
    let check = |rows: &[NavalCombatDetailsRow], engaged: u8| {
        assert_eq!(rows.len(), 1);
        for side in &rows[0].sides {
            let inputs = side.commander_inputs.as_ref().unwrap();
            assert_eq!(inputs.timed_traits, Recorded::NotRecorded);
            assert_eq!((inputs.commander, &inputs.name), (commander.commander, &commander.name));
            assert_eq!((side.engaged, &side.units[..]), (engaged, &[4, u64::MAX, 2][..]));
        }
    };
    check(tables::NavalCombatDetails::borrow(table.rows()).unwrap(), 1);
    table.step(3).unwrap();
    check(tables::NavalCombatDetails::borrow(table.rows()).unwrap(), 0);
    let custom_sea_column = (0..64).find(|&column| TableId::NavalCombatDetails.field_name(column) == Some("custom_sea")).unwrap();
    let custom_sea_error_column = (0..64).find(|&column| TableId::NavalCombatDetails.field_name(column) == Some("custom_sea_error")).unwrap();
    let mut none_sea = Vec::new();
    Option::<Vec<u8>>::None.encode(&mut none_sea).unwrap();
    let conflict_recording = record_runs_at(FORMAT, &[(0, 1)], &[], |hour, store| {
        store.status(TableId::NavalCombatDetails.index(), Status::Complete, true, Some(hour), true).unwrap();
        let row = row_at(1);
        let mut changes = store.table(TableId::NavalCombatDetails.index()).unwrap();
        changes.insert_at(0, row.key()).unwrap();
        let old = old_sides(&row);
        changes.write(0, sides_column, Cell::Opaque(&old)).unwrap();
        changes.write(0, custom_sea_column, Cell::Opaque(&none_sea)).unwrap();
        changes.write(0, custom_sea_error_column, Cell::U8(0)).unwrap();
    });
    let conflict_pages: Arc<dyn Pages> = conflict_recording;
    let mut with_sea = Table::<tables::NavalCombatDetails>::open(&conflict_pages, 0, Some(&["sides", "custom_sea", "custom_sea_error"]), "The test").unwrap();
    let conflict = with_sea.seek(0).expect_err("selecting custom_sea and custom_sea_error validates their invariant");
    assert!(conflict.to_string().contains("custom sea capture status conflicts with values"), "{conflict}");
    let odd_recording = record_runs_at(FORMAT, &[(0, 2)], &[], |hour, store| {
        store.status(TableId::NavalCombatDetails.index(), Status::Complete, true, Some(hour), true).unwrap();
        let row = row_at(1);
        let mut changes = store.table(TableId::NavalCombatDetails.index()).unwrap();
        if hour == 0 {
            changes.insert_at(0, row.key()).unwrap();
            let old = old_sides(&row);
            changes.write(0, sides_column, Cell::Opaque(&old)).unwrap();
            changes.write(0, custom_sea_column, Cell::Opaque(&none_sea)).unwrap();
            changes.write(0, custom_sea_error_column, Cell::U8(1)).unwrap();
        } else {
            let mut odd_sea = Vec::new();
            Some(vec![7u8]).encode(&mut odd_sea).unwrap();
            changes.write(0, custom_sea_column, Cell::Opaque(&odd_sea)).unwrap();
            changes.write(0, custom_sea_error_column, Cell::U8(0)).unwrap();
        }
    });
    let odd_pages: Arc<dyn Pages> = odd_recording;
    let mut odd_table = Table::<tables::NavalCombatDetails>::open(&odd_pages, 0, Some(&["sides", "custom_sea", "custom_sea_error"]), "The test").unwrap();
    odd_table.seek(0).unwrap();
    let odd = odd_table.step(1).expect_err("stepping into odd-length custom_sea fails validation on partial projection");
    assert!(odd.to_string().contains("odd custom sea i16 bytes"), "{odd}");
}
