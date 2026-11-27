use super::{PieceStart, fold};
use crate::engaged_index::Pass;
use viewer_layer_core::test_support::sound_recording::{record_runs, units};
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

const NAMES: [&[u8]; 4] = [b"C1", b"C2", b"C3", b""];

const EMPTY: [TableId; 10] = [
    TableId::Players,
    TableId::HumanControllers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatParticipants,
    TableId::NavalCombatShips,
    TableId::TaskForces,
    TableId::CombatEngagementRefreshes,
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
    use viewer_layer_core::{playhead_host::units::of, test_support::reloaded_playback::{reloaded, segment_of, tag}};
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
    use viewer_layer_core::lane_rows::{Kept, Table};
    use viewer_layer_core::test_support::sound_recording::record_runs_at;
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

#[test]
fn lane_fold_invalidates_cached_inputs_when_refresh_witness_changes() {
    use replay_data::rows::CombatEngagementRefreshRow;
    let witness_rows = |hour: Hour| -> Vec<CombatEngagementRefreshRow> {
        if hour == 9 {
            vec![CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 101,
                side: 0,
                viewer: 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }]
        } else {
            Vec::new()
        }
    };
    let mut held = HashMap::new();
    let recording = record_runs(&[(0, 12)], &NAMES, |at, store| {
        if at == 0 {
            held.clear();
        }
        written(at, store, &mut held);
        store
            .status(
                TableId::CombatEngagementRefreshes.index(),
                Status::Complete,
                true,
                Some(at),
                true,
            )
            .unwrap();
        let entry = held.entry(TableId::CombatEngagementRefreshes).or_default();
        write::<tables::CombatEngagementRefreshes>(store, &witness_rows(at), entry);
    });
    let units = units(recording, "engaged-witness");
    let shared = Arc::new(History::default());
    let started = |_: usize| PieceStart {
        branch: 3,
        load: false,
        restart: false,
    };
    fold(
        &units.pages,
        &units.timeline,
        &started,
        None,
        Country::Recorded,
        Arc::clone(&shared),
        None,
        |_, _, _| {},
    )
    .unwrap();
    let lookup = |hour: Hour| {
        shared
            .lookup(0, 3, hour, 1, &[101])
            .map(|flags| flags[&101])
    };
    assert_eq!(lookup(8), Ok(Engaged::No));
    assert_eq!(lookup(9), Ok(Engaged::Yes));
}

#[test]
fn lanes_fold_reads_format_145_recording_without_refresh_witness_table() {
    use viewer_layer_core::test_support::sound_recording::record_runs_at;
    use playhead::pages::{Pages, Run};
    use replay_columnar::{state::Kind, unit::reader::Extents};
    use viewer_engagements::engaged::Ambiguity;
    struct WithoutTable {
        inner: Arc<dyn Pages>,
        omitted: usize,
    }
    impl Pages for WithoutTable {
        fn extents(&self, run: usize) -> &dyn Extents {
            self.inner.extents(run)
        }
        fn runs(&self) -> &[Run] {
            self.inner.runs()
        }
        fn table_entry(&self, run: usize, block: usize, table: usize) -> std::io::Result<playhead::pages::EntryRef> {
            self.inner.table_entry(run, block, table)
        }
        fn kinds(&self, table: usize) -> Option<&[Kind]> {
            (table != self.omitted).then(|| self.inner.kinds(table)).flatten()
        }
        fn format(&self) -> u16 {
            self.inner.format()
        }
    }
    let mut held = HashMap::new();
    let recording = record_runs_at(145, &[(0, 12)], &NAMES, |at, store| {
        if at == 0 {
            held.clear();
        }
        for table in EMPTY {
            if table == TableId::CombatEngagementRefreshes {
                continue;
            }
            store.status(table.index(), Status::Complete, true, Some(at), true).unwrap();
        }
        for rows in hour_rows(at) {
            let table = rows.table();
            store.status(table.index(), Status::Complete, true, Some(at), true).unwrap();
            let entry = held.entry(table).or_default();
            match rows {
                TableRows::CountryLinks(rows) => write::<tables::CountryLinks>(store, &rows, entry),
                TableRows::Countries(rows) => write::<tables::Countries>(store, &rows, entry),
                TableRows::LocalPlayer(rows) => write::<tables::LocalPlayer>(store, &rows, entry),
                TableRows::FactionMembers(rows) => write::<tables::FactionMembers>(store, &rows, entry),
                TableRows::LandCombats(rows) => write::<tables::LandCombats>(store, &rows, entry),
                TableRows::LandCombatSides(rows) => write::<tables::LandCombatSides>(store, &rows, entry),
                _ => unreachable!("the sequence writes no other table"),
            }
        }
    });
    assert_eq!(recording.format(), 145);
    let units = units(recording, "engaged-f145");
    let pages: Arc<dyn Pages> = Arc::new(WithoutTable {
        inner: Arc::clone(&units.pages),
        omitted: TableId::CombatEngagementRefreshes.index(),
    });
    assert!(pages.kinds(TableId::CombatEngagementRefreshes.index()).is_none());
    let shared = Arc::new(History::default());
    let started = |_: usize| PieceStart {
        branch: 3,
        load: false,
        restart: false,
    };
    fold(
        &pages,
        &units.timeline,
        &started,
        None,
        Country::Recorded,
        Arc::clone(&shared),
        None,
        |_, _, _| {},
    )
    .unwrap();
    let lookup = |hour: Hour, combat: u64| {
        shared
            .lookup(0, 3, hour, 1, &[combat])
            .map(|flags| flags[&combat])
    };
    assert_eq!(lookup(3, 100), Ok(Engaged::Yes));
    assert_eq!(lookup(5, 101), Ok(Engaged::No));
    assert_eq!(lookup(8, 101), Ok(Engaged::Ambiguous(Ambiguity::Order)));
    assert_eq!(lookup(9, 101), Ok(Engaged::Ambiguous(Ambiguity::Scope)));
    assert_eq!(lookup(10, 101), Ok(Engaged::Yes));
}

#[test]
fn lanes_fold_evaluates_zero_viewer_sweep_from_reserved_country_operands() {
    use replay_data::rows::{
        CombatEngagementRefreshRow, CountryCounterInputRow, Recorded, ReservedCountryNamingRow,
        ReverseZeroVisibilityInputs, ZeroSubjectInputs,
    };
    let run = |reserved_faction: Option<u64>, include_counter_inputs: bool| {
        let mut held = HashMap::new();
        let recording = record_runs(&[(0, 12)], &NAMES, |at, store| {
            if at == 0 {
                held.clear();
            }
            written(at, store, &mut held);
            if let Some(faction) = reserved_faction {
                store
                    .status(
                        TableId::ReservedCountryNaming.index(),
                        Status::Complete,
                        true,
                        Some(at),
                        true,
                    )
                    .unwrap();
                let entry = held.entry(TableId::ReservedCountryNaming).or_default();
                write::<tables::ReservedCountryNaming>(
                    store,
                    &[ReservedCountryNamingRow {
                        faction: Recorded::Value(Some(faction)),
                        ..Default::default()
                    }],
                    entry,
                );
            }
            if include_counter_inputs {
                store
                    .status(
                        TableId::CountryCounterInputs.index(),
                        Status::Complete,
                        true,
                        Some(at),
                        true,
                    )
                    .unwrap();
                let entry = held.entry(TableId::CountryCounterInputs).or_default();
                let rows: Vec<CountryCounterInputRow> = (1..=3)
                    .map(|country| CountryCounterInputRow {
                        country,
                        forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                            war: Some([0, 0]),
                            military_access: Some(0),
                            target: Some(0),
                            subject: Some(ZeroSubjectInputs {
                                relation_type: None,
                                parties: None,
                            }),
                        })),
                        ..Default::default()
                    })
                    .collect();
                write::<tables::CountryCounterInputs>(store, &rows, entry);
            }
            store
                .status(
                    TableId::CombatEngagementRefreshes.index(),
                    Status::Complete,
                    true,
                    Some(at),
                    true,
                )
                .unwrap();
            let witness_rows = if at == 9 {
                vec![CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                    target: 0,
                    side: 0,
                    viewer: 0,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                }]
            } else {
                Vec::new()
            };
            let entry = held.entry(TableId::CombatEngagementRefreshes).or_default();
            write::<tables::CombatEngagementRefreshes>(store, &witness_rows, entry);
        });
        let units = units(recording, "engaged-zero-view");
        let shared = Arc::new(History::default());
        let started = |_: usize| PieceStart {
            branch: 3,
            load: false,
            restart: false,
        };
        fold(
            &units.pages,
            &units.timeline,
            &started,
            None,
            Country::Recorded,
            Arc::clone(&shared),
            None,
            |_, _, _| {},
        )
        .unwrap();
        let lookup = |hour: Hour| shared.lookup(0, 3, hour, 1, &[101]).map(|flags| flags[&101]);
        (lookup(8), lookup(9))
    };
    assert_eq!(run(Some(9), true), (Ok(Engaged::No), Ok(Engaged::Yes)));
    assert_eq!(run(Some(77), true), (Ok(Engaged::No), Ok(Engaged::No)));
    assert_eq!(run(Some(9), false), (Ok(Engaged::No), Ok(Engaged::Unknown)));
    assert_eq!(run(None, true), (Ok(Engaged::No), Ok(Engaged::Unknown)));
}

#[test]
fn lanes_fold_preserves_recording_history_across_unselected_observer_hour() {
    use replay_data::rows::{
        CombatEngagementRefreshRow, CountryCounterInputRow, Recorded, ReservedCountryNamingRow,
        ReverseZeroVisibilityInputs, ZeroSubjectInputs,
    };
    let mut held = HashMap::new();
    let recording = record_runs(&[(0, 12)], &NAMES, |at, store| {
        if at == 0 {
            held.clear();
        }
        for table in EMPTY {
            store
                .status(table.index(), Status::Complete, true, Some(at), true)
                .unwrap();
        }
        for rows in hour_rows(at) {
            let table = rows.table();
            store
                .status(table.index(), Status::Complete, true, Some(at), true)
                .unwrap();
            let entry = held.entry(table).or_default();
            match rows {
                TableRows::CountryLinks(rows) => write::<tables::CountryLinks>(store, &rows, entry),
                TableRows::Countries(rows) => write::<tables::Countries>(store, &rows, entry),
                TableRows::LocalPlayer(_) => {
                    let observer = if at == 9 { 0 } else { 1 };
                    write::<tables::LocalPlayer>(
                        store,
                        &[LocalPlayerRow {
                            country: 0,
                            observer_country: observer,
                        }],
                        entry,
                    );
                }
                TableRows::FactionMembers(_) => {
                    write::<tables::FactionMembers>(
                        store,
                        &[FactionMemberRow {
                            country: 3,
                            faction: 9,
                            ordinal: 0,
                        }],
                        entry,
                    );
                }
                TableRows::LandCombats(mut rows) => {
                    for row in &mut rows {
                        if row.id == 101 {
                            row.attackers = vec![31];
                            row.attacker_classes = vec![0];
                        }
                    }
                    write::<tables::LandCombats>(store, &rows, entry);
                }
                TableRows::LandCombatSides(rows) => {
                    write::<tables::LandCombatSides>(store, &rows, entry)
                }
                _ => unreachable!("the sequence writes no other table"),
            }
        }
        store
            .status(
                TableId::ReservedCountryNaming.index(),
                Status::Complete,
                true,
                Some(at),
                true,
            )
            .unwrap();
        write::<tables::ReservedCountryNaming>(
            store,
            &[ReservedCountryNamingRow {
                faction: Recorded::Value(Some(9)),
                ..Default::default()
            }],
            held.entry(TableId::ReservedCountryNaming).or_default(),
        );
        store
            .status(
                TableId::CountryCounterInputs.index(),
                Status::Complete,
                true,
                Some(at),
                true,
            )
            .unwrap();
        let counter_rows: Vec<CountryCounterInputRow> = (1..=3)
            .map(|country| CountryCounterInputRow {
                country,
                forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                    war: Some([0, 0]),
                    military_access: Some(0),
                    target: Some(0),
                    subject: Some(ZeroSubjectInputs {
                        relation_type: None,
                        parties: None,
                    }),
                })),
                ..Default::default()
            })
            .collect();
        write::<tables::CountryCounterInputs>(
            store,
            &counter_rows,
            held.entry(TableId::CountryCounterInputs).or_default(),
        );
        let witness_rows = if at == 9 {
            vec![CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }]
        } else {
            Vec::new()
        };
        write::<tables::CombatEngagementRefreshes>(
            store,
            &witness_rows,
            held.entry(TableId::CombatEngagementRefreshes).or_default(),
        );
    });
    let units = units(recording, "engaged-unselected-observer");
    let shared = Arc::new(History::default());
    let started = |_: usize| PieceStart {
        branch: 3,
        load: true,
        restart: false,
    };
    fold(
        &units.pages,
        &units.timeline,
        &started,
        None,
        Country::Recorded,
        Arc::clone(&shared),
        None,
        |_, _, _| {},
    )
    .unwrap();
    let lookup = |hour: Hour| shared.lookup(0, 3, hour, 1, &[101]).map(|flags| flags[&101]);
    assert_eq!(lookup(8), Ok(Engaged::No));
    assert_eq!(lookup(10), Ok(Engaged::Yes));
}

#[test]
fn lanes_fold_preserves_raw_tags_for_zero_and_reserved_alias_viewers() {
    use replay_data::rows::{
        CombatEngagementRefreshRow, PacketHour, Recorded, ReservedCountryNamingRow,
        ReservedReverseVisibilityInputs, ReverseZeroVisibilityInputs,
    };
    let run = |link_alias: bool,
               primary: Vec<u32>,
               secondary: Vec<u32>,
               sweep_target: u64,
               reverse_war: bool| {
        let mut held = HashMap::new();
        let recording = record_runs(&[(0, 12)], &NAMES, |at, store| {
            if at == 0 {
                held.clear();
            }
            for table in EMPTY {
                store
                    .status(table.index(), Status::Complete, true, Some(at), true)
                    .unwrap();
            }
            for rows in hour_rows(at) {
                let table = rows.table();
                store
                    .status(table.index(), Status::Complete, true, Some(at), true)
                    .unwrap();
                let entry = held.entry(table).or_default();
                match rows {
                    TableRows::CountryLinks(mut rows) => {
                        if link_alias {
                            rows.push(CountryLinkRow {
                                id: 777,
                                canonical_slot: 0,
                            });
                        }
                        write::<tables::CountryLinks>(store, &rows, entry);
                    }
                    TableRows::Countries(rows) => write::<tables::Countries>(store, &rows, entry),
                    TableRows::LocalPlayer(rows) => {
                        write::<tables::LocalPlayer>(store, &rows, entry)
                    }
                    TableRows::FactionMembers(rows) => {
                        write::<tables::FactionMembers>(store, &rows, entry)
                    }
                    TableRows::LandCombats(rows) => {
                        write::<tables::LandCombats>(store, &rows, entry)
                    }
                    TableRows::LandCombatSides(mut rows) => {
                        for row in &mut rows {
                            if row.combat == 101 {
                                row.attacker_primary = primary.clone();
                                row.attacker_secondary = secondary.clone();
                                row.defender_primary = Vec::new();
                                row.defender_secondary = Vec::new();
                            }
                        }
                        write::<tables::LandCombatSides>(store, &rows, entry);
                    }
                    _ => unreachable!("the sequence writes no other table"),
                }
            }
            if reverse_war {
                store
                    .status(
                        TableId::ReservedCountryNaming.index(),
                        Status::Complete,
                        true,
                        Some(at),
                        true,
                    )
                    .unwrap();
                write::<tables::ReservedCountryNaming>(
                    store,
                    &[ReservedCountryNamingRow {
                        faction: Recorded::Value(Some(0)),
                        reverse_visibility: Recorded::Value(Some(
                            ReservedReverseVisibilityInputs {
                                sample_hour: PacketHour::Host,
                                cells: vec![Some(ReverseZeroVisibilityInputs {
                                    war: Some([1, 0]),
                                    ..Default::default()
                                })],
                            },
                        )),
                        ..Default::default()
                    }],
                    held.entry(TableId::ReservedCountryNaming).or_default(),
                );
            }
            let witness_rows = if at == 9 {
                vec![CombatEngagementRefreshRow {
                    kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                    target: sweep_target,
                    side: 0,
                    viewer: 0,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                }]
            } else {
                Vec::new()
            };
            write::<tables::CombatEngagementRefreshes>(
                store,
                &witness_rows,
                held.entry(TableId::CombatEngagementRefreshes).or_default(),
            );
        });
        let units = units(recording, "engaged-raw-tags");
        let shared = Arc::new(History::default());
        let started = |_: usize| PieceStart {
            branch: 3,
            load: true,
            restart: false,
        };
        fold(
            &units.pages,
            &units.timeline,
            &started,
            None,
            Country::Recorded,
            Arc::clone(&shared),
            None,
            |_, _, _| {},
        )
        .unwrap();
        shared.lookup(0, 3, 9, 1, &[101]).map(|flags| flags[&101])
    };

    assert_eq!(run(true, Vec::new(), vec![777], 0, false), Ok(Engaged::No));
    assert_eq!(run(false, Vec::new(), vec![777], 0, false), Ok(Engaged::No));
    assert_eq!(run(true, vec![777], Vec::new(), 777, true), Ok(Engaged::No));
    assert_eq!(run(true, vec![0], Vec::new(), 777, true), Ok(Engaged::No));
    assert_eq!(run(true, vec![777], Vec::new(), 777, false), Ok(Engaged::Unknown));
    assert_eq!(run(true, vec![777], Vec::new(), 0, false), Ok(Engaged::Yes));
}

#[test]
fn lanes_fold_serves_recording_track_for_selected_tag_mapped_to_slot_zero() {
    use replay_data::rows::CombatEngagementRefreshRow;
    let mut held = HashMap::new();
    let recording = record_runs(&[(0, 12)], &NAMES, |at, store| {
        if at == 0 {
            held.clear();
        }
        for table in EMPTY {
            store
                .status(table.index(), Status::Complete, true, Some(at), true)
                .unwrap();
        }
        for rows in hour_rows(at) {
            let table = rows.table();
            store
                .status(table.index(), Status::Complete, true, Some(at), true)
                .unwrap();
            let entry = held.entry(table).or_default();
            match rows {
                TableRows::CountryLinks(mut rows) => {
                    rows.push(CountryLinkRow {
                        id: 777,
                        canonical_slot: 0,
                    });
                    write::<tables::CountryLinks>(store, &rows, entry);
                }
                TableRows::Countries(rows) => write::<tables::Countries>(store, &rows, entry),
                TableRows::LocalPlayer(_) => write::<tables::LocalPlayer>(
                    store,
                    &[LocalPlayerRow {
                        country: 0,
                        observer_country: 777,
                    }],
                    entry,
                ),
                TableRows::FactionMembers(_) => {
                    write::<tables::FactionMembers>(store, &[], entry)
                }
                TableRows::LandCombats(rows) => write::<tables::LandCombats>(store, &rows, entry),
                TableRows::LandCombatSides(mut rows) => {
                    for row in &mut rows {
                        if row.combat == 101 {
                            row.attacker_primary = Vec::new();
                            row.attacker_secondary = vec![1];
                            row.defender_primary = Vec::new();
                            row.defender_secondary = Vec::new();
                        }
                    }
                    write::<tables::LandCombatSides>(store, &rows, entry);
                }
                _ => unreachable!("the sequence writes no other table"),
            }
        }
        let witness_rows = if at == 9 {
            vec![CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 1,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }]
        } else {
            Vec::new()
        };
        write::<tables::CombatEngagementRefreshes>(
            store,
            &witness_rows,
            held.entry(TableId::CombatEngagementRefreshes).or_default(),
        );
    });
    let units = units(recording, "engaged-slot-zero-selected");
    let shared = Arc::new(History::default());
    let started = |_: usize| PieceStart {
        branch: 3,
        load: true,
        restart: false,
    };
    fold(
        &units.pages,
        &units.timeline,
        &started,
        None,
        Country::Recorded,
        Arc::clone(&shared),
        None,
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(
        shared.lookup(0, 3, 9, 0, &[101]).map(|flags| flags[&101]),
        Ok(Engaged::Yes)
    );
    let branches = shared.branches.lock().unwrap();
    assert!(branches[&3].recording_player(9, 0));
}

#[test]
fn lanes_fold_remapped_observer_tag_keeps_unavailable_hour_ambiguous() {
    use viewer_engagements::engaged::Ambiguity;
    let mut held = HashMap::new();
    let recording = record_runs(&[(0, 3)], &NAMES, |at, store| {
        if at == 0 {
            held.clear();
        }
        for table in EMPTY {
            let status = if table == TableId::CombatEngagementRefreshes && at == 2 {
                Status::Unavailable { reason: 1 }
            } else {
                Status::Complete
            };
            store
                .status(table.index(), status, true, Some(at), true)
                .unwrap();
        }
        let links = vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 701,
                canonical_slot: if at == 0 { 1 } else { 2 },
            },
            CountryLinkRow {
                id: 202,
                canonical_slot: 2,
            },
        ];
        let countries: Vec<_> = (1..=3)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: NAMES[id as usize - 1].to_vec(),
                ..Default::default()
            })
            .collect();
        let factions = vec![
            FactionMemberRow {
                country: 1,
                faction: 9,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 3,
                faction: 9,
                ordinal: 1,
            },
        ];
        let local_player = vec![LocalPlayerRow {
            country: 0,
            observer_country: if at == 0 { 701 } else { 202 },
        }];
        let combats = vec![LandCombatRow {
            id: 101,
            attacker_classes: vec![0],
            defender_classes: vec![0],
            attackers: vec![31],
            defenders: vec![22],
            ..Default::default()
        }];
        let sides = vec![LandCombatSideRow {
            combat: 101,
            attacker_primary: vec![3],
            defender_primary: Vec::new(),
            indicator: 1,
            ..Default::default()
        }];
        for (table, write_fn) in [
            (TableId::CountryLinks, 0usize),
            (TableId::Countries, 1),
            (TableId::LocalPlayer, 2),
            (TableId::FactionMembers, 3),
            (TableId::LandCombats, 4),
            (TableId::LandCombatSides, 5),
        ] {
            store
                .status(table.index(), Status::Complete, true, Some(at), true)
                .unwrap();
            let entry = held.entry(table).or_default();
            match write_fn {
                0 => write::<tables::CountryLinks>(store, &links, entry),
                1 => write::<tables::Countries>(store, &countries, entry),
                2 => write::<tables::LocalPlayer>(store, &local_player, entry),
                3 => write::<tables::FactionMembers>(store, &factions, entry),
                4 => write::<tables::LandCombats>(store, &combats, entry),
                _ => write::<tables::LandCombatSides>(store, &sides, entry),
            }
        }
        if at < 2 {
            write::<tables::CombatEngagementRefreshes>(
                store,
                &[],
                held.entry(TableId::CombatEngagementRefreshes).or_default(),
            );
        }
    });
    let units = units(recording, "engaged-remapped-observer");
    let shared = Arc::new(History::default());
    let started = |_: usize| PieceStart {
        branch: 3,
        load: true,
        restart: false,
    };
    fold(
        &units.pages,
        &units.timeline,
        &started,
        None,
        Country::Recorded,
        Arc::clone(&shared),
        None,
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(
        shared.lookup(0, 3, 0, 1, &[101]).map(|flags| flags[&101]),
        Ok(Engaged::Yes)
    );
    assert_eq!(
        shared.lookup(0, 3, 1, 2, &[101]).map(|flags| flags[&101]),
        Ok(Engaged::Yes)
    );
    assert_eq!(
        shared.lookup(0, 3, 2, 2, &[101]).map(|flags| flags[&101]),
        Ok(Engaged::Ambiguous(Ambiguity::Scope))
    );
}
