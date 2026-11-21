
use crate::{Answer, Identity, Query, State, answer_at};
use dashboard_declare::{Carry, Column, Key, Measure, Series};
use dashboard_query::{
    identity::{Holder, HolderRun, Holders},
    series::{Line, SeriesRead, SeriesState},
    timeline::{Join, Stretch, Timeline},
};
use playhead::{
    changes::Changes,
    lane::Lane,
    pages::{Pages, Span, WorldPages},
};
use replay::{campaign::Replay, spool::Record};
use replay_columnar::{envelope::UNIT_ORDINALS, summary::Cells};
use replay_data::{
    identity::{CaptureMode, Identity as Recorded, Roster, SampleBoundary, SessionStart, Verification},
    rows::{CampaignRow, CountryLinkRow, CountryResourceRow, CountryRow, TableId, TableRows},
};
use replay_frame::{
    branches::{BranchReason, BranchStart, Parentage},
    frame::{DecodedFrame, FrameEncoder, FrameInput, Metrics, TableView},
};
use std::sync::{Arc, OnceLock};

const FIRST: i32 = 500;
const LAST: i32 = FIRST + 2 * UNIT_ORDINALS as i32 + 99;
const ITA_FROM: i32 = FIRST + 300;

fn exists(id: u32, hour: i32) -> bool {
    id == 9001 || hour >= ITA_FROM
}

fn power(id: u32, hour: i32) -> Option<i64> {
    match hour.rem_euclid(29) {
        3..=5 if id == 9001 => None,
        at => Some(i64::from(at) * 777 - i64::from(id - 9000) * 5_000 + i64::from(hour / 50)),
    }
}

fn resources(id: u32, hour: i32) -> Vec<u32> {
    match id {
        9001 if (FIRST + 900..FIRST + 1_200).contains(&hour) => vec![1, 2, 3],
        9001 => vec![1, 2],
        _ if exists(id, hour) => vec![2],
        _ => Vec::new(),
    }
}

fn resource(id: u32, resource: u32, hour: i32) -> CountryResourceRow {
    let step = i64::from(hour / (3 + resource as i32));
    CountryResourceRow { country: id, resource, special_total: i64::from(resource) * 9_000 - (step % 13) * 500, production_demand: -(step % 7) * 1_100, ..CountryResourceRow::default() }
}

fn frame(hour: i32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside the fixture".into() });
    tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: b"follow".to_vec() }]));
    let mut countries = Vec::new();
    let mut links = Vec::new();
    for (id, slot, tag) in [(9001, 7, b"GER"), (9002, 8, b"ITA")] {
        if exists(id, hour) {
            countries.push(CountryRow { id, canonical_slot: slot, tag: tag.to_vec(), political_power_raw: power(id, hour), command_power_raw: 0, ..CountryRow::default() });
            links.push(CountryLinkRow { id, canonical_slot: slot });
        }
    }
    let rows = [9001, 9002].into_iter().flat_map(|id| resources(id, hour).into_iter().map(move |held| resource(id, held, hour))).collect();
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
    tables[TableId::CountryResources.index()] = TableView::Complete(TableRows::CountryResources(rows));
    DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
}

fn replay() -> &'static Replay {
    static HELD: OnceLock<Replay> = OnceLock::new();
    HELD.get_or_init(|| {
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Recorded {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
            },
        };
        let mut records = vec![
            Record::SessionStart(session),
            Record::BranchStart(BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: FIRST, parent: Parentage::Unknown, reason: BranchReason::SessionStart }),
        ];
        let mut encoder = FrameEncoder::new();
        for hour in FIRST..=LAST {
            let mut input = FrameInput::from(frame(hour));
            input.scope_to(CaptureMode::Full);
            records.push(Record::Frame(encoder.encode_input(input, false).unwrap()));
        }
        Replay::from_records(b"follow".to_vec(), &[records]).unwrap()
    })
}

fn query() -> dashboard_query::Query<'static> {
    let timeline = Timeline { id: 1, first: FIRST, last: LAST, stretches: vec![Stretch { run: 0, first: FIRST, last: LAST, join: Join::Start }], unheld: Vec::new() };
    let run = |first: i32, slot: u32, id: u32| vec![HolderRun { first, last: LAST, holder: Holder::Found { slot, id }, starts_segment: true }];
    let holders = Holders::new([("GER".to_owned(), run(FIRST, 7, 9001)), ("ITA".to_owned(), run(ITA_FROM, 8, 9002))].into(), Vec::new());
    dashboard_query::Query { replay: replay(), timeline: Arc::new(timeline), holders: Arc::new(holders) }
}

const POWER: usize = 2;
const COMMAND: usize = 3;

fn negated(cells: &dyn Cells, slot: u32) -> Option<i64> {
    Some(cells.number(COMMAND, slot)? - cells.number(POWER, slot)?)
}

static NEGATED: Series = Series {
    kind: "test-negated",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure {
        name: "negated",
        column: Column::Computed { id: 900, inputs: &["political_power_raw", "command_power_raw"], value: negated },
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
};

const SPECIAL_TOTAL: usize = 4;
const PRODUCTION_DEMAND: usize = 5;

fn surplus(cells: &dyn Cells, slot: u32) -> Option<i64> {
    cells.number(SPECIAL_TOTAL, slot)?.checked_add(cells.number(PRODUCTION_DEMAND, slot)?)
}

static RESOURCE_SURPLUS: Series = Series {
    kind: "test-surplus",
    table: TableId::CountryResources,
    key: Key::Country,
    parts: Some(dashboard_declare::Parts { bits: 32, id: Some((0, 32)) }),
    measures: &[Measure {
        name: "surplus",
        column: Column::Computed { id: 901, inputs: &["special_total", "production_demand"], value: surplus },
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
};

fn lane(pages: &Arc<dyn Pages>, query: &Query, columns: Vec<usize>, from: i32) -> Lane {
    let mut lane = Lane::new(Arc::clone(pages), 0, query.table(), &columns, false).unwrap();
    lane.seek(from).unwrap();
    lane.fetch_through(i32::MAX).unwrap();
    lane
}

fn rows(answer: &Answer) -> Vec<(u128, Option<i64>, String)> {
    (0..answer.keys.len()).map(|row| (answer.keys[row], (answer.states[row] == State::Value).then_some(answer.values[row]), answer.name(row).unwrap_or_default().to_owned())).collect()
}

fn value(state: &SeriesState) -> Option<i64> {
    state.run.as_ref().map(|run| run.value_raw.parse().unwrap())
}

#[test]
fn a_followed_computed_column_equals_series_state_at_every_step() {
    assert_eq!(dashboard_declare::field(TableId::Countries, "political_power_raw").unwrap(), POWER);
    assert_eq!(dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap(), COMMAND);
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let followed = Query::Series { series: &NEGATED, measure: &NEGATED.measures[0], countries: vec!["GER".into(), "ITA".into()], top: None, least: false, parts: false, identity };
    let line = Line::new(&state, &NEGATED, &NEGATED.measures[0]).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let edge = FIRST + UNIT_ORDINALS as i32;
    let (mut valued, mut unrecorded) = (0, 0);
    for (from, to) in [(ITA_FROM - 30, ITA_FROM + 60), (edge - 40, edge + 40)] {
        let mut lane = lane(&pages, &followed, vec![POWER, COMMAND], from);
        let mut changes = Changes::default();
        for hour in from..=to {
            if hour > from {
                assert_eq!(lane.step(hour, &mut changes).unwrap(), hour);
            }
            let answer = Answer::parse(&answer_at(&followed, &lane).unwrap()).unwrap();
            let states = line.states(&[SeriesRead { country: "GER".into(), hour, part: None }, SeriesRead { country: "ITA".into(), hour, part: None }], None).unwrap();
            for (row, state) in states.iter().enumerate() {
                let held = rows(&answer)[row].1;
                assert_eq!(held, value(state), "{} at {hour}: {state:?}", state.country);
                let id = 9001 + row as u32;
                assert_eq!(held, exists(id, hour).then(|| power(id, hour).map(|power| -power)).flatten(), "{} at {hour}", state.country);
                valued += usize::from(held.is_some());
                unrecorded += usize::from(held.is_none() && exists(id, hour));
            }
        }
    }
    assert!(valued > 200 && unrecorded > 4, "{valued} values and {unrecorded} hours not recorded compared");
}

#[test]
fn a_followed_owners_parts_equal_series_state_of_every_part_at_every_step() {
    let state = query();
    let series = &RESOURCE_SURPLUS;
    let measure = &series.measures[0];
    let parts = series.parts.unwrap();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let line = Line::new(&state, series, measure).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let columns = dashboard_query::read::columns_of(&dashboard_declare::source(series.table, measure).unwrap());
    let mut compared = 0;
    for top in [None, Some(2)] {
        let followed = Query::Series { series, measure, countries: vec!["GER".into(), "ITA".into()], top, least: false, parts: true, identity: identity.clone() };
        for (from, to) in [(FIRST + 880, FIRST + 920), (FIRST + 1_180, FIRST + 1_220), (ITA_FROM - 5, ITA_FROM + 5)] {
            let mut lane = lane(&pages, &followed, columns.clone(), from);
            let mut changes = Changes::default();
            for hour in from..=to {
                if hour > from {
                    assert_eq!(lane.step(hour, &mut changes).unwrap(), hour);
                }
                let answer = Answer::parse(&answer_at(&followed, &lane).unwrap()).unwrap();
                let states = line.states(&[SeriesRead { country: "GER".into(), hour, part: None }, SeriesRead { country: "ITA".into(), hour, part: None }], top).unwrap();
                let wanted: Vec<(u128, Option<i64>, String)> = states
                    .iter()
                    .map(|state| {
                        let id = if state.country.tag() == Some("GER") { 9001u128 } else { 9002 };
                        let key = state.part.as_ref().map_or(0, |part| parts.key(id, part.parse().unwrap()));
                        (key, value(state), state.country.to_string())
                    })
                    .collect();
                assert_eq!(rows(&answer), wanted, "top {top:?} at {hour}");
                compared += wanted.len();
            }
        }
    }
    assert!(compared > 300, "{compared} rows compared");
}
