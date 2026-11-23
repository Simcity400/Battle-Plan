use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::{CountryLinkRow, CountryRow};
use std::collections::HashMap;

#[derive(Default)]
struct Fixed {
    registrations: HashMap<u128, Vec<u64>>,
    wings: HashMap<u64, (Option<i32>, Option<i64>)>,
}

impl Joins for Fixed {
    fn registration(&mut self, key: u128) -> io::Result<Result<Vec<u64>, u8>> {
        Ok(self.registrations.get(&key).cloned().ok_or(Score::NO_REGISTRATION))
    }

    fn wing(&mut self, id: u64) -> io::Result<Result<(Option<i32>, Option<i64>), u8>> {
        Ok(self.wings.get(&id).copied().ok_or(Score::NO_WING))
    }
}

fn site(base: u64, access: u8) -> GameScoreAirSite {
    let error = if access == GameScoreAirSite::ACCESS_UNKNOWN { b"unread".to_vec() } else { Vec::new() };
    GameScoreAirSite { base, access, error }
}

fn input(country: u32, sites: Vec<GameScoreAirSite>) -> CountryGameScoreInputRow {
    CountryGameScoreInputRow {
        country,
        army_count: Some(12),
        navy_count: Some(3),
        land_count: Some(40),
        air_state: if sites.is_empty() { CountryGameScoreInputRow::AIR_ABSENT } else { CountryGameScoreInputRow::AIR_PRESENT },
        first_sites: sites,
        ..Default::default()
    }
}

const ONES: [Option<i64>; 6] = [Some(Q), Some(Q), Some(Q), Some(Q), Some(Q), Some(Q / 2)];

const FACTORIES: [i64; 7] = [250_000, 1_090_000, 700_000, 1, 0, 0, 0];

fn joins() -> Fixed {
    Fixed {
        registrations: [(5 << 32 | 10, vec![100, 101, 100])].into(),
        wings: [(100, (Some(10), Some(2 * Q))), (101, (Some(1), Some(Q)))].into(),
    }
}

#[test]
fn every_part_follows_the_games_arithmetic_and_the_total_leaves_out_industry_and_land() {
    let (mut joins, mut reach) = (joins(), Reach::default());
    let row = input(1, vec![site(5, GameScoreAirSite::ACCESS_TRUE), site(6, GameScoreAirSite::ACCESS_FALSE)]);
    let parts = scores(Ok(&row), Ok(FACTORIES), &ONES, Some(10), &mut joins, &mut reach).unwrap();
    let air = 2 * (20 * Q) + Q;
    assert_eq!(parts, [Ok(12 * Q), Ok(3 * Q), Ok(air), Ok((2 + 10 + 7 - 1) * Q), Ok(40 * Q), Ok((15 * Q + air) / 2)]);
    assert_eq!(reach, Reach { registrations: vec![5 << 32 | 10], wings: vec![100, 101, 100] }, "a base without access is never looked up");
    let absent = scores(Ok(&input(2, Vec::new())), Ok(FACTORIES), &[Some(Q), Some(Q), None, Some(Q), Some(Q), Some(Q)], None, &mut joins, &mut Reach::default()).unwrap();
    assert_eq!(absent[AIR], Ok(0), "no strategic-air entry is a known zero, whatever its multiplier");
}

#[test]
fn what_cannot_be_read_is_not_recorded_by_name_never_zero() {
    let mut joins = joins();
    let unknown = |parts: [Part; 6]| parts.map(|part| part.err());
    let missing = scores(Err(Score::NO_INPUT_ROW), Ok(FACTORIES), &ONES, Some(10), &mut joins, &mut Reach::default()).unwrap();
    let no_row = Some(Score::NO_INPUT_ROW);
    assert_eq!(unknown(missing), [no_row, no_row, no_row, None, no_row, no_row], "industry needs no inputs row");
    let row = input(1, vec![site(5, GameScoreAirSite::ACCESS_TRUE)]);
    let mut multipliers = ONES;
    multipliers[AIR] = None;
    let parts = scores(Ok(&row), Err(Score::NO_FACTORIES_ROW), &multipliers, Some(10), &mut joins, &mut Reach::default()).unwrap();
    assert_eq!(unknown(parts), [None, None, Some(Score::MULTIPLIER), Some(Score::NO_FACTORIES_ROW), None, Some(Score::TOTAL_PARTS)]);
    let air = |row: &CountryGameScoreInputRow, slot: Option<u32>, joins: &mut Fixed| {
        let mut reach = Reach::default();
        let part = super::air(row, slot, Some(Q), joins, &mut reach).unwrap();
        (part, reach)
    };
    assert_eq!(air(&row, None, &mut joins).0, Err(Score::NO_LINK));
    let unread = input(1, vec![site(5, GameScoreAirSite::ACCESS_UNKNOWN)]);
    assert_eq!(air(&unread, Some(10), &mut joins).0, Err(Score::BASE_ACCESS));
    let elsewhere = input(1, vec![site(7, GameScoreAirSite::ACCESS_TRUE)]);
    assert_eq!(air(&elsewhere, Some(10), &mut joins), (Err(Score::NO_REGISTRATION), Reach { registrations: vec![7 << 32 | 10], wings: Vec::new() }), "a missing registration is reached, so its arrival recomputes");
    joins.wings.remove(&101);
    assert_eq!(air(&row, Some(10), &mut joins), (Err(Score::NO_WING), Reach { registrations: vec![5 << 32 | 10], wings: vec![100, 101] }));
    joins.wings.insert(101, (None, Some(Q)));
    assert_eq!(air(&row, Some(10), &mut joins).0, Err(Score::PLANES));
    let entry = CountryGameScoreInputRow { air_state: CountryGameScoreInputRow::AIR_UNKNOWN, air_error: b"unread".to_vec(), first_sites: Vec::new(), ..row };
    assert_eq!(air(&entry, Some(10), &mut joins).0, Err(Score::AIR_ENTRY));
    assert_eq!(counted(None, Some(Q)), Err(Score::COUNT_UNREAD));
    assert_eq!(counted(Some(1), None), Err(Score::MULTIPLIER));
}

const TEXTS: &[&[u8]] = &[b"", b"not read"];

fn world(store: &mut Store, hour: i32) {
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryGameScoreInputs,
        TableId::GameScoreMultipliers,
        TableId::CountryFactories,
        TableId::AirBaseRegistrations,
        TableId::AirWingGameScoreStats,
    ] {
        store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    }
    for (id, slot) in [(1, 10), (2, 20)] {
        testing::put(store, TableId::Countries, &CountryRow { id, canonical_slot: slot, ..Default::default() }, TEXTS).unwrap();
        testing::put(store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot: slot }, TEXTS).unwrap();
        let factories = CountryFactoriesRow { country: id, military_total_raw: 300_000, ..Default::default() };
        testing::put(store, TableId::CountryFactories, &factories, TEXTS).unwrap();
    }
    testing::put(store, TableId::CountryGameScoreInputs, &input(1, vec![site(5, GameScoreAirSite::ACCESS_TRUE)]), TEXTS).unwrap();
    testing::put(store, TableId::CountryGameScoreInputs, &input(2, Vec::new()), TEXTS).unwrap();
    let multipliers = GameScoreMultiplierRow {
        total_raw: Some(Q),
        army_raw: Some(Q),
        navy_raw: Some(Q),
        air_raw: Some(Q),
        industry_raw: Some(Q),
        land_raw: Some(Q),
        ..Default::default()
    };
    testing::put(store, TableId::GameScoreMultipliers, &multipliers, TEXTS).unwrap();
    let registration = AirBaseRegistrationRow { base: 5, slot: 10, capacity_raw: 1, wings: vec![100, 101], ..Default::default() };
    testing::put(store, TableId::AirBaseRegistrations, &registration, TEXTS).unwrap();
    wing(store, 100, Some(10), 2 * Q);
    wing(store, 101, Some(1), Q);
}

fn wing(store: &mut Store, id: u64, planes: Option<i32>, stat: i64) {
    let planes_error = if planes.is_none() { b"not read".to_vec() } else { Vec::new() };
    let row = AirWingGameScoreStatsRow { id, planes, planes_error, stat_raw: Some(stat), mission_index: Some(1), ..Default::default() };
    testing::put(store, TableId::AirWingGameScoreStats, &row, TEXTS).unwrap();
}

fn begin(store: &mut Store, hour: i32) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
}

fn edit(store: &mut Store, hour: i32) {
    match hour {
        0 => world(store, hour),
        1 => wing(store, 101, Some(1), 3 * Q),
        2 => wing(store, 100, None, 2 * Q),
        3 => {
            testing::remove(store, TableId::Countries, 2).unwrap();
            testing::remove(store, TableId::CountryGameScoreInputs, 2).unwrap();
        }
        4 => store.status(TableId::AirWingGameScoreStats.index(), Status::Unavailable { reason: 1 }, true, Some(hour), false).unwrap(),
        _ => {}
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
    begin(store, hour);
    edit(store, hour);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(GameScore::default())]);
    writers.start_run();
    writers
}

fn air_of(store: &Store, id: u128) -> (Option<i64>, Option<i64>) {
    let view = store.view(TableId::CountryGameScores.index());
    let slot = view.slot(id).expect("a row");
    (view.number(AIR, slot), view.number(6 + AIR, slot))
}

#[test]
fn a_wing_that_changes_moves_the_air_score_of_the_country_that_reaches_it_and_a_gone_country_has_no_row() {
    let (mut store, mut writers) = (testing::store(), writers());
    run(&mut store, &mut writers, 0);
    assert_eq!(air_of(&store, 1), (Some(20 * Q + Q), Some(0)));
    assert_eq!(air_of(&store, 2), (Some(0), Some(0)));
    run(&mut store, &mut writers, 1);
    assert_eq!(air_of(&store, 1), (Some(20 * Q + 3 * Q), Some(0)));
    run(&mut store, &mut writers, 2);
    assert_eq!(air_of(&store, 1), (None, Some(i64::from(Score::PLANES))));
    let total = store.view(TableId::CountryGameScores.index());
    assert_eq!(total.number(TOTAL, total.slot(1).unwrap()), None, "no total without air");
    run(&mut store, &mut writers, 3);
    assert_eq!(testing::rows(&store, TableId::CountryGameScores).iter().map(|(key, _)| *key).collect::<Vec<_>>(), [1]);
    run(&mut store, &mut writers, 4);
    assert_eq!(air_of(&store, 1), (None, Some(i64::from(Score::WINGS_INCOMPLETE))));
}

#[test]
fn an_hour_recomputes_only_the_countries_its_changes_reach() {
    let mut store = testing::store();
    let mut score = GameScore::default();
    let mut keys = Vec::new();
    for at in 0..=1 {
        begin(&mut store, at);
        edit(&mut store, at);
        store.settle();
        for table in OPTIONAL.into_iter().chain([TableId::Countries, TableId::CountryLinks, TableId::CountryGameScores]) {
            store.index(table.index());
        }
        let hour = Hour::new(&store, 1, at, at == 0, at == 0, &[]);
        keys.push(testing::logging(&store, score.tables(), |rows| {
            score.hour(&hour, rows)?;
            Ok(rows.keys(TableId::CountryGameScores).0)
        }).unwrap());
        if at == 0 {
            let mut duplicate = GameScore::default();
            let error = testing::logging(&store, duplicate.tables(), |rows| {
                duplicate.hour(&hour, rows)?;
                duplicate.hour(&hour, rows)?;
                Ok(rows.keys(TableId::CountryGameScores).0)
            }).unwrap_err();
            assert_eq!(error.to_string(), format!("it set one {} row twice in one hour", TableId::CountryGameScores.words()));
        }
        store.end_hour(&mut Discard).unwrap();
    }
    assert_eq!(keys, [vec![1, 2], vec![1]], "wing 101 is reached by country 1 alone");
}

#[test]
fn a_wing_change_that_moves_no_score_sets_no_row() {
    let edit = |store: &mut Store, at: i32| {
        if at == 0 {
            world(store, at);
        } else {
            let row = AirWingGameScoreStatsRow { id: 100, planes: Some(10), planes_error: b"not read".to_vec(), stat_raw: Some(2 * Q), mission_index: Some(1), ..Default::default() };
            testing::put(store, TableId::AirWingGameScoreStats, &row, TEXTS).unwrap();
        }
    };
    let mut store = testing::store();
    let mut score = GameScore::default();
    let mut recomputed = Vec::new();
    for at in 0..=1 {
        begin(&mut store, at);
        edit(&mut store, at);
        store.settle();
        for table in OPTIONAL.into_iter().chain([TableId::Countries, TableId::CountryLinks, TableId::CountryGameScores]) {
            store.index(table.index());
        }
        recomputed.push(testing::logging(&store, score.tables(), |rows| {
            score.hour(&Hour::new(&store, 1, at, at == 0, at == 0, &[]), rows)?;
            Ok(rows.keys(TableId::CountryGameScores).0)
        }).unwrap());
        store.end_hour(&mut Discard).unwrap();
    }
    assert_eq!(recomputed, [vec![1, 2], vec![1]], "the wing's country is recomputed");
    assert_eq!(score.by_wing.get(&100), Some(&vec![1]));
    assert_eq!(score.reach.get(&1), Some(&Reach { registrations: vec![5 << 32 | 10], wings: vec![100, 101] }));
    let (mut store, mut writers) = (testing::store(), Writers::new(vec![Box::new(testing::noting::Noting(GameScore::default()))]));
    writers.start_run();
    testing::noting::take();
    for at in 0..=1 {
        begin(&mut store, at);
        edit(&mut store, at);
        store.settle();
        writers.hour(1, at, at == 0, &mut store).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }
    let written: Vec<Vec<u128>> = testing::noting::take().iter().map(|hour| hour.iter().flat_map(|(_, sets, _)| sets.clone()).collect()).collect();
    assert_eq!(written, [vec![1, 2], Vec::new()], "its row set again equal writes nothing");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    for at in 0..=4 {
        run(&mut unbroken, &mut writing, at);
    }
    let (mut stopped, mut before) = (testing::store(), writers());
    for at in 0..=1 {
        run(&mut stopped, &mut before, at);
    }
    let saved = before.save().unwrap();
    let mut resumed = testing::resumed(&stopped, TEXTS);
    let mut after = writers();
    after.load(&saved, &mut resumed).unwrap();
    after.prime(&resumed, 1, 1);
    for at in 2..=4 {
        run(&mut resumed, &mut after, at);
    }
    assert_eq!(testing::rows(&resumed, TableId::CountryGameScores), testing::rows(&unbroken, TableId::CountryGameScores));
}

#[test]
fn its_state_is_its_tables_so_a_resume_recomputes_from_them() {
    let mut score = GameScore::default();
    let mut blob = Bytes::new();
    score.save(&mut blob).unwrap();
    assert!(blob.bytes().is_empty());
    score.load(&[]).unwrap();
    assert!(score.rebuild);
    assert!(score.load(&[1]).is_err());
}
