use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::{UnitLeaderRow, LandCombatDetailsRow, LandCombatResultsRow, LandCombatSideDetails, LandResultSide, NavalCombatDetailsRow, NavalCombatResultDetailsRow, NavalCombatSideDetails, NavalResultRow, NavalResultSideDetails, NavalResultSourceRow};

const TEXTS: &[&[u8]] = &[b"not read", b""];
const NOT_READ: u32 = 0;

const L1: u64 = 1 << 32 | 6;
const L2: u64 = 2 << 32 | 6;
const L3: u64 = 3 << 32 | 6;
const L4: u64 = 4 << 32 | 6;
const L5: u64 = 5 << 32 | 6;
const C1: u64 = 10 << 32 | 2;
const C2: u64 = 11 << 32 | 2;
const N1: u64 = 20 << 32 | 4;
const R1: u64 = 30 << 32 | 8;

const INPUTS: [TableId; 6] = [
    TableId::LandCombatDetails,
    TableId::NavalCombatDetails,
    TableId::LandCombatResults,
    TableId::NavalResultSources,
    TableId::NavalResults,
    TableId::NavalCombatResultDetails,
];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(LeaderBattles::default())]);
    writers.start_run();
    writers
}

fn land(combat: u64, [attacker, defender]: [u64; 2]) -> LandCombatDetailsRow {
    let side = |commander| LandCombatSideDetails { commander, ..Default::default() };
    LandCombatDetailsRow { combat, province: 7, sides: [side(attacker), side(defender)], ..Default::default() }
}

fn naval(combat: u64, [attacker, defender]: [u64; 2]) -> NavalCombatDetailsRow {
    let side = |commander| NavalCombatSideDetails { commander, ..Default::default() };
    NavalCombatDetailsRow { combat, province: 8, sides: [side(attacker), side(defender)], custom_sea: Some(Vec::new()), ..Default::default() }
}

fn land_result(id: u64, combat: u64, winners: u8, [attacker, defender]: [u64; 2]) -> LandCombatResultsRow {
    let side = |commander| LandResultSide { commander, ..Default::default() };
    LandCombatResultsRow { id, combat, end_hour: 2, winners, sides: [side(attacker), side(defender)], ..Default::default() }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for table in INPUTS {
        let status = if hour == 4 && table == TableId::LandCombatDetails { Status::Unavailable { reason: NOT_READ } } else { Status::Complete };
        store.status(table.index(), status, true, Some(hour), true).unwrap();
    }
    store.status(TableId::UnitLeaders.index(), Status::Complete, true, Some(hour), true).unwrap();
    if hour == 0 {
        for leader in [L1, L2, L3, L4, L5] {
            testing::put(store, TableId::UnitLeaders, &UnitLeaderRow { id: (leader >> 32) as u32, reference_type: leader as u32, ..Default::default() }, TEXTS).unwrap();
        }
    }
    if hour == 3 {
        testing::remove(store, TableId::UnitLeaders, u128::from(L2)).unwrap();
    }
    match hour {
        0 => testing::put(store, TableId::LandCombatDetails, &land(C1, [L1, L2]), TEXTS).unwrap(),
        1 => {
            testing::put(store, TableId::LandCombatDetails, &land(C1, [L1, L2]), TEXTS).unwrap();
            testing::put(store, TableId::LandCombatDetails, &land(C2, [0, L1]), TEXTS).unwrap();
            testing::put(store, TableId::NavalCombatDetails, &naval(N1, [L3, 0]), TEXTS).unwrap();
        }
        2 => {
            testing::remove(store, TableId::LandCombatDetails, u128::from(C1)).unwrap();
            testing::put(store, TableId::LandCombatResults, &land_result(1, C1, 1, [L1, L4]), TEXTS).unwrap();
        }
        3 => {
            testing::forget(store, TableId::LandCombatResults, 0).unwrap();
            testing::put(store, TableId::LandCombatResults, &land_result(2, C1, 2, [L1, L4]), TEXTS).unwrap();
            testing::remove(store, TableId::NavalCombatDetails, u128::from(N1)).unwrap();
            let source = NavalResultSourceRow { id: 1, game_hour: 3, result: R1, combat: N1, ..Default::default() };
            testing::put(store, TableId::NavalResultSources, &source, TEXTS).unwrap();
            testing::put(store, TableId::NavalResults, &NavalResultRow { id: R1, result_hour: 3, winning_side: 2, ..Default::default() }, TEXTS).unwrap();
            let details = NavalCombatResultDetailsRow { result: R1, sides: [NavalResultSideDetails { leader: L3, ..Default::default() }, NavalResultSideDetails::default()] };
            testing::put(store, TableId::NavalCombatResultDetails, &details, TEXTS).unwrap();
        }
        4 => {
            testing::forget(store, TableId::LandCombatResults, 1).unwrap();
            testing::forget(store, TableId::NavalResultSources, 0).unwrap();
        }
        _ => testing::put(store, TableId::LandCombatDetails, &land(C2, [L5, L1]), TEXTS).unwrap(),
    }
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

type Seen = (u64, i64, i64, u64, i64, i64, i64, Option<i64>, Option<i64>);

fn rows(store: &Store) -> Vec<Seen> {
    let view = store.view(TableId::LeaderBattles.index());
    testing::rows(store, TableId::LeaderBattles)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            let cell = |column| view.number(column, slot);
            ((key >> 64) as u64, cell(0).unwrap(), cell(1).unwrap(), cell(2).unwrap() as u64, cell(3).unwrap(), cell(4).unwrap(), cell(5).unwrap(), cell(6), cell(8))
        })
        .collect()
}

#[test]
fn participations_and_results_are_rows_of_their_hour_for_every_leader_of_the_combat() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut listed = Vec::new();
    for hour in 0..=5 {
        run(&mut store, &mut writers, hour);
        seen.push(rows(&store));
        listed.push(testing::listed(&store, TableId::LeaderBattles));
    }
    let (part, res) = (i64::from(LeaderBattleRow::PARTICIPATION), i64::from(LeaderBattleRow::RESULT));
    let (land, navy) = (i64::from(LeaderBattleRow::LAND), i64::from(LeaderBattleRow::NAVAL));
    let (won, lost) = (Some(i64::from(LeaderBattleRow::WON)), Some(i64::from(LeaderBattleRow::LOST)));
    assert_eq!(
        seen,
        [
            vec![],
            vec![(L1, part, land, C2, 2, 1, 0, None, None), (L3, part, navy, N1, 1, 1, 0, None, None)],
            vec![
                (L1, res, land, C1, 1, 0, 1, Some(1), won),
                (L2, res, land, C1, 2, 0, 1, Some(1), lost),
                (L4, part, land, C1, 2, 2, 1, None, None),
                (L4, res, land, C1, 2, 2, 1, Some(1), lost),
            ],
            vec![
                (L1, res, land, C1, 1, 0, 1, None, None),
                (L2, res, land, C1, 2, 0, 1, None, None),
                (L3, res, navy, N1, 1, 1, 0, Some(2), lost),
                (L4, res, land, C1, 2, 2, 1, None, None),
            ],
            vec![],
            vec![(L5, part, land, C2, 1, 5, 1, None, None)],
        ],
        "the baseline's commanders are held, not written; a result names every leader of its combat and a new one in its sides; a disagreeing result unknows the winners; an hour the writer did not run appends nothing, and the next reads every combat whole"
    );
    assert_eq!(listed, [vec![], vec![0, 1], vec![2, 3, 4, 5], vec![6, 7, 8, 9], vec![6, 7, 8, 9], vec![10]], "each row is appended once and forgotten at the writer's next hour; an hour it did not run forgets nothing");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for hour in 0..=5 {
        run(&mut unbroken, &mut writing, hour);
        expected.push((testing::rows(&unbroken, TableId::LeaderBattles), testing::listed(&unbroken, TableId::LeaderBattles), testing::rows(&unbroken, TableId::LeaderBattleTotals)));
    }
    for stop in [1, 2, 4] {
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
            let seen = (testing::rows(&resumed, TableId::LeaderBattles), testing::listed(&resumed, TableId::LeaderBattles), testing::rows(&resumed, TableId::LeaderBattleTotals));
            assert_eq!(seen, expected[hour as usize], "stopped after hour {stop}, hour {hour}");
        }
    }
}

fn totals(store: &Store) -> Vec<(u64, [i64; 3])> {
    let view = store.view(TableId::LeaderBattleTotals.index());
    testing::rows(store, TableId::LeaderBattleTotals)
        .into_iter()
        .map(|(key, _)| {
            let slot = view.slot(key).unwrap();
            (key as u64, [0, 1, 2].map(|column| view.number(column, slot).unwrap()))
        })
        .collect()
}

#[test]
fn totals_count_each_leaders_rows_of_the_run_through_the_hour() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut counted: std::collections::BTreeMap<u64, [i64; 3]> = std::collections::BTreeMap::new();
    let (part, won, lost) = (i64::from(LeaderBattleRow::PARTICIPATION), i64::from(LeaderBattleRow::WON), i64::from(LeaderBattleRow::LOST));
    for hour in 0..=5 {
        run(&mut store, &mut writers, hour);
        if store.view(TableId::LeaderBattles.index()).status() == Status::Complete {
            for (leader, entry, .., outcome) in rows(&store) {
                let held = counted.entry(leader).or_default();
                match (entry, outcome) {
                    (entry, _) if entry == part => held[0] += 1,
                    (_, Some(outcome)) if outcome == won => held[1] += 1,
                    (_, Some(outcome)) if outcome == lost => held[2] += 1,
                    _ => {}
                }
            }
        }
        if hour == 3 {
            counted.remove(&L2);
        }
        assert_eq!(totals(&store), counted.iter().map(|(leader, held)| (*leader, *held)).collect::<Vec<_>>(), "hour {hour}");
    }
    assert_eq!(totals(&store), vec![(L1, [1, 1, 0]), (L3, [1, 0, 1]), (L4, [1, 0, 1]), (L5, [1, 0, 0])]);
}
