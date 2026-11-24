use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{
    manifest::Status,
    store::{Store, Value as Held},
    unit::Discard,
};
use replay_data::rows::{CountryLinkRow, DestroyedDivisionsRow, DivisionRow, DivisionStatusRow, DivisionTemplateIconRow, LandEventCoverageRow, NameOperands, PacketHour, Recorded, TemplateDisplayName, TemplateNameValue};

const TEXTS: &[&[u8]] = &[b"not read", b"", b"infantry"];
const NOT_READ: u32 = 0;

const D1: u64 = 1 << 32 | 9;
const D2: u64 = 2 << 32 | 9;
const D3: u64 = 3 << 32 | 9;
const D4: u64 = 4 << 32 | 9;
const D5: u64 = 5 << 32 | 9;
const KILLER: u64 = 77 << 32 | 9;
const TEMPLATE: u64 = 40 << 32 | 3;

const INPUTS: [TableId; 7] = [
    TableId::Divisions,
    TableId::DivisionEquipmentDeletions,
    TableId::DivisionStatus,
    TableId::DivisionTemplateIcons,
    TableId::DestroyedDivisions,
    TableId::LandEventCoverage,
    TableId::CountryLinks,
];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(DivisionLog::default())]);
    writers.start_run();
    writers
}

fn division(id: u64, roster: u32, expeditionary: u32, province: u32, reveal_skip: u8) -> DivisionRow {
    DivisionRow { id, roster_owner: roster, logical_owner: roster, expeditionary_owner: expeditionary, province, template: TEMPLATE, hit_points: 90_000, organisation: 50_000, orders_group: 0, reveal_skip, theatre: Some(0).into() }
}

fn status(id: u64, name: &[u8]) -> DivisionStatusRow {
    DivisionStatusRow {
        division: id,
        name: Recorded::Value(Some(name.to_vec())),
        naming: Recorded::Value(Some(NameOperands { kind: 1, order: id as i32 >> 1, ordered: 1, programmatic: 0, custom: Vec::new(), group: b"INF".to_vec() })),
        ..Default::default()
    }
}

fn witness(division: u64, country: u32, disposition: Disposition, cause: Option<Cause>) -> DivisionEquipmentDeletionRow {
    DivisionEquipmentDeletionRow { division_handle: division, country, expeditionary_owner: 0, disposition, cause }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    run_logging(store, writers, hour, 2);
}

fn run_logging(store: &mut Store, writers: &mut Writers, hour: i32, logged: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for table in INPUTS {
        let status = if hour == 4 && table == TableId::Divisions { Status::Unavailable { reason: NOT_READ } } else { Status::Complete };
        store.status(table.index(), status, true, Some(hour), true).unwrap();
    }
    match hour {
        0 => {
            for (id, raw) in [(1, 10), (2, 20)] {
                testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: raw }, TEXTS).unwrap();
            }
            for row in [division(D1, 1, 0, 100, 0), division(D2, 2, 0, 200, 0), division(D3, 2, 0, 250, 1)] {
                testing::put(store, TableId::Divisions, &row, TEXTS).unwrap();
            }
            for (id, name) in [(D1, &b"1st"[..]), (D2, b"2nd")] {
                testing::put(store, TableId::DivisionStatus, &status(id, name), TEXTS).unwrap();
            }
            let display_name = TemplateDisplayName { sample_hour: PacketHour::Host, value: TemplateNameValue::Changed { literal: b"Infantry Division".to_vec(), fallback_key: None } };
            let icon = DivisionTemplateIconRow { template: TEMPLATE, priority_unit: 7, priority_unit_name: b"infantry".to_vec(), custom_icon: 3, country: 2, display_name: Recorded::Value(Some(display_name)), ..Default::default() };
            testing::put(store, TableId::DivisionTemplateIcons, &icon, TEXTS).unwrap();
        }
        1 => {
            testing::put(store, TableId::Divisions, &division(D4, 2, 1, 300, 0), TEXTS).unwrap();
            testing::put(store, TableId::Divisions, &division(D3, 2, 0, 250, 0), TEXTS).unwrap();
            testing::put(store, TableId::Divisions, &division(D1, 1, 0, 101, 0), TEXTS).unwrap();
        }
        2 => {
            testing::remove(store, TableId::Divisions, u128::from(D1)).unwrap();
            testing::remove(store, TableId::DivisionStatus, u128::from(D1)).unwrap();
            testing::put(store, TableId::DivisionEquipmentDeletions, &witness(D1, 1, Disposition::Loss, Some(Cause::DivisionDestroyed)), TEXTS).unwrap();
        }
        3 => {
            testing::forget(store, TableId::DestroyedDivisions, 0).unwrap();
            testing::remove(store, TableId::Divisions, u128::from(D2)).unwrap();
            testing::remove(store, TableId::Divisions, u128::from(D3)).unwrap();
            testing::put(store, TableId::DivisionEquipmentDeletions, &witness(D2, 2, Disposition::HqTransferred, None), TEXTS).unwrap();
        }
        4 => {
            testing::put(store, TableId::DivisionEquipmentDeletions, &witness(D4, 2, Disposition::Loss, Some(Cause::DivisionDisbanded)), TEXTS).unwrap();
        }
        _ => {
            testing::remove(store, TableId::Divisions, u128::from(D4)).unwrap();
            testing::put(store, TableId::Divisions, &division(D5, 2, 0, 400, 0), TEXTS).unwrap();
        }
    }
    let span = LandEventCoverageRow { id: 1, epoch: 1, from_hour: hour - 1, to_hour: hour, complete: true, lost_events: 0 };
    testing::put(store, TableId::LandEventCoverage, &span, TEXTS).unwrap();
    if hour == logged {
        let death =DestroyedDivisionsRow { id: 1, division: D1, owner: 1, killer_country: 2, killer_division: Some(KILLER), province: 555, hour, ..Default::default() };
        testing::put(store, TableId::DestroyedDivisions, &death, TEXTS).unwrap();
    }
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn number(store: &Store, column: usize, key: u128) -> Option<i64> {
    let view = store.view(TableId::DivisionEvents.index());
    view.number(column, view.slot(key)?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    kind: u8,
    division: u64,
    slot: Option<i64>,
    native_hour: Option<i64>,
    exact: Option<i64>,
    after: Option<i64>,
    province: Option<i64>,
    place_hour: Option<i64>,
    template_hour: Option<i64>,
    killer_slot: Option<i64>,
}

fn events(store: &Store) -> Vec<Event> {
    testing::rows(store, TableId::DivisionEvents)
        .into_iter()
        .map(|(key, _)| {
            let cell = |column| number(store, column, key);
            Event {
                kind: (key >> 96) as u8,
                division: key as u64,
                slot: cell(3),
                native_hour: cell(4),
                exact: cell(5),
                after: cell(6),
                province: cell(7),
                place_hour: cell(8),
                template_hour: cell(10),
                killer_slot: cell(19),
            }
        })
        .collect()
}

fn event(kind: u8, division: u64, slot: i64, native_hour: i64, exact: bool, after: Option<i64>, place: Option<(i64, i64)>, template_hour: Option<i64>, killer_slot: Option<i64>) -> Event {
    Event {
        kind,
        division,
        slot: Some(slot),
        native_hour: Some(native_hour),
        exact: Some(i64::from(exact)),
        after,
        province: place.map(|(province, _)| province),
        place_hour: place.and_then(|(_, hour)| (hour >= 0).then_some(hour)),
        template_hour,
        killer_slot,
    }
}

#[test]
fn sightings_after_the_baseline_and_witnessed_removals_are_rows_of_their_hour_with_what_the_deletion_found() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut listed = Vec::new();
    let mut name_before = None;
    for hour in 0..=5 {
        if hour == 2 {
            let view = store.view(TableId::DivisionStatus.index());
            name_before = view.slot(u128::from(D1)).and_then(|slot| view.list(inputs::NAMES[1], slot)).map(<[u8]>::to_vec);
        }
        run(&mut store, &mut writers, hour);
        seen.push(events(&store));
        listed.push(testing::listed(&store, TableId::DivisionEvents));
        if hour == 2 {
            let lost = key(DivisionEventRow::LOST, 2, D1);
            let view = store.view(TableId::DivisionEvents.index());
            let slot = view.slot(lost).unwrap();
            assert_eq!(view.list(12, slot).map(<[u8]>::to_vec), name_before, "the removed division's name operands as the deletion found them");
            assert_eq!(view.number(13, slot), Some(1), "read at the hour before");
            assert_eq!(view.number(20, slot), Some(KILLER as i64), "the covered death's killer division");
            assert!(matches!(view.value(16, slot), Held::Text(_)), "its template's icon sub-unit name");
            assert_eq!(view.number(21, slot), Some((TEMPLATE >> 32) as i64), "the template facet is the handle's id");
            assert_eq!(view.number(22, slot), Some(1), "one covered death matched");
            assert_eq!(view.text(23, slot).and_then(|id| view.string(id)), Some(&b"Infantry Division"[..]), "the template's literal name");
            assert!(!view.recorded(24, slot), "no key where the literal names the template");
            assert_eq!(view.number(25, slot), Some(2), "the template's raw country");
        }
        if hour == 3 {
            let view = store.view(TableId::DivisionEvents.index());
            let slot = view.slot(key(DivisionEventRow::TRANSFER, 3, D2)).unwrap();
            assert_eq!(view.number(22, slot), None, "a transfer matches no death");
        }
        if hour == 4 {
            assert!(matches!(store.view(TableId::DivisionEvents.index()).status(), Status::Unavailable { .. }), "Divisions not read: the log is not known that hour");
        }
    }
    let (created, lost, transfer, vanished) = (DivisionEventRow::CREATED, DivisionEventRow::LOST, DivisionEventRow::TRANSFER, DivisionEventRow::VANISHED);
    assert_eq!(
        seen,
        [
            vec![],
            vec![event(created, D3, 20, 1, false, Some(0), Some((250, 1)), Some(1), None), event(created, D4, 10, 1, false, Some(0), Some((300, 1)), Some(1), None)],
            vec![event(lost, D1, 10, 2, true, None, Some((555, 2)), Some(1), Some(20))],
            vec![event(transfer, D2, 20, 3, false, None, Some((200, 2)), Some(2), None), event(vanished, D3, 20, 3, false, None, Some((250, 2)), Some(2), None)],
            vec![],
            vec![event(created, D5, 20, 5, false, Some(3), Some((400, 5)), Some(5), None), event(lost, D4, 20, 5, false, None, Some((300, -1)), None, None)],
        ],
        "the baseline census holds no sighting; an intel copy shown later is sighted; an unchanged witness is no event; a division leaving the census with no witness vanished; an hour the writer did not run appends nothing, and the next reads every witness and division whole, its deleted rows' read hours not known"
    );
    assert_eq!(listed, [vec![], vec![0, 1], vec![2], vec![3, 4], vec![3, 4], vec![5, 6]], "each row is appended once and forgotten at the writer's next hour; an hour it did not run forgets nothing");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=5 {
        run(&mut unbroken, &mut writing, hour);
        expected.push((testing::rows(&unbroken, TableId::DivisionEvents), testing::listed(&unbroken, TableId::DivisionEvents)));
    }
    for stop in [2, 4] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour);
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=5 {
            run(&mut resumed, &mut after, hour);
            assert_eq!((testing::rows(&resumed, TableId::DivisionEvents), testing::listed(&resumed, TableId::DivisionEvents)), expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}

#[test]
fn a_death_logged_the_hour_before_its_removal_dates_it_through_a_resume() {
    let lost = key(DivisionEventRow::LOST, 2, D1);
    let at_two = |store: &Store| {
        let view = store.view(TableId::DivisionEvents.index());
        let slot = view.slot(lost).unwrap();
        (view.number(4, slot), view.number(7, slot), view.number(19, slot), view.number(22, slot))
    };
    let (mut unbroken, mut writing) = (testing::store(), writers());
    for hour in 0..=2 {
        run_logging(&mut unbroken, &mut writing, hour, 1);
    }
    assert_eq!(at_two(&unbroken), (Some(1), Some(555), Some(20), Some(1)), "dated at its death an hour before, its killer named, one death matched");
    let (mut stopped, mut before) = (testing::store(), writers());
    for hour in 0..=1 {
        run_logging(&mut stopped, &mut before, hour, 1);
    }
    let saved = before.save().unwrap();
    let mut resumed = testing::resumed(&stopped, TEXTS);
    let mut after = writers();
    after.load(&saved, &mut resumed).unwrap();
    after.prime(&resumed, 1, 1);
    run_logging(&mut resumed, &mut after, 2, 1);
    assert_eq!(testing::rows(&resumed, TableId::DivisionEvents), testing::rows(&unbroken, TableId::DivisionEvents));
    assert_eq!(testing::listed(&resumed, TableId::DivisionEvents), testing::listed(&unbroken, TableId::DivisionEvents), "the log lists the same rows after the take-up");
}
