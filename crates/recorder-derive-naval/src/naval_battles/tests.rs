use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};

const TEXTS: &[&[u8]] = &[b"not read"];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(NavalBattles::default())]);
    writers.start_run();
    writers
}

type Member = (u64, u8, u32, u64, bool);

type Linked = (u64, i32, u64, u64);

#[derive(Default)]
struct Input {
    combats: Vec<(u64, u32)>,
    members: Vec<Member>,
    results: Vec<u64>,
    sources: Vec<Linked>,
    unread: bool,
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, first: bool, input: &Input) {
    store.begin_hour(hour, if first { TEXTS } else { &[] }).unwrap();
    for table in [TableId::NavalCombats, TableId::NavalCombatParticipants, TableId::NavalResults, TableId::NavalResultSources] {
        let status = if input.unread && table == TableId::NavalCombats { Status::Unavailable { reason: 0 } } else { Status::Complete };
        store.status(table.index(), status, true, Some(hour), true).unwrap();
    }
    if !input.unread {
        let keys: Vec<u128> = input.combats.iter().map(|&(combat, _)| u128::from(combat)).collect();
        testing::keep_only(store, TableId::NavalCombats, &keys).unwrap();
        for &(combat, province) in &input.combats {
            testing::cells(store, TableId::NavalCombats, u128::from(combat), &[(PROVINCE[0], Cell::U32(province))]).unwrap();
        }
        let key = |&(combat, side, slot, _, _): &Member| u128::from(combat) << 33 | u128::from(side) << 32 | u128::from(slot);
        let keys: Vec<u128> = input.members.iter().map(key).collect();
        testing::keep_only(store, TableId::NavalCombatParticipants, &keys).unwrap();
        for member in &input.members {
            let state = if member.4 { 0x20 } else { 0 };
            testing::cells(store, TableId::NavalCombatParticipants, key(member), &[(MEMBER[0], Cell::U64(member.3)), (MEMBER[1], Cell::U32(state))]).unwrap();
        }
    }
    let results: Vec<u128> = input.results.iter().map(|&result| u128::from(result)).collect();
    testing::keep_only(store, TableId::NavalResults, &results).unwrap();
    for &result in &input.results {
        testing::cells(store, TableId::NavalResults, u128::from(result), &[(0, Cell::U32(7))]).unwrap();
    }
    testing::keep_only(store, TableId::NavalResultSources, &[]).unwrap();
    for &(id, game_hour, result, combat) in &input.sources {
        let given = [(0, Cell::I32(game_hour)), (1, Cell::U64(result)), (2, Cell::U64(combat)), (3, Cell::Bool(false)), (4, Cell::Bool(false)), (5, Cell::OptionalI32(None))];
        testing::cells(store, TableId::NavalResultSources, u128::from(id), &given).unwrap();
    }
    store.settle();
    writers.hour(1, hour, first, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn numbers(store: &Store, table: TableId) -> Vec<(u64, Vec<Option<i64>>)> {
    let view = store.view(table.index());
    let mut rows: Vec<_> = view.rows().map(|slot| (view.key(slot).unwrap() as u64, (0..view.kinds().len()).map(|column| view.number(column, slot)).collect())).collect();
    rows.sort_unstable();
    rows
}

fn battle(combat: u64, first: i64, peak: (i64, i64), sunk: [i64; 2]) -> (u64, Vec<Option<i64>>) {
    (combat, vec![Some(first), Some(peak.0), Some(peak.1), Some(sunk[0]), Some(sunk[1]), Some(0), Some(0)])
}

fn observation(result: u64, refusal: u8, combat: u64, observed: Option<[i64; 13]>) -> (u64, Vec<Option<i64>>) {
    let mut cells = vec![Some(i64::from(refusal)), Some(combat as i64)];
    match observed {
        Some(values) => cells.extend(values.map(Some)),
        None => cells.extend([None; 13]),
    }
    (result, cells)
}

fn script(hour: i32) -> Input {
    let afloat: Vec<Member> = vec![(1, 0, 0, 11, false), (1, 0, 1, 12, false), (1, 1, 0, 21, false)];
    match hour {
        0 => Input { combats: vec![(1, 10)], members: afloat, ..Input::default() },
        1 => Input { combats: vec![(1, 10)], members: vec![(1, 0, 0, 11, false), (1, 0, 1, 12, true), (1, 1, 0, 21, false), (1, 1, 1, 0, false)], ..Input::default() },
        2 => Input {
            combats: vec![(1, 10), (2, 20), (3, 30)],
            members: vec![(1, 0, 0, 11, false), (1, 1, 0, 21, false), (1, 1, 1, 0, false)],
            results: vec![100],
            sources: vec![(5, 1, 100, 1), (6, 2, 101, 1)],
            unread: false,
        },
        3 => Input { combats: vec![(2, 21)], results: vec![100, 102, 103, 104], sources: vec![(3, 3, 105, 1), (7, 2, 102, 2), (8, 3, 103, 9), (9, 3, 104, 3)], ..Input::default() },
        4 => Input { combats: vec![(2, 21)], results: vec![102, 103, 104], ..Input::default() },
        5 => Input { unread: true, results: vec![102, 103, 104], ..Input::default() },
        _ => Input { combats: vec![(2, 21)], results: vec![102, 103, 104], ..Input::default() },
    }
}

#[test]
fn a_result_two_sources_name_in_one_hour_holds_the_later_ones_refusal() {
    let (mut store, mut writers) = (testing::store(), writers());
    let afloat: Vec<Member> = vec![(1, 0, 0, 11, false), (1, 1, 0, 21, false)];
    run(&mut store, &mut writers, 0, true, &Input { combats: vec![(1, 10)], members: afloat.clone(), ..Input::default() });
    run(&mut store, &mut writers, 1, false, &Input { combats: vec![(1, 10)], members: afloat, results: vec![100], sources: vec![(5, 1, 100, 1), (6, 1, 100, 1)], unread: false });
    assert_eq!(store.view(TableId::NavalResultObservations.index()).status(), Status::Complete);
    assert_eq!(numbers(&store, TableId::NavalResultObservations), vec![observation(100, Observation::DUPLICATE, 1, None)]);
}

#[test]
fn battles_keep_peaks_and_floors_and_results_observe_the_battle_their_source_named() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut battles = Vec::new();
    let mut observations = Vec::new();
    for hour in 0..=6 {
        run(&mut store, &mut writers, hour, hour == 0, &script(hour));
        battles.push(numbers(&store, TableId::NavalBattles));
        observations.push(numbers(&store, TableId::NavalResultObservations));
    }
    assert_eq!(battles[0], vec![battle(1, 0, (3, 0), [0, 0])]);
    assert_eq!(battles[1], vec![battle(1, 0, (3, 0), [1, 0])], "a sunk member raises its side's floor; it still counts among the ships");
    assert_eq!(battles[2], vec![battle(1, 0, (3, 0), [1, 0]), battle(2, 2, (0, 2), [0, 0]), battle(3, 2, (0, 2), [0, 0])], "a detached member lowers no peak or floor");
    assert_eq!(battles[3], vec![battle(2, 3, (0, 3), [0, 0])], "a combat gone has no battle; one at another province is a battle of its own");
    assert_eq!(battles[5], battles[4], "an hour whose combats are not read writes nothing");
    assert_eq!(battles[6], vec![battle(2, 6, (0, 6), [0, 0])], "after hours not read every combat starts a battle");
    let at_one = observation(100, Observation::OBSERVED, 1, Some([10, 0, 1, 3, 0, 2, 0, 1, 0, 1, 1, 0, 0]));
    assert_eq!(
        observations[2],
        vec![at_one.clone(), observation(101, Observation::DUPLICATE, 1, None)],
        "a source an hour before its read observes the battle as that read left it; a combat linked twice is refused"
    );
    let ended = observation(104, Observation::OBSERVED, 3, Some([30, 2, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        observations[3],
        vec![
            at_one,
            observation(102, Observation::MISMATCH, 2, None),
            observation(103, Observation::NOT_OBSERVED, 9, None),
            ended.clone(),
            observation(105, Observation::INVALID_SOURCE, 1, None),
        ],
        "a result the census never listed loses its observation; a battle begun after its source, a combat not read and a source out of order are refused"
    );
    assert_eq!(
        observations[4],
        vec![observation(102, Observation::MISMATCH, 2, None), observation(103, Observation::NOT_OBSERVED, 9, None), ended],
        "an observation lives while its result does"
    );
    assert_eq!(observations[6], observations[4]);
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let tables = |store: &Store| (testing::rows(store, TableId::NavalBattles), testing::rows(store, TableId::NavalResultObservations));
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=6 {
        run(&mut unbroken, &mut writing, hour, hour == 0, &script(hour));
        expected.push(tables(&unbroken));
    }
    for stop in [1, 2, 3] {
        let (mut stopped, mut before) = (testing::store(), writers());
        for hour in 0..=stop {
            run(&mut stopped, &mut before, hour, hour == 0, &script(hour));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, stop);
        for hour in stop + 1..=6 {
            run(&mut resumed, &mut after, hour, false, &script(hour));
            assert_eq!(tables(&resumed), expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}
