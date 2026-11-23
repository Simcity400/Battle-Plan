use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::RowCodec;

const TEXTS: &[&[u8]] = &[b"", b"AAA", b"BBB", b"CCC"];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(Peace::default())]);
    writers.start_run();
    writers
}

struct World {
    owners: Vec<(u32, u32)>,
    wars: Vec<(u32, u32, i32, i32)>,
    owned: Vec<(u32, i32)>,
    roster: Vec<(u64, i32, u8)>,
}

fn sync<R: RowCodec>(store: &mut Store, table: TableId, rows: &[R]) {
    let keep: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (key, _) in testing::rows(store, table) {
        if !keep.contains(&key) {
            testing::remove(store, table, key).unwrap();
        }
    }
    for row in rows {
        testing::put(store, table, row, TEXTS).unwrap();
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for read in READS {
        store.status(read.table.index(), Status::Complete, true, Some(hour), true).unwrap();
    }
    let countries: Vec<CountryRow> = (1..=3).map(|id| CountryRow { id, canonical_slot: id, tag: TEXTS[id as usize].to_vec(), ..Default::default() }).collect();
    sync(store, TableId::Countries, &countries);
    let links: Vec<CountryLinkRow> = (1..=3).map(|id| CountryLinkRow { id, canonical_slot: id }).collect();
    sync(store, TableId::CountryLinks, &links);
    let states: Vec<StateRow> = world.owners.iter().map(|&(id, owner)| StateRow { id, owner, controller: owner, ..Default::default() }).collect();
    sync(store, TableId::States, &states);
    let provinces: Vec<ProvinceRow> = world.owners.iter().map(|&(state, owner)| ProvinceRow { id: state * 10, state, owner, controller: owner, victory_points: (state % 7) as i32, ..Default::default() }).collect();
    sync(store, TableId::Provinces, &provinces);
    let wars: Vec<WarRelationRow> = world.wars.iter().map(|&(first_country, second_country, start_hour, end_hour)| WarRelationRow { first_country, second_country, start_hour, end_hour, ..Default::default() }).collect();
    sync(store, TableId::WarRelations, &wars);
    let owned: Vec<CountryCounterInputRow> = world.owned.iter().map(|&(country, owned_state_count)| CountryCounterInputRow { country, owned_state_count, ..Default::default() }).collect();
    sync(store, TableId::CountryCounterInputs, &owned);
    let roster: Vec<PeaceConferenceRow> = world.roster.iter().map(|&(id, current_turn, finalized)| PeaceConferenceRow { id, ordinal: 0, current_turn, finalized }).collect();
    sync(store, TableId::PeaceConferences, &roster);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

type Outcome = (i32, u32, u8, Option<u64>, i32, u8, u32, u32, (u32, u32), (u32, u32), Option<i64>);

fn outcomes(store: &Store) -> Vec<Outcome> {
    const COLUMNS: [usize; 11] = columns::<PeaceOutcomeRow, 11>(["observation", "conference", "before_hour", "change", "group", "state", "from_slot", "from_id", "to_slot", "to_id", "victory_points"]);
    let view = store.view(TableId::PeaceOutcomes.index());
    let mut rows: Vec<Outcome> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            let number = |at: usize| view.number(COLUMNS[at], slot);
            let cell = |at: usize| number(at).unwrap();
            (
                (key >> 32) as u32 as i32,
                key as u32,
                cell(0) as u8,
                number(1).map(|id| id as u64),
                cell(2) as i32,
                cell(3) as u8,
                cell(4) as u32,
                cell(5) as u32,
                (cell(6) as u32, cell(7) as u32),
                (cell(8) as u32, cell(9) as u32),
                number(10),
            )
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn span(store: &Store, hour: i32, conference: u64) -> Vec<String> {
    let rows = testing::rows(store, TableId::PeaceConferenceSpans);
    assert!(rows.iter().all(|(key, _)| *key as u32 as i32 == hour), "a span row lives the hour it was written");
    rows.into_iter().find(|(key, _)| (key >> 32) as u64 == conference).map(|(_, cells)| cells).unwrap_or_default()
}

fn unsampled(hour: i32) -> World {
    let war = |end| vec![(1, 2, 0, end)];
    match hour {
        0 | 1 => World { owners: vec![(100, 2), (101, 2), (102, 1), (103, 2)], wars: war(0), owned: vec![(1, 1), (2, 3), (3, 0)], roster: vec![] },
        _ => World { owners: vec![(100, 1), (101, 3), (102, 1), (103, 1)], wars: war(2), owned: vec![(1, 3), (2, 0), (3, 1)], roster: vec![] },
    }
}

fn sampled(hour: i32) -> World {
    let before = World { owners: vec![(100, 2), (102, 1)], wars: vec![(1, 2, 0, 0)], owned: vec![(1, 1), (2, 1)], roster: vec![] };
    let after = |roster| World { owners: vec![(100, 1), (102, 1)], wars: vec![], owned: vec![(1, 2), (2, 0)], roster };
    match hour {
        0 => before,
        1 => World { roster: vec![(5, 1, 0)], ..before },
        2 => after(vec![(5, 2, 1)]),
        _ => after(vec![]),
    }
}

#[test]
fn a_war_end_owner_change_no_conference_claims_is_an_outcome_living_its_hour() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    for at in 0..=3 {
        run(&mut store, &mut writers, at, &unsampled(at));
        seen.push(outcomes(&store));
    }
    let (not_recorded, transfer, released) = (PeaceOutcomeRow::NOT_RECORDED, PeaceOutcomeRow::TRANSFER, PeaceOutcomeRow::RELEASED);
    assert_eq!(
        seen,
        [
            vec![],
            vec![],
            vec![
                (2, 0, not_recorded, None, 1, transfer, 0, 100, (2, 2), (1, 1), Some(2)),
                (2, 1, not_recorded, None, 1, transfer, 0, 103, (2, 2), (1, 1), Some(5)),
                (2, 2, not_recorded, None, 1, released, 0, 101, (2, 2), (3, 3), Some(3)),
            ],
            vec![],
        ],
        "the run's first hours are baselines; a transfer whose war pair ends at the after read counts; a country that owned nothing receiving a loser's state is released"
    );
}

#[test]
fn a_sampled_conference_is_compared_once_from_its_last_read_before_finalization() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut spans = Vec::new();
    let mut folded: Vec<Vec<String>> = Vec::new();
    for at in 0..=4 {
        run(&mut store, &mut writers, at, &sampled(at));
        seen.push(outcomes(&store));
        spans.push(span(&store, at, 5));
        let last = folded.last().cloned().unwrap_or_default();
        folded.push(if spans[at as usize].is_empty() { last } else { spans[at as usize].clone() });
    }
    let (recorded, transfer) = (PeaceOutcomeRow::RECORDED, PeaceOutcomeRow::TRANSFER);
    assert_eq!(seen, [vec![], vec![], vec![(2, 0, recorded, Some(5), 1, transfer, 0, 100, (2, 2), (1, 1), Some(2))], vec![], vec![]], "the pair gone from the after read is evidence; no unsampled outcome repeats it");
    let number = |value: i64| format!("{:?}", Held::Number(value as u64));
    let ended = [number(1), number(2), number(2), number(1), number(2), number(3), number(i64::from(PeaceConferenceSpanRow::COMPARED)), number(1), number(2)];
    assert_eq!(spans[3], ended, "listed at 1, last at 2, finalized at 2, gone at 3, compared from 1 at 2");
    assert_eq!(spans[2][6..], spans[3][6..], "the hour that reads the finalized roster and compares writes the comparison, not the roster read alone");
    assert!(spans[0].is_empty() && !spans[1].is_empty() && spans[4].is_empty(), "a row each hour that moved the span, none after it ended");
    assert_eq!(folded[4], ended, "the reply's last row of the conference up to an hour is the span the table held at that hour");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    for world in [sampled as fn(i32) -> World, unsampled] {
        let (mut unbroken, mut writing) = (testing::store(), writers());
        let mut expected = Vec::new();
        for at in 0..=3 {
            run(&mut unbroken, &mut writing, at, &world(at));
            expected.push((testing::rows(&unbroken, TableId::PeaceOutcomes), testing::rows(&unbroken, TableId::PeaceConferenceSpans)));
        }
        let (mut stopped, mut before) = (testing::store(), writers());
        for at in 0..=1 {
            run(&mut stopped, &mut before, at, &world(at));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, 1);
        for at in 2..=3 {
            run(&mut resumed, &mut after, at, &world(at));
            assert_eq!((testing::rows(&resumed, TableId::PeaceOutcomes), testing::rows(&resumed, TableId::PeaceConferenceSpans)), expected[at as usize], "hour {at}");
        }
    }
}

#[test]
fn its_state_survives_a_save_and_a_load() {
    let (mut store, mut writers) = (testing::store(), writers());
    for at in 0..=1 {
        run(&mut store, &mut writers, at, &sampled(at));
    }
    let saved = writers.save().unwrap();
    let mut again = self::writers();
    let mut resumed = testing::resumed(&store, TEXTS);
    again.load(&saved, &mut resumed).unwrap();
    assert_eq!(again.save().unwrap(), saved, "an open conference's before, the previous read and the tags write back alike");
}
