use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::Writers,
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryFactoriesRow, CountryRow, GroupRow, GroupSpanRow as Span, Recorded, TerritoryTotalRow};

const FACTION: u64 = 401;
const NUMBER: u32 = 3;
const ONE: i64 = 100_000;

struct At {
    members: &'static [u32],
    incomplete: bool,
    states: &'static [(u32, Option<i64>)],
    military: &'static [(u32, i64)],
}

const fn id(slot: u32) -> u32 {
    9000 + slot
}

fn feed(store: &mut Store, hour: i32, at: &At) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    let countries: Vec<CountryRow> = [7, 8, 9].map(|slot| CountryRow { id: id(slot), canonical_slot: slot, ..Default::default() }).into();
    put(store, hour, TableId::Countries, &countries, Status::Complete);
    let mut spans = vec![Span { kind: Span::PRESENT, group: NUMBER, value: 0, epoch: 0, ordinal: 0 }];
    spans.extend(at.members.iter().enumerate().map(|(at, &slot)| Span { kind: Span::MEMBER, group: NUMBER, value: slot, epoch: 0, ordinal: at as u32 }));
    spans.push(Span { kind: Span::MEMBER, group: Span::HUMANS, value: 9, epoch: 0, ordinal: 0 });
    if at.incomplete {
        spans.push(Span { kind: Span::INCOMPLETE, group: NUMBER, value: 0, epoch: 0, ordinal: 0 });
    }
    put(store, hour, TableId::GroupSpans, &spans, Status::Complete);
    put(store, hour, TableId::Groups, &[GroupRow { number: NUMBER, faction: FACTION }], Status::Complete);
    let totals: Vec<TerritoryTotalRow> = at
        .states
        .iter()
        .map(|&(slot, states)| TerritoryTotalRow { slot, controlled_states: states.map_or(Recorded::NotRecorded, Recorded::Value), ..Default::default() })
        .collect();
    put(store, hour, TableId::TerritoryTotals, &totals, Status::Complete);
    let factories: Vec<CountryFactoriesRow> =
        at.military.iter().map(|&(slot, built)| CountryFactoriesRow { country: id(slot), military_total_raw: built * ONE, ..Default::default() }).collect();
    put(store, hour, TableId::CountryFactories, &factories, Status::Complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(FactionTotals::default())]);
    writers.start_run();
    writers
}

fn shown(store: &Store) -> Option<(Option<i64>, Option<i64>)> {
    let row = cells(store, TableId::FactionTotals, u128::from(FACTION))?;
    let at = |name: &str| (0..TableId::FactionTotals.field_count()).find(|&field| TableId::FactionTotals.field_name(field) == Some(name)).unwrap();
    Some((row[at("controlled_states")], row[at("built1")]))
}

const HOURS: &[At] = &[
    At { members: &[7, 8], incomplete: false, states: &[(7, Some(5)), (8, Some(3)), (9, Some(40))], military: &[(7, 5), (8, 3), (9, 90)] },
    At { members: &[7, 8], incomplete: false, states: &[(7, Some(5)), (8, Some(4)), (9, Some(40))], military: &[(7, 5), (8, 3), (9, 90)] },
    At { members: &[7], incomplete: false, states: &[(7, Some(5)), (8, Some(4)), (9, Some(41))], military: &[(7, 5), (8, 3), (9, 91)] },
    At { members: &[7], incomplete: true, states: &[(7, Some(5)), (8, Some(4)), (9, Some(41))], military: &[(7, 5), (8, 3), (9, 91)] },
    At { members: &[7, 8], incomplete: false, states: &[(7, None), (8, Some(4)), (9, Some(41))], military: &[(7, 6), (8, 3), (9, 91)] },
    At { members: &[], incomplete: false, states: &[(7, Some(5)), (8, Some(4)), (9, Some(41))], military: &[(7, 6), (8, 3), (9, 91)] },
];

#[test]
fn a_factions_total_adds_its_members_at_each_hour_and_is_not_recorded_where_one_is_not() {
    let mut store = store();
    let mut writers = writers();
    let expected = [Some((Some(8), Some(8))), Some((Some(9), Some(8))), Some((Some(5), Some(5))), Some((None, None)), Some((None, Some(9))), None];
    for (hour, at) in HOURS.iter().enumerate() {
        feed(&mut store, hour as i32, at);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        assert_eq!(shown(&store), expected[hour], "hour {hour}");
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let run = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, at) in HOURS.iter().enumerate() {
            feed(&mut store, hour as i32, at);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::FactionTotals));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(FactionTotals::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = run(None);
    for resume in 0..HOURS.len() - 1 {
        assert_eq!(run(Some(resume)), whole, "resumed after hour {resume}");
    }
}
