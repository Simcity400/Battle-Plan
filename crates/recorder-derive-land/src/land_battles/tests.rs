use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};
use replay_data::rows::{CountryLinkRow, LandCombatResultsRow, LandEquipmentLoss, LandEventCoverageRow, LandResultSide, LandSideLosses};

const TEXTS: &[&[u8]] = &[b"not read"];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(LandBattles::default())]);
    writers.start_run();
    writers
}

#[derive(Clone)]
struct Fought {
    id: u64,
    province: u32,
    duration: i32,
    attackers: Vec<u64>,
    defenders: Vec<u64>,
    sides: Option<[Vec<u32>; 2]>,
    manpower: [i32; 2],
}

fn fought(id: u64, province: u32, duration: i32, attackers: &[u64], defenders: &[u64]) -> Fought {
    Fought { id, province, duration, attackers: attackers.to_vec(), defenders: defenders.to_vec(), sides: Some([vec![1], vec![2]]), manpower: [duration * 3, duration * 2] }
}

fn finalizer(id: u64, combat: u64, province: u32, end_hour: i32) -> LandCombatResultsRow {
    LandCombatResultsRow {
        id,
        combat,
        province,
        duration: 2,
        end_hour,
        winners: 1,
        sides: [LandResultSide { countries: vec![1], ..Default::default() }, LandResultSide { countries: vec![2], ..Default::default() }],
        losses: [LandSideLosses { manpower: 10, equipment: Vec::new() }, LandSideLosses { manpower: 20, equipment: Vec::new() }],
        ..Default::default()
    }
}

#[derive(Default)]
struct Input {
    combats: Vec<Fought>,
    finalizers: Vec<LandCombatResultsRow>,
    drain: Option<bool>,
    unread: bool,
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, first: bool, input: &Input) {
    store.begin_hour(hour, if first { TEXTS } else { &[] }).unwrap();
    for table in [TableId::LandCombats, TableId::LandCombatSides, TableId::LandCombatProgress, TableId::LandCombatResults, TableId::LandEventCoverage, TableId::CountryLinks] {
        let status = if input.unread && table == TableId::LandCombats { Status::Unavailable { reason: 0 } } else { Status::Complete };
        store.status(table.index(), status, true, Some(hour), true).unwrap();
    }
    if first {
        for (id, canonical_slot) in [(1, 10), (2, 20)] {
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot }, TEXTS).unwrap();
        }
    }
    if !input.unread {
        let keys: Vec<u128> = input.combats.iter().map(|combat| u128::from(combat.id)).collect();
        testing::keep_only(store, TableId::LandCombats, &keys).unwrap();
        let with_sides: Vec<u128> = input.combats.iter().filter(|combat| combat.sides.is_some()).map(|combat| u128::from(combat.id)).collect();
        testing::keep_only(store, TableId::LandCombatSides, &with_sides).unwrap();
        testing::keep_only(store, TableId::LandCombatProgress, &keys).unwrap();
        for combat in &input.combats {
            let key = u128::from(combat.id);
            testing::cells(store, TableId::LandCombats, key, &[(COMBAT[0], Cell::U32(combat.province)), (COMBAT[1], Cell::I32(combat.duration)), (COMBAT[2], Cell::U64List(&combat.attackers)), (COMBAT[3], Cell::U64List(&combat.defenders))]).unwrap();
            if let Some([attacking, defending]) = &combat.sides {
                let none: &[u32] = &[];
                testing::cells(store, TableId::LandCombatSides, key, &[(SIDES[0], Cell::U32List(attacking)), (SIDES[1], Cell::U32List(none)), (SIDES[2], Cell::U32List(defending)), (SIDES[3], Cell::U32List(none))]).unwrap();
            }
            testing::cells(store, TableId::LandCombatProgress, key, &[(PROGRESS[0], Cell::I32(combat.manpower[0])), (PROGRESS[1], Cell::I32(combat.manpower[1]))]).unwrap();
        }
    }
    testing::keep_only(store, TableId::LandCombatResults, &[]).unwrap();
    for row in &input.finalizers {
        testing::put(store, TableId::LandCombatResults, row, TEXTS).unwrap();
    }
    testing::keep_only(store, TableId::LandEventCoverage, &[]).unwrap();
    if let Some(complete) = input.drain {
        testing::put(store, TableId::LandEventCoverage, &LandEventCoverageRow { id: hour as u32, epoch: 1, from_hour: hour - 1, to_hour: hour, complete, lost_events: 0 }, TEXTS).unwrap();
    }
    store.settle();
    writers.hour(1, hour, first, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn battles(store: &Store) -> Vec<(u64, i32, bool, i32, Option<(i64, i64)>)> {
    let view = store.view(TableId::LandBattles.index());
    let mut rows: Vec<_> = view
        .rows()
        .map(|slot| {
            let number = |column| view.number(column, slot);
            (view.key(slot).unwrap() as u64, number(0).unwrap() as i32, number(1) == Some(1), number(2).unwrap() as i32, number(3).zip(number(4)))
        })
        .collect();
    rows.sort_unstable();
    rows
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ending {
    combat: u64,
    start: i32,
    result_only: bool,
    kind: u8,
    end_hour: i32,
    last_read: Option<i64>,
    result: Option<i64>,
    result_read: bool,
    attackers: Option<Vec<u32>>,
    defenders: Option<Vec<u32>>,
    unknown: u8,
    result_attackers: Option<Vec<u32>>,
}

fn ends(store: &Store) -> Vec<Ending> {
    let view = store.view(TableId::LandBattleEnds.index());
    let list = |column, slot| view.list(column, slot).map(|bytes| bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().unwrap())).collect());
    let mut rows: Vec<Ending> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            let number = |column| view.number(column, slot);
            Ending {
                combat: (key >> 33) as u64,
                start: key as u32 as i32,
                result_only: (key >> 32) & 1 == 1,
                kind: number(0).unwrap() as u8,
                end_hour: number(1).unwrap() as i32,
                last_read: number(5),
                result: number(18),
                result_read: number(19) == Some(1),
                attackers: list(15, slot),
                defenders: list(16, slot),
                unknown: number(17).unwrap() as u8,
                result_attackers: list(24, slot),
            }
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn ending(combat: u64, start: i32, kind: u8, end_hour: i32, last_read: Option<i64>, result: Option<i64>, result_read: bool) -> Ending {
    Ending {
        combat,
        start,
        result_only: kind == End::RESULT_ONLY,
        kind,
        end_hour,
        last_read,
        result,
        result_read,
        attackers: Some(vec![10]),
        defenders: Some(vec![20]),
        unknown: 0,
        result_attackers: result.map(|_| vec![10]),
    }
}

fn script(hour: i32) -> Input {
    let drained = Some(true);
    match hour {
        0 => Input { combats: vec![fought(7, 100, 5, &[1, 2, 2], &[3])], finalizers: vec![finalizer(40, 9, 900, 0)], drain: drained, unread: false },
        1 => {
            let mut eight = fought(8, 200, 0, &[4], &[5]);
            eight.sides = Some([vec![3], vec![2]]);
            Input { combats: vec![fought(7, 100, 6, &[1, 2, 6], &[3]), eight], drain: drained, ..Input::default() }
        }
        2 => {
            let mut eight = fought(8, 200, 1, &[4], &[5]);
            eight.sides = Some([vec![3], vec![2]]);
            Input { combats: vec![fought(7, 101, 7, &[1], &[3]), eight], drain: drained, ..Input::default() }
        }
        3 => Input { combats: vec![fought(7, 101, 8, &[1], &[3])], finalizers: vec![finalizer(50, 8, 200, 3), finalizer(51, 99, 300, 2)], drain: drained, unread: false },
        4 => Input { combats: vec![fought(7, 101, 9, &[1], &[3])], finalizers: vec![finalizer(52, 7, 101, 3)], drain: drained, unread: false },
        5 => Input { combats: vec![fought(7, 101, 10, &[1], &[3]), fought(10, 400, 0, &[8], &[9])], drain: Some(false), ..Input::default() },
        6 => Input { unread: true, drain: drained, ..Input::default() },
        7 => Input { combats: vec![fought(7, 101, 12, &[1], &[3]), fought(10, 400, 2, &[8], &[9])], drain: drained, ..Input::default() },
        _ => Input { combats: vec![fought(7, 101, 13, &[1], &[3])], drain: Some(false), ..Input::default() },
    }
}

#[test]
fn battles_begin_peak_and_end_by_absence_reuse_finalizer_and_unread_census() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    for hour in 0..=8 {
        run(&mut store, &mut writers, hour, hour == 0, &script(hour));
        seen.push((battles(&store), ends(&store)));
    }
    assert_eq!(seen[0], (vec![(7, 0, false, 5, Some((3, 0)))], vec![]), "a run's first hour is a baseline: its battles bounded, its finalizer no event");
    assert_eq!(seen[1].0, vec![(7, 0, false, 5, Some((4, 1))), (8, 1, true, 0, Some((2, 1)))], "a peak rises; a battle read the hour after a read starts exact");
    assert_eq!(seen[2].0, vec![(7, 2, true, 0, Some((2, 2))), (8, 1, true, 0, Some((2, 1)))]);
    assert_eq!(seen[2].1, vec![ending(7, 0, End::REUSED, 2, Some(1), None, true)], "a handle at another province is a battle of its own");
    let mut absent = ending(8, 1, End::ABSENT, 3, Some(2), Some(50), true);
    (absent.attackers, absent.unknown) = (Some(vec![]), End::ATTACKERS_UNKNOWN);
    let mut own = ending(99, 2, End::RESULT_ONLY, 2, None, Some(51), true);
    (own.attackers, own.defenders, own.unknown) = (None, None, End::ATTACKERS_UNKNOWN | End::DEFENDERS_UNKNOWN);
    assert_eq!(seen[3].1, vec![absent, own], "a finalizer ends the battle gone at its hour, at its own end hour; one that ends none, even stamped with the read before, is a result of its own");
    assert_eq!(seen[4].0, vec![], "a finalizer stamped with the read before ends a battle whose row is still read");
    assert_eq!(seen[4].1, vec![ending(7, 2, End::FINALIZED, 3, Some(4), Some(52), true)]);
    assert_eq!(seen[5].0, vec![(7, 5, true, 0, Some((2, 5))), (10, 5, true, 0, Some((2, 5)))], "the finalized combat read again is a new battle");
    assert_eq!(seen[5].1, vec![], "end rows live one hour");
    assert_eq!(seen[6], seen[5], "an hour whose census is not read writes nothing");
    assert_eq!(seen[7].0, vec![(7, 7, false, 12, Some((2, 7))), (10, 7, false, 2, Some((2, 7)))], "after hours not read every combat starts a bounded battle");
    assert_eq!(
        seen[7].1,
        vec![ending(7, 5, End::UNKNOWN, 6, Some(5), None, false), ending(10, 5, End::UNKNOWN, 6, Some(5), None, false)],
        "every battle of the hour before ends at the first hour not read, its ending not recorded"
    );
    assert_eq!(seen[8].1, vec![ending(10, 7, End::ABSENT, 8, Some(7), None, false)], "an end under a drain not read whole has its result not recorded");
}

#[test]
fn a_result_of_the_read_before_ends_the_battle_it_belongs_to_never_the_one_that_replaced_it() {
    let (mut store, mut writers) = (testing::store(), writers());
    let drain = Some(true);
    let one = |id, province, duration| fought(id, province, duration, &[1], &[2]);
    let hours = [
        Input { combats: vec![one(20, 500, 5), one(30, 600, 3)], drain, ..Input::default() },
        Input { combats: vec![one(20, 500, 6), one(30, 600, 4), one(40, 700, 0), one(50, 800, 0)], drain, ..Input::default() },
        Input { combats: vec![one(20, 500, 7), one(30, 600, 5), one(40, 700, 1), one(50, 800, 1)], finalizers: vec![finalizer(70, 20, 500, 1)], drain, ..Input::default() },
        Input {
            combats: vec![one(30, 600, 0), one(50, 800, 2), one(60, 900, 0)],
            finalizers: vec![finalizer(71, 30, 600, 2), finalizer(72, 40, 700, 2), finalizer(73, 60, 900, 2)],
            drain,
            ..Input::default()
        },
        Input { combats: vec![one(30, 600, 1)], finalizers: vec![finalizer(74, 50, 800, 4)], drain, ..Input::default() },
    ];
    let mut seen = Vec::new();
    for (hour, input) in hours.iter().enumerate() {
        run(&mut store, &mut writers, hour as i32, hour == 0, input);
        seen.push((battles(&store), ends(&store)));
    }
    assert_eq!(seen[2].1, vec![ending(20, 0, End::FINALIZED, 1, Some(2), Some(70), true)], "a battle still read when its result of the read before arrives");
    assert_eq!(
        seen[3].1,
        vec![
            ending(30, 0, End::REUSED, 2, Some(2), Some(71), true),
            ending(40, 1, End::ABSENT, 2, Some(2), Some(72), true),
            ending(60, 3, End::FINALIZED, 2, Some(3), Some(73), true),
        ],
        "the reused battle takes its result, not its replacement; a battle gone, and one begun and ended after the read before, take theirs"
    );
    assert_eq!(seen[3].0, vec![(30, 3, true, 0, Some((2, 3))), (50, 1, true, 0, Some((2, 1)))], "the replacement goes on");
    assert_eq!(seen[4].0, vec![(30, 3, true, 0, Some((2, 3)))], "its next read is the same battle, not a third");
    assert_eq!(seen[4].1, vec![ending(50, 1, End::ABSENT, 4, Some(3), Some(74), true)], "a result of the hour after its last read");
}

#[test]
fn two_results_of_one_combat_ending_at_one_hour_are_one_row_of_what_they_agree_on() {
    let (mut store, mut writers) = (testing::store(), writers());
    run(&mut store, &mut writers, 0, true, &Input { drain: Some(true), ..Input::default() });
    let lost = |variant, amount_raw| LandEquipmentLoss { variant, amount_raw };
    let mut first = finalizer(60, 99, 300, 1);
    first.losses[0].equipment = vec![lost(7, 150_000)];
    first.losses[1].equipment = vec![lost(8, 25_000), lost(9, 100_000)];
    let mut other = finalizer(61, 99, 300, 1);
    other.winners = 2;
    other.losses[0].equipment = vec![lost(7, 50_000)];
    other.losses[1].equipment = first.losses[1].equipment.clone();
    run(&mut store, &mut writers, 1, false, &Input { finalizers: vec![first, other], drain: Some(true), ..Input::default() });
    assert_eq!(store.view(TableId::LandBattleEnds.index()).status(), Status::Complete);
    let view = store.view(TableId::LandBattleEnds.index());
    let slots: Vec<u32> = view.rows().collect();
    assert_eq!(slots.len(), 1, "one row");
    let number = |column| view.number(column, slots[0]);
    assert_eq!((number(0), number(1), number(4)), (Some(i64::from(End::RESULT_ONLY)), Some(1), Some(300)));
    assert_eq!((number(18), number(19)), (None, Some(1)), "two results: no one result id, a result read");
    assert_eq!((number(20), number(21), number(22), number(23)), (None, Some(2), Some(10), Some(20)), "the winners they disagree on are not recorded");
    let pool = |column| view.list(column, slots[0]).map(|bytes| <Vec<LandEquipmentLoss> as replay_data::value::Value>::decode(&mut replay_data::codec::Decoder::new(bytes)).expect("a pool"));
    assert_eq!(pool(27), None, "the attacker's final equipment they give apart is not recorded");
    assert_eq!(pool(28), Some(vec![lost(8, 25_000), lost(9, 100_000)]), "the defender's final equipment by design, as both give it");
    let mut own = ending(99, 1, End::RESULT_ONLY, 1, None, None, true);
    (own.attackers, own.defenders, own.unknown, own.result_attackers) = (None, None, End::ATTACKERS_UNKNOWN | End::DEFENDERS_UNKNOWN, Some(vec![10]));
    assert_eq!(ends(&store), vec![own]);
    run(&mut store, &mut writers, 2, false, &Input { finalizers: vec![finalizer(62, 99, 300, 1)], drain: Some(true), ..Input::default() });
    let mut late = ending(99, 1, End::RESULT_ONLY, 1, None, Some(62), true);
    (late.attackers, late.defenders, late.unknown) = (None, None, End::ATTACKERS_UNKNOWN | End::DEFENDERS_UNKNOWN);
    assert_eq!(ends(&store), vec![late], "a result of the same combat and hour in the next drain is a row of its own, written that hour under the same key");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=8 {
        run(&mut unbroken, &mut writing, hour, hour == 0, &script(hour));
        expected.push((testing::rows(&unbroken, TableId::LandBattles), testing::rows(&unbroken, TableId::LandBattleEnds)));
    }
    for stop in [3, 4] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour, hour == 0, &script(hour));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=8 {
            run(&mut resumed, &mut after, hour, false, &script(hour));
            let rows = (testing::rows(&resumed, TableId::LandBattles), testing::rows(&resumed, TableId::LandBattleEnds));
            assert_eq!(rows, expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}
