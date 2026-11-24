
use crate::{Answer, Identity, Query, SeriesMode, State, answer_at};
use dashboard_declare::{Carry, Column, Key, Measure, Series};
use dashboard_query::{
    coverage::CoverageScope,
    identity::{Holder, HolderRun, Holders},
    series::{Line, SeriesRead, SeriesState},
    timeline::{Join, Stretch, Timeline},
};
use playhead::{
    changes::Changes,
    lane::Lane,
    pages::{Pages, Span, WorldPages},
};
use replay::campaign::Replay;
use replay_capture_spool::spool::Record;
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
        Replay::from_records(recorder_derive::writers(), b"follow".to_vec(), &[records]).unwrap()
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

fn negated(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
    replay_columnar::summary::Computed::of(cells.number(COMMAND, slot).zip(cells.number(POWER, slot)).map(|(command, power)| command - power))
}

fn ratio_of_power(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
    let Some(power) = cells.number(POWER, slot) else {
        return replay_columnar::summary::Computed::Unrecorded;
    };
    let Some(command) = cells.number(COMMAND, slot) else {
        return replay_columnar::summary::Computed::Unrecorded;
    };
    if command <= 0 {
        return replay_columnar::summary::Computed::NotApplicable("No command power");
    }
    replay_columnar::summary::Computed::Value(power / command)
}

static RATIO_OF_POWER: Series = Series {
    kind: "test-ratio",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure {
        name: "ratio",
        column: Column::Computed { id: 902, inputs: &["political_power_raw", "command_power_raw"], value: ratio_of_power },
        decimals: 0,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }],
    drops: false,
    absent_none: false,
};

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
    absent_none: false,
};

const SPECIAL_TOTAL: usize = 4;
const PRODUCTION_DEMAND: usize = 5;

fn surplus(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
    replay_columnar::summary::Computed::of(cells.number(SPECIAL_TOTAL, slot).zip(cells.number(PRODUCTION_DEMAND, slot)).and_then(|(a, b)| a.checked_add(b)))
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
    absent_none: false,
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
fn a_followed_computed_column_distinguishes_not_applicable_from_unrecorded() {
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let followed = Query::Series { series: &RATIO_OF_POWER, measure: &RATIO_OF_POWER.measures[0], countries: vec!["GER".into()], mode: SeriesMode::Keyed(CoverageScope::Table), identity };
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let recorded = (FIRST..ITA_FROM).find(|&hour| power(9001, hour).is_some()).unwrap();
    let unrecorded = (FIRST..ITA_FROM).find(|&hour| power(9001, hour).is_none()).unwrap();
    let lane_recorded = lane(&pages, &followed, vec![POWER, COMMAND], recorded);
    let answer_recorded = Answer::parse(&answer_at(&followed, &lane_recorded).unwrap()).unwrap();
    assert_eq!(answer_recorded.states, [State::Missing]);
    assert_eq!(answer_recorded.reason(0), Some("No command power"));
    let lane_unrecorded = lane(&pages, &followed, vec![POWER, COMMAND], unrecorded);
    let answer_unrecorded = Answer::parse(&answer_at(&followed, &lane_unrecorded).unwrap()).unwrap();
    assert_eq!(answer_unrecorded.states, [State::Unrecorded]);
    assert_eq!(answer_unrecorded.reason(0), Some("GER's ratio was not recorded at this hour"));
}

#[test]
fn a_followed_computed_column_equals_series_state_at_every_step() {
    assert_eq!(dashboard_declare::field(TableId::Countries, "political_power_raw").unwrap(), POWER);
    assert_eq!(dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap(), COMMAND);
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let followed = Query::Series { series: &NEGATED, measure: &NEGATED.measures[0], countries: vec!["GER".into(), "ITA".into()], mode: SeriesMode::Keyed(CoverageScope::Table), identity };
    let line = Line::new(&state, &NEGATED, &NEGATED.measures[0]).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
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
fn a_followed_series_reads_every_tag_its_holders_name_as_its_top_names_them() {
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let asked = |reads: serde_json::Value| crate::parse::parse(&serde_json::json!({ "product": "series-state",
        "params": { "branchId": "1", "family": { "kind": "country-factories", "measure": "free7" }, "reads": reads } }), &identity);
    let Ok(Query::Series { countries, mode: SeriesMode::Keyed(CoverageScope::Table), .. }) = asked(serde_json::json!("every")) else { panic!("every holder's tag, unranked") };
    let tags: Vec<_> = countries.iter().map(|country| country.tag().expect("named by its tag, as a top names its rows")).collect();
    assert_eq!(tags, ["GER", "ITA"]);
    assert!(asked(serde_json::json!("some")).is_err(), "reads names every country or a list");
}

#[test]
fn a_followed_top_of_named_countries_ranks_only_them_as_series_state_does() {
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let series = dashboard_declare::series("country-measure").unwrap();
    let measure = series.measure("politicalPower").unwrap();
    let asked = |reads: serde_json::Value| crate::parse::parse(&serde_json::json!({ "product": "series-state",
        "params": { "branchId": "1", "family": { "kind": "country-measure", "measure": "politicalPower" }, "reads": reads, "top": 2 } }), &identity);
    let Ok(Query::Series { countries, mode: SeriesMode::Top { n: 2, least: false, population: false }, .. }) = asked(serde_json::json!([{ "country": "ITA" }])) else {
        panic!("a top of named countries is a ranking of those countries")
    };
    assert_eq!(countries.len(), 1);
    let both = asked(serde_json::json!([{ "country": "GER" }, { "country": "ITA" }])).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let line = Line::new(&state, series, measure).unwrap();
    let everyone = Query::series_top(series, measure, 2, identity.clone());
    let (mut compared, mut refused) = (0, 0);
    for hour in (ITA_FROM..ITA_FROM + 60).step_by(7).chain((ITA_FROM..ITA_FROM + 60).filter(|&hour| power(9001, hour).is_none()).take(2)) {
        let answer = Answer::parse(&answer_at(&both, &lane(&pages, &both, both.declared(), hour)).unwrap()).unwrap();
        let population = Answer::parse(&answer_at(&everyone, &lane(&pages, &everyone, everyone.declared(), hour)).unwrap()).unwrap();
        let reads: Vec<SeriesRead> = ["GER", "ITA"].iter().map(|tag| SeriesRead { country: (*tag).into(), hour, part: None }).collect();
        let mut expected: Vec<(i64, String)> = line.states(&reads, None).unwrap().into_iter()
            .filter_map(|state| Some((state.run?.value_raw.parse::<i64>().ok()?, state.country.to_string()))).collect();
        if expected.len() < 2 {
            let unrecorded = Some("GER's politicalPower was not recorded at this hour".to_owned());
            assert_eq!((answer.not_recorded.clone(), population.not_recorded.clone()), (unrecorded.clone(), unrecorded), "an unrecorded contender refuses both tops at {hour}, never drops out");
            refused += 1;
            continue;
        }
        expected.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let named: Vec<&str> = (0..answer.keys.len()).filter_map(|row| answer.name(row)).collect();
        assert_eq!(named, expected.iter().map(|(_, tag)| tag.as_str()).collect::<Vec<_>>(), "hour {hour}");
        assert_eq!(Ok(named.iter().map(|tag| dashboard_query::identity::Country::from(*tag)).collect::<Vec<_>>()), line.rank_at(hour, 2, false).unwrap(), "the followed top is the exact rank at {hour}");
        compared += 1;
    }
    assert!(compared > 3 && refused == 2, "{compared} hours ranked both countries, {refused} refused");
}

#[test]
fn a_followed_owners_parts_equal_series_state_of_every_part_at_every_step() {
    let state = query();
    let series = &RESOURCE_SURPLUS;
    let measure = &series.measures[0];
    let parts = series.parts.unwrap();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let line = Line::new(&state, series, measure).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let columns = dashboard_query::read::columns_of(&dashboard_declare::source(series.table, measure).unwrap());
    let mut compared = 0;
    for top in [None, Some(2)] {
        let followed = Query::Series { series, measure, countries: vec!["GER".into(), "ITA".into()], mode: SeriesMode::Parts { top }, identity: identity.clone() };
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

#[test]
fn a_followed_supply_health_withholds_v1_and_admits_v2_across_step_and_backward_seek() {
    use crate::{Follow, Message, Sink, StepStats, decode};
    use playhead::{Playhead, Settings as PlaySettings, Timeline as PlayTimeline};
    use recorder_derive_framework::framework::{
        Definitions,
        definitions::{Define, Form},
    };
    use replay_recording::{records::Records, columnar_frames::Recording, schema::schema};
    use replay_journal::{columnar_file::{FileWriter, HourWriter, Resumed}, synthetic::{Input, Sample, Synthetic}};
    use replay_settings::compression::Settings;
    use replay_columnar::{
        container::{self, SectionKey, Tags},
        envelope::{self, Kind},
    };
    use replay_data::{
        format::FormatVersion,
        identity::IdentityCodec,
        rows::{ClockRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow, Recorded as RecCell, TABLE_COUNT},
    };
    use std::{io::Cursor, sync::mpsc};

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
    let (first_v1, last_v1, first_v2, last_v2) = (100, 111, 112, 123);
    let branch = BranchStart {
        id: 1,
        session: 1,
        loaded_hour: None,
        first_hour: first_v1,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    };
    let records = Records { sessions: vec![session.clone()], branches: vec![branch], ..Records::default() };
    let kinds = schema().unwrap();
    let mut content = Vec::new();
    session.identity.encode(&mut content).unwrap();
    let identity_env = envelope::Identity {
        schema: envelope::schema_digest(&kinds),
        campaign: b"follow-mixed".to_vec(),
        content,
        capture_mode: CaptureMode::Full.byte(),
    };
    let definitions = || {
        Definitions::read_none()
            .with_define(Define { section: "NMilitary", name: "SUPPLY_GRACE", form: Form::Int32 }, Ok(72))
            .with_define(Define { section: "NMilitary", name: "LOW_SUPPLY", form: Form::Fixed5 }, Ok(50_000))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(15_500))
            .with_define(Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: Form::Fixed5 }, Ok(35_000))
    };
    let campaign = [CampaignRow { id: b"follow-mixed".to_vec() }];
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..Default::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    let divisions = [DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, province: 1, ..Default::default() }];
    let land = [DivisionLandInputsRow { division: 1, naval_transfer: false, ..Default::default() }];
    let statuses = [DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_grace_maximum: RecCell::Value(7_200_000),
        supply_grace_remaining: RecCell::Value(0),
        supply_ratio_exact: RecCell::Value(Some(15_000)),
        supply_grace_exact: RecCell::Value(Some(0)),
        ..Default::default()
    }];
    let write_hours = |writer: &mut HourWriter<Cursor<Vec<u8>>>, start: i32, end: i32| {
        writer.start_run(1, envelope::epoch::START, start).unwrap();
        let mut synthetic = Synthetic::new(1, start).unwrap();
        for hour in start..=end {
            synthetic.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..Default::default() }], partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Campaign, Input::Snapshot { rows: &campaign, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &land, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.table(TableId::DivisionStatus, Input::Snapshot { rows: &statuses, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
            synthetic.end_encoded(|manifest, sections| writer.hour(manifest, sections)).unwrap().unwrap();
        }
    };

    let key = SectionKey::derived(TableId::SupplyHealth.index() as u32);

    let file = FileWriter::create(Cursor::new(Vec::new()), identity_env.clone(), TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v1, last_v1);
    let mut file = writer.finish().unwrap();
    let encoded = records.encode(FormatVersion::CURRENT).unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let v1_marked = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
    let (extent, length) = container::locate(v1_marked.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(v1_marked.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut stripped = container::ContainerWriter::resume(Cursor::new(v1_marked), root, at, 1, None).unwrap();
    stripped.commit(None, true, |_| Ok(())).unwrap();
    let v1_bytes = stripped.into_inner().into_inner();
    let (extent, length) = container::locate(v1_bytes.as_slice(), container::Open::Newest).unwrap().unwrap();
    let root = container::read_root(v1_bytes.as_slice(), &extent).unwrap();
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let file = FileWriter::resume(Cursor::new(v1_bytes), Resumed { root, at, last: Some((1, last_v1)), open: None }, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v2, last_v2);
    let two_run_bytes = writer.finish().unwrap().finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();

    let v1_run_pages = WorldPages::over(Recording::open(two_run_bytes.clone()).unwrap(), Span::Run(0)).unwrap();
    let held_v1_run = crate::host::held_tables(&v1_run_pages, TABLE_COUNT);
    assert!(!held_v1_run[TableId::SupplyHealth.index()], "pure v1 run of mixed recording must not be held by dashboard-follow");

    let file = FileWriter::create(Cursor::new(Vec::new()), identity_env, TABLE_COUNT, Settings { level: 1 }, None).unwrap();
    let mut writer = HourWriter::new(file, 1, recorder_derive::writers()).unwrap();
    writer.define(definitions());
    write_hours(&mut writer, first_v1, last_v2);
    let mut file = writer.finish().unwrap();
    let extent = file.container().put_zstd(Kind::Controls, Tags::default(), &encoded, 1).unwrap();
    file.container().append(SectionKey::RECORDS, extent).unwrap();
    let single = file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner();
    let (extent, length) = container::locate(single.as_slice(), container::Open::Newest).unwrap().unwrap();
    let mut root = container::read_root(single.as_slice(), &extent).unwrap();
    root.sections.retain(|section| section.key != key);
    let at = container::Revision { number: root.revision, length, root: extent.digest };
    let mut rewritten = container::ContainerWriter::resume(Cursor::new(single), root, at, 1, None).unwrap();
    rewritten.section(key, 2).unwrap();
    let mut payload = replay_columnar::leaf::Writer::new();
    payload.varint(0).unwrap();
    payload.varint(12).unwrap();
    let payload = payload.into_bytes();
    let extent = rewritten.put(Kind::Stored, Tags::default(), payload.len() as u64, &payload).unwrap();
    rewritten.append(key, extent).unwrap();
    rewritten.commit(None, true, |_| Ok(())).unwrap();
    let mid_run_bytes = rewritten.into_inner().into_inner();

    let series = dashboard_declare::series("supply-health").unwrap();
    let measure = series.measure("noSupply").unwrap();
    let run_follow = |bytes: Vec<u8>, stretches: Vec<Stretch>| {
        let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(Recording::open(bytes).unwrap(), Span::Whole).unwrap());
        let timeline = Arc::new(Timeline { id: 1, first: first_v1, last: last_v2, stretches, unheld: Vec::new() });
        let holders = Arc::new(Holders::new(
            [(
                "GER".to_owned(),
                vec![HolderRun { first: first_v1, last: last_v2, holder: Holder::Found { slot: 7, id: 9001 }, starts_segment: true }],
            )]
            .into(),
            Vec::new(),
        ));
        let identity = Identity { holders, timeline };
        let query = Query::Series { series, measure, countries: vec!["GER".into()], mode: SeriesMode::Keyed(CoverageScope::Table), identity };

        let follow = Follow::new(0);
        let (send, recv) = mpsc::channel();
        let sink: Sink = Box::new(move |stats: &StepStats, message: Option<Message>| {
            let _ = send.send((*stats, message));
        });
        let hook = follow.hook(sink);
        let settings = PlaySettings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 8, speed: 1.0, rate: 1.0, ..PlaySettings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(&pages), PlayTimeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
        let reader = playhead.declare(Vec::new(), None).unwrap();
        let mut consumer = playhead.consumer(reader);
        let weak = Arc::downgrade(&playhead);
        follow.bind(move |tables, _| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads()).unwrap();
        follow.follow(1, query).unwrap();

        let drive_to = |target: i32, seek: bool, consumer: &mut playhead::Consumer| -> Answer {
            if seek {
                playhead.seek(target);
            } else {
                playhead.play(1.0);
            }
            let mut last_answer: Option<Answer> = None;
            let mut steps = Vec::new();
            loop {
                consumer.advance(target);
                consumer.take(target, &mut steps);
                let (stats, message) = recv.recv().unwrap();
                if let Some(message) = message {
                    let decoded = decode(&message.bytes).unwrap();
                    for &(id, _, body) in &decoded.answers {
                        if id == 1 {
                            last_answer = Some(Answer::parse(body).unwrap());
                        }
                    }
                }
                if stats.last == target {
                    return last_answer.unwrap_or_else(|| panic!("no answer emitted when reaching target hour {target} (seek={seek})"));
                }
            }
        };

        let at_v1 = drive_to(last_v1, true, &mut consumer);
        assert!(
            at_v1.not_recorded.as_deref().is_some_and(|why| why.contains("SupplyHealth")),
            "dashboard-follow must withhold last v1 hour {last_v1}: {at_v1:?}"
        );

        let at_v2 = drive_to(first_v2, false, &mut consumer);
        assert_eq!(at_v2.not_recorded, None, "dashboard-follow must admit first v2 hour {first_v2}: {at_v2:?}");
        assert_eq!(rows(&at_v2), vec![(7, Some(1), "GER".to_owned())]);

        let back_v1 = drive_to(last_v1, true, &mut consumer);
        assert!(
            back_v1.not_recorded.as_deref().is_some_and(|why| why.contains("SupplyHealth")),
            "dashboard-follow backward seek must withhold v1 hour {last_v1}: {back_v1:?}"
        );
    };

    run_follow(
        mid_run_bytes,
        vec![Stretch { run: 0, first: first_v1, last: last_v2, join: Join::Start }],
    );
    run_follow(
        two_run_bytes,
        vec![
            Stretch { run: 0, first: first_v1, last: last_v1, join: Join::Start },
            Stretch { run: 1, first: first_v2, last: last_v2, join: Join::Break },
        ],
    );
}

#[test]
fn golden_slot_and_tag_answers_match_checked_in_fixtures_and_board_reasons() {
    use dashboard_query::{identity::Country, series::board};
    let state = query();
    let series = dashboard_declare::series("country-measure").unwrap();
    let measure = series.measure("politicalPower").unwrap();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let countries = vec![Country::Slot(7), Country::Tag("ITA".into())];
    let followed = Query::Series { series, measure, countries: countries.clone(), mode: SeriesMode::Keyed(CoverageScope::Table), identity };
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap());

    let lane_525 = lane(&pages, &followed, vec![POWER], 525);
    let bytes_525 = answer_at(&followed, &lane_525).unwrap();
    assert_eq!(bytes_525.as_slice(), include_bytes!("../fixtures/series_slot_tag_525.bin").as_slice());

    let lane_800 = lane(&pages, &followed, vec![POWER], ITA_FROM);
    let bytes_800 = answer_at(&followed, &lane_800).unwrap();
    assert_eq!(bytes_800.as_slice(), include_bytes!("../fixtures/series_slot_tag_800.bin").as_slice());

    let line = Line::new(&state, series, measure).unwrap();
    let cols = [board::Column { line, partial: None }];
    let board_525 = board::at(&cols, board::Rows::Countries(&countries), 525).unwrap();
    assert_eq!(board_525.rows.len(), 1);
    assert_eq!(board_525.rows[0].country, Country::Slot(7));
    assert_eq!(board_525.rows[0].tag, "GER");
    assert_eq!(board_525.rows[0].cells[0].missing.as_deref(), Some("GER's politicalPower was not recorded at this hour"));

    let parsed_525 = Answer::parse(&bytes_525).unwrap();
    assert_eq!(parsed_525.reason(0), board_525.rows[0].cells[0].missing.as_deref());
}

#[test]
fn top_ranking_missing_reasons_match_exact_board_for_unavailable_and_not_recorded_tables() {
    use dashboard_query::series::board;
    use replay_data::rows::{ClockRow, CountryFactoriesRow};
    use replay_journal::synthetic::{Input, Sample};

    let factories = dashboard_declare::series("country-factories").unwrap();
    let free1 = factories.measure("free1").unwrap();

    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let followed_unavailable = Query::series_top(factories, free1, 2, identity);
    let pages: Arc<dyn Pages> = Arc::new(
        WorldPages::over(replay_recording::columnar_frames::Recording::open(replay().bytes().to_vec()).unwrap(), Span::Whole).unwrap(),
    );
    let unavailable_lane = lane(&pages, &followed_unavailable, followed_unavailable.declared(), ITA_FROM);
    let unavailable_answer = Answer::parse(&answer_at(&followed_unavailable, &unavailable_lane).unwrap()).unwrap();
    let unavailable_cols = [board::Column { line: Line::new(&state, factories, free1).unwrap(), partial: None }];
    let unavailable_board = board::at(&unavailable_cols, board::Rows::Top { by: 0, n: 2, least: false }, ITA_FROM).unwrap();
    assert!(unavailable_board.rows.is_empty());
    assert!(unavailable_answer.keys.is_empty());
    assert_eq!(unavailable_board.missing.as_deref(), Some("Country factories not available at this hour: outside the fixture"));
    assert_eq!(unavailable_answer.not_recorded, unavailable_board.missing);

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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: FIRST, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"follow-unrecorded".to_vec() };
    let unrecorded_replay = Replay::from_synthetic(
        recorder_derive::writers(),
        campaign.id.clone(),
        session,
        branch,
        FIRST,
        |frame, hour| {
            frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
            frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
            frame.table(TableId::Countries, Input::Snapshot {
                rows: &[CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }],
                partial: None,
                read: true,
                sample: Sample::FromReadBit,
            })?;
            frame.table(TableId::CountryLinks, Input::Snapshot {
                rows: &[CountryLinkRow { id: 9001, canonical_slot: 7 }],
                partial: None,
                read: true,
                sample: Sample::FromReadBit,
            })?;
            frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::NotRecorded { read: false, sample: Sample::FromReadBit })
        },
    )
    .unwrap();
    let timeline = Arc::new(Timeline { id: 1, first: FIRST, last: FIRST, stretches: vec![Stretch { run: 0, first: FIRST, last: FIRST, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(Holders::new(
        [("GER".to_owned(), vec![HolderRun { first: FIRST, last: FIRST, holder: Holder::Found { slot: 7, id: 9001 }, starts_segment: true }])].into(),
        Vec::new(),
    ));
    let unrecorded_state = dashboard_query::Query { replay: &unrecorded_replay, timeline: Arc::clone(&timeline), holders: Arc::clone(&holders) };
    let unrecorded_identity = Identity { holders, timeline };
    let followed_unrecorded = Query::series_top(factories, free1, 2, unrecorded_identity);
    let unrecorded_pages: Arc<dyn Pages> = Arc::new(
        WorldPages::over(replay_recording::columnar_frames::Recording::open(unrecorded_replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap(),
    );
    let unrecorded_lane = lane(&unrecorded_pages, &followed_unrecorded, followed_unrecorded.declared(), FIRST);
    let unrecorded_answer = Answer::parse(&answer_at(&followed_unrecorded, &unrecorded_lane).unwrap()).unwrap();
    let unrecorded_cols = [board::Column { line: Line::new(&unrecorded_state, factories, free1).unwrap(), partial: None }];
    let unrecorded_board = board::at(&unrecorded_cols, board::Rows::Top { by: 0, n: 2, least: false }, FIRST).unwrap();
    assert!(unrecorded_board.rows.is_empty());
    assert!(unrecorded_answer.keys.is_empty());
    assert_eq!(unrecorded_board.missing.as_deref(), Some("Country factories not recorded at this hour"));
    assert_eq!(unrecorded_answer.not_recorded, unrecorded_board.missing);
}

#[test]
fn follow_and_live_cells_apply_witness_rule_and_invalidate_across_swapped_truncated_hours() {
    use crate::{
        Follow, Message, Rows, Sink, StepStats, decode,
        live::{CellValue, Cells as LiveCells, NO_ROW, Source as LiveSource},
    };
    use dashboard_query::series::board;
    use playhead::{Playhead, Settings as PlaySettings, Timeline as PlayTimeline};
    use replay_data::rows::{ClockRow, CountryExperienceRow, CountryFactoriesRow};
    use replay_frame::frame::{DecodedFrame, Metrics, TableView};
    use replay_journal::synthetic::{Input, Sample};
    use std::sync::mpsc;

    const T_BASE: i32 = 120;
    const T_LAST: i32 = 143;
    const T1: i32 = 128;
    const T2: i32 = 129;
    const T3: i32 = 134;
    const T_UNAVAIL: i32 = 139;
    const T_NOTREC: i32 = 141;
    const EXP_TRUNC: &str = "Country experience read only in part at this hour (1 rows not read)";
    const EXP_TRUNC_TABLE: &str = "Country experience read only in part at this hour (1 rows not read), and which rows were read is not held at the head";
    const EXP_UNAVAIL: &str = "Country experience not available at this hour: reader fault";
    const EXP_NOTREC: &str = "Country experience not recorded at this hour";
    const RES_TRUNC_TABLE: &str = "Country resources read only in part at this hour (1 rows not read), and which rows were read is not held at the head";

    fn exp_ratio(cells: &dyn Cells, slot: u32) -> replay_columnar::summary::Computed {
        let Some(army) = cells.number(0, slot) else {
            return replay_columnar::summary::Computed::Unrecorded;
        };
        let Some(navy) = cells.number(3, slot) else {
            return replay_columnar::summary::Computed::Unrecorded;
        };
        if navy <= 0 {
            return replay_columnar::summary::Computed::NotApplicable("No navy experience");
        }
        replay_columnar::summary::Computed::Value(army / navy)
    }

    static RATIO_EXP: Series = Series {
        kind: "test-follow-exp-ratio",
        table: TableId::CountryExperience,
        key: Key::Country,
        parts: None,
        measures: &[Measure {
            name: "ratio",
            column: Column::Computed { id: 911, inputs: &["army_raw", "navy_raw"], value: exp_ratio },
            decimals: 0,
            aggregates: 0,
            carry: Carry::None,
            rank: 0,
        }],
        drops: false,
        absent_none: false,
    };

    static ABSENT_ZERO_EXP: Series = Series {
        kind: "test-follow-exp-zero",
        table: TableId::CountryExperience,
        key: Key::Country,
        parts: None,
        measures: &[Measure {
            name: "army",
            column: Column::Cell("army_raw"),
            decimals: 5,
            aggregates: 0,
            carry: Carry::None,
            rank: 0,
        }],
        drops: false,
        absent_none: true,
    };

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
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: T_BASE, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"follow-truncated".to_vec() };
    let countries = [
        CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() },
        CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() },
        CountryRow { id: 9003, canonical_slot: 9, tag: b"FRA".to_vec(), ..CountryRow::default() },
    ];
    let links = [
        CountryLinkRow { id: 9001, canonical_slot: 7 },
        CountryLinkRow { id: 9002, canonical_slot: 8 },
        CountryLinkRow { id: 9003, canonical_slot: 9 },
    ];
    let exp_ger = CountryExperienceRow { country: 9001, army_raw: 100, navy_raw: 0, ..CountryExperienceRow::default() };
    let exp_ita = CountryExperienceRow { country: 9002, army_raw: 200, navy_raw: 10, ..CountryExperienceRow::default() };
    let fac_ger = CountryFactoriesRow { country: 9001, military_total_raw: 1_000_000, ..CountryFactoriesRow::default() };
    let fac_ita = CountryFactoriesRow { country: 9002, military_total_raw: 2_000_000, ..CountryFactoriesRow::default() };
    let res_ger1 = CountryResourceRow { country: 9001, resource: 1, produced: 110_000, ..CountryResourceRow::default() };
    let res_ger2 = CountryResourceRow { country: 9001, resource: 2, produced: 220_000, ..CountryResourceRow::default() };

    let replay = Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session, branch, T_LAST, |frame, hour| {
        frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit })?;
        frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial: None, read: true, sample: Sample::FromReadBit })?;
        match hour {
            T1 => {
                frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ger), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryFactories, Input::Snapshot { rows: std::slice::from_ref(&fac_ger), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryResources, Input::Snapshot { rows: std::slice::from_ref(&res_ger1), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
            }
            T2 => {
                frame.table(TableId::CountryExperience, Input::Snapshot { rows: std::slice::from_ref(&exp_ita), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryFactories, Input::Snapshot { rows: std::slice::from_ref(&fac_ita), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryResources, Input::Snapshot { rows: std::slice::from_ref(&res_ger2), partial: Some(1), read: true, sample: Sample::FromReadBit })?;
            }
            T3 => {
                frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone()], partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryFactories, Input::Snapshot { rows: &[fac_ger.clone(), fac_ita.clone()], partial: Some(1), read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryResources, Input::Snapshot { rows: &[res_ger1.clone(), res_ger2.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
            }
            T_UNAVAIL => {
                frame.table::<CountryExperienceRow>(TableId::CountryExperience, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
                frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
                frame.table::<CountryResourceRow>(TableId::CountryResources, Input::Unavailable { reason: "reader fault", read: false, sample: Sample::FromReadBit })?;
            }
            T_NOTREC => {
                frame.table::<CountryExperienceRow>(TableId::CountryExperience, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
                frame.table::<CountryFactoriesRow>(TableId::CountryFactories, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
                frame.table::<CountryResourceRow>(TableId::CountryResources, Input::NotRecorded { read: false, sample: Sample::FromReadBit })?;
            }
            _ => {
                frame.table(TableId::CountryExperience, Input::Snapshot { rows: &[exp_ger.clone(), exp_ita.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryFactories, Input::Snapshot { rows: &[fac_ger.clone(), fac_ita.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
                frame.table(TableId::CountryResources, Input::Snapshot { rows: &[res_ger1.clone(), res_ger2.clone()], partial: None, read: true, sample: Sample::FromReadBit })?;
            }
        }
        Ok(())
    })
    .unwrap();

    let timeline = Arc::new(Timeline { id: 1, first: T_BASE, last: T_LAST, stretches: vec![Stretch { run: 0, first: T_BASE, last: T_LAST, join: Join::Start }], unheld: Vec::new() });
    let holders = Arc::new(dashboard_query::identity::holders(replay.recording(), &timeline).unwrap());
    let state = dashboard_query::Query { replay: &replay, timeline: Arc::clone(&timeline), holders: Arc::clone(&holders) };
    let identity = Identity { holders: Arc::clone(&holders), timeline: Arc::clone(&timeline) };
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());

    let exp_series = dashboard_declare::series("country-experience").unwrap();
    let army = exp_series.measure("army").unwrap();
    let tags = vec!["GER".into(), "ITA".into(), "FRA".into()];
    let q_scalar = Query::Series { series: exp_series, measure: army, countries: tags.clone(), mode: SeriesMode::Keyed(CoverageScope::Row), identity: identity.clone() };
    let q_ratio = Query::Series { series: &RATIO_EXP, measure: &RATIO_EXP.measures[0], countries: tags.clone(), mode: SeriesMode::Keyed(CoverageScope::Row), identity: identity.clone() };
    let q_zero = Query::Series { series: &ABSENT_ZERO_EXP, measure: &ABSENT_ZERO_EXP.measures[0], countries: tags.clone(), mode: SeriesMode::Keyed(CoverageScope::Row), identity: identity.clone() };
    let q_table_selected = crate::parse::parse(
        &serde_json::json!({
            "product": "series-state",
            "params": {
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": "army" },
                "reads": [{ "country": "GER" }, { "country": "ITA" }],
            },
        }),
        &identity,
    )
    .unwrap();
    let q_row_selected = crate::parse::parse(
        &serde_json::json!({
            "product": "series-state",
            "coverage": "row",
            "params": {
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": "army" },
                "reads": [{ "country": "GER" }, { "country": "ITA" }],
            },
        }),
        &identity,
    )
    .unwrap();
    for (label, bad) in [
        (
            "series-state top",
            serde_json::json!({
                "product": "series-state",
                "coverage": "row",
                "params": {
                    "branchId": "1",
                    "family": { "kind": "country-experience", "measure": "army" },
                    "top": 2,
                },
            }),
        ),
        (
            "series-state reads: every",
            serde_json::json!({
                "product": "series-state",
                "coverage": "row",
                "params": {
                    "branchId": "1",
                    "family": { "kind": "country-experience", "measure": "army" },
                    "reads": "every",
                },
            }),
        ),
        (
            "army-size top with countries",
            serde_json::json!({
                "product": "army-size",
                "coverage": "row",
                "params": {
                    "branchId": "1",
                    "countries": ["GER", "ITA"],
                    "top": 1,
                    "measure": "divisions",
                    "countFor": "owner",
                    "byType": false,
                },
            }),
        ),
        (
            "air-force-size top with countries",
            serde_json::json!({
                "product": "air-force-size",
                "coverage": "row",
                "params": {
                    "branchId": "1",
                    "countries": ["GER", "ITA"],
                    "top": 1,
                    "measure": "wings",
                    "ownership": "effective",
                },
            }),
        ),
        (
            "navy-size top with countries",
            serde_json::json!({
                "product": "navy-size",
                "coverage": "row",
                "params": {
                    "branchId": "1",
                    "countries": ["GER", "ITA"],
                    "top": 1,
                    "measure": "strength",
                    "ownership": "logical",
                },
            }),
        ),
    ] {
        assert!(crate::parse::parse(&bad, &identity).is_err(), "{label} must reject coverage: row");
    }
    let q_top = Query::series_top(exp_series, army, 2, identity.clone());
    let q_all = Query::Cells { table: TableId::CountryExperience.index(), columns: vec![0], rows: Rows::All };
    let q_keys = Query::Cells { table: TableId::CountryExperience.index(), columns: vec![0], rows: Rows::Keys(vec![9001, 9002, 9003]) };
    let res_series = dashboard_declare::series("country-resources").unwrap();
    let q_parts = Query::Series { series: res_series, measure: res_series.measure("produced").unwrap(), countries: vec!["GER".into()], mode: SeriesMode::Parts { top: None }, identity };

    let line_scalar = Line::new(&state, exp_series, army).unwrap();
    let line_ratio = Line::new(&state, &RATIO_EXP, &RATIO_EXP.measures[0]).unwrap();
    let line_zero = Line::new(&state, &ABSENT_ZERO_EXP, &ABSENT_ZERO_EXP.measures[0]).unwrap();
    let board_cols = [
        board::Column { line: Line::new(&state, exp_series, army).unwrap(), partial: None },
        board::Column { line: Line::new(&state, &RATIO_EXP, &RATIO_EXP.measures[0]).unwrap(), partial: None },
        board::Column { line: Line::new(&state, &ABSENT_ZERO_EXP, &ABSENT_ZERO_EXP.measures[0]).unwrap(), partial: None },
    ];

    for hour in [T_BASE, T1, T2, T3, T_UNAVAIL, T_NOTREC] {
        let l = lane(&pages, &q_scalar, vec![0, 3], hour);
        let a_scalar = Answer::parse(&answer_at(&q_scalar, &l).unwrap()).unwrap();
        let a_ratio = Answer::parse(&answer_at(&q_ratio, &l).unwrap()).unwrap();
        let a_zero = Answer::parse(&answer_at(&q_zero, &l).unwrap()).unwrap();
        let a_top = Answer::parse(&answer_at(&q_top, &l).unwrap()).unwrap();
        let a_all = Answer::parse(&answer_at(&q_all, &l).unwrap()).unwrap();
        let a_keys = Answer::parse(&answer_at(&q_keys, &l).unwrap()).unwrap();
        let reads: Vec<SeriesRead> = tags.iter().map(|c| SeriesRead { country: c.clone(), hour, part: None }).collect();
        let exact_scalar = line_scalar.states(&reads, None).unwrap();
        let exact_ratio = line_ratio.states(&reads, None).unwrap();
        let exact_board = board::at(&board_cols, board::Rows::Countries(&tags), hour).unwrap();

        if hour == T_UNAVAIL || hour == T_NOTREC {
            let expected = if hour == T_UNAVAIL { EXP_UNAVAIL } else { EXP_NOTREC };
            assert_eq!(a_scalar.not_recorded.as_deref(), Some(expected));
            assert_eq!(a_top.not_recorded.as_deref(), Some(expected));
            assert_eq!(a_all.not_recorded.as_deref(), Some(expected));
            assert_eq!(a_keys.not_recorded.as_deref(), Some(expected));
            continue;
        }
        assert_eq!(a_scalar.not_recorded, None, "hour {hour}");
        assert_eq!(a_keys.not_recorded, None, "hour {hour}");
        for row in 0..3 {
            let followed_val = (a_scalar.states[row] == State::Value).then_some(a_scalar.values[row]);
            assert_eq!(followed_val, value(&exact_scalar[row]), "scalar parity row {row} hour {hour}");
            assert_eq!(a_scalar.reason(row), exact_scalar[row].missing.as_deref(), "scalar reason parity row {row} hour {hour}");
            assert_eq!(a_scalar.reason(row), exact_board.rows[row].cells[0].missing.as_deref(), "board parity row {row} hour {hour}");
            assert_eq!(a_ratio.reason(row), exact_ratio[row].missing.as_deref(), "ratio reason parity row {row} hour {hour}");
            assert_eq!(a_zero.reason(row), exact_board.rows[row].cells[2].missing.as_deref(), "absent_none reason parity row {row} hour {hour}");
        }
        if hour == T1 {
            assert_eq!(a_scalar.states, [State::Value, State::NoRow, State::NoRow]);
            assert_eq!(a_scalar.reason(1), Some(EXP_TRUNC));
            assert_eq!(a_ratio.states, [State::Missing, State::NoRow, State::NoRow]);
            assert_eq!(a_ratio.reason(0), Some("No navy experience"));
            assert_eq!(a_ratio.reason(1), Some(EXP_TRUNC));
            assert_eq!(a_zero.states, [State::Value, State::NoRow, State::NoRow]);
            assert_eq!(a_zero.reason(2), Some(EXP_TRUNC));
            assert_eq!(a_keys.states, [State::Value, State::NoRow, State::NoRow]);
            assert_eq!(a_keys.reason(1), Some(EXP_TRUNC));
            assert_eq!(a_top.not_recorded.as_deref(), Some(EXP_TRUNC_TABLE));
            assert_eq!(a_all.not_recorded.as_deref(), Some(EXP_TRUNC_TABLE));
            let a_table_selected = Answer::parse(&answer_at(&q_table_selected, &l).unwrap()).unwrap();
            assert_eq!(
                a_table_selected.not_recorded.as_deref(),
                Some(EXP_TRUNC_TABLE),
                "default Table-scoped selected-group follow refuses the whole answer on a truncated hour"
            );
            let a_row_selected = Answer::parse(&answer_at(&q_row_selected, &l).unwrap()).unwrap();
            assert_eq!(a_row_selected.not_recorded, None);
            assert_eq!(a_row_selected.states, [State::Value, State::NoRow]);
            assert_eq!(a_row_selected.values[0], 100);
            assert_eq!(a_row_selected.reason(1), Some(EXP_TRUNC));
            let res_lane = lane(&pages, &q_parts, q_parts.declared(), hour);
            let a_parts = Answer::parse(&answer_at(&q_parts, &res_lane).unwrap()).unwrap();
            assert_eq!(a_parts.not_recorded.as_deref(), Some(RES_TRUNC_TABLE));
        }
        if hour == T3 {
            let a_table_selected = Answer::parse(&answer_at(&q_table_selected, &l).unwrap()).unwrap();
            assert_eq!(
                a_table_selected.not_recorded.as_deref(),
                Some(EXP_TRUNC_TABLE),
                "default Table-scoped follow still refuses when asked keys are witnessed but another row was unread"
            );
            let a_row_selected = Answer::parse(&answer_at(&q_row_selected, &l).unwrap()).unwrap();
            assert_eq!(a_row_selected.not_recorded, None);
            assert_eq!(a_row_selected.states, [State::Value, State::Value]);
        }
        if hour == T_BASE {
            assert_eq!(a_zero.states, [State::Value, State::Value, State::Value]);
            assert_eq!(a_zero.values[2], 0);
            assert_eq!(exact_board.rows[2].cells[2].value_raw.as_deref(), Some("0"));
            let exact_zero = line_zero.states(&reads, None).unwrap();
            assert_eq!(exact_zero[2].run.as_ref().map(|run| run.value_raw.as_str()), Some("0"), "an absent row of an absent-is-none series reads zero, as followed and on the board");
            assert_eq!(a_keys.states, [State::Value, State::Value, State::NoRow]);
            assert_eq!(a_keys.reason(2), Some(NO_ROW));
        }
        if hour == T1 {
            let exact_zero = line_zero.states(&reads, None).unwrap();
            assert_eq!(exact_zero[0].run.as_ref().map(|r| r.value_raw.as_str()), Some("100"));
            assert_eq!(exact_zero[2].missing.as_deref(), Some(EXP_TRUNC));
        }
    }

    let cells = LiveCells::default();
    cells.value("ger", "army", LiveSource { table: TableId::CountryExperience, column: 0, key: 9001 }, 1.0);
    cells.value("ita", "army", LiveSource { table: TableId::CountryExperience, column: 0, key: 9002 }, 1.0);
    cells.value("fra", "army", LiveSource { table: TableId::CountryExperience, column: 0, key: 9003 }, 1.0);
    let live = cells.finish();
    let mut trunc_frame = DecodedFrame { branch: 1, hour: T1, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() };
    trunc_frame.tables[TableId::CountryExperience.index()] = TableView::Truncated { rows: TableRows::CountryExperience(vec![exp_ger]), missing: 1 }.into();
    assert_eq!(
        live.values(&trunc_frame)[0],
        vec![CellValue::Number(100), CellValue::Missing(EXP_TRUNC.into()), CellValue::Missing(EXP_TRUNC.into())]
    );

    let follow = Follow::new(0);
    let (send, recv) = mpsc::channel();
    let sink: Sink = Box::new(move |stats: &StepStats, message: Option<Message>| {
        let _ = send.send((*stats, message));
    });
    let hook = follow.hook(sink);
    let settings = PlaySettings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 8, speed: 1.0, rate: 1.0, ..PlaySettings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages), PlayTimeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
    let reader = playhead.declare(Vec::new(), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    let weak = Arc::downgrade(&playhead);
    follow.bind(move |tables, _| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads()).unwrap();
    follow.follow(1, q_scalar).unwrap();
    follow.follow(2, Query::Cells { table: TableId::CountryExperience.index(), columns: vec![0], rows: Rows::Live(vec![9001, 9002]) }).unwrap();

    let mut held: Option<Vec<u8>> = None;
    let mut drive = |target: i32, seek: bool| -> (Answer, Answer) {
        if seek {
            playhead.seek(target);
        } else {
            playhead.play(1.0);
        }
        let mut steps = Vec::new();
        loop {
            consumer.advance(target);
            consumer.take(target, &mut steps);
            let (stats, message) = recv.recv().unwrap();
            if let Some(message) = message {
                held = Some(match held.take() {
                    Some(older) if decode(&older).unwrap().stamp == decode(&message.bytes).unwrap().stamp => {
                        crate::answer::merge(&older, &message.bytes).unwrap().0
                    }
                    _ => message.bytes,
                });
            }
            if stats.last == target {
                let decoded = decode(held.as_deref().unwrap()).unwrap();
                let one = Answer::parse(decoded.answers.iter().find(|&&(id, ..)| id == 1).unwrap().2).unwrap();
                let two = Answer::parse(decoded.answers.iter().find(|&&(id, ..)| id == 2).unwrap().2).unwrap();
                return (one, two);
            }
        }
    };

    let (s_127, _) = drive(T1 - 1, true);
    assert_eq!(s_127.states, [State::Value, State::Value, State::NoRow]);
    let (s_t1, l_t1) = drive(T1, false);
    assert_eq!(s_t1.states, [State::Value, State::NoRow, State::NoRow]);
    assert_eq!(s_t1.reason(1), Some(EXP_TRUNC));
    assert_eq!(l_t1.states, [State::Value, State::NoRow]);
    assert_eq!(l_t1.reason(1), Some(EXP_TRUNC));
    let (s_t2, l_t2) = drive(T2, false);
    assert_eq!(s_t2.states, [State::NoRow, State::Value, State::NoRow], "swapping witnesses across consecutive truncated hours must invalidate followed series");
    assert_eq!(s_t2.reason(0), Some(EXP_TRUNC));
    assert_eq!(l_t2.states, [State::NoRow, State::Value], "swapping witnesses across consecutive truncated hours must invalidate followed live cells");
    assert_eq!(l_t2.reason(0), Some(EXP_TRUNC));

    let table_follow = Follow::new(0);
    let (t_send, t_recv) = mpsc::channel();
    let t_sink: Sink = Box::new(move |stats: &StepStats, message: Option<Message>| {
        let _ = t_send.send((*stats, message));
    });
    let t_hook = table_follow.hook(t_sink);
    let t_settings = PlaySettings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 8, speed: 1.0, rate: 1.0, ..PlaySettings::default() };
    let t_playhead = Arc::new(Playhead::new(Arc::clone(&pages), PlayTimeline::whole(&*pages).unwrap(), t_settings, Some(t_hook)).unwrap());
    let t_reader = t_playhead.declare(Vec::new(), None).unwrap();
    let mut t_consumer = t_playhead.consumer(t_reader);
    let t_weak = Arc::downgrade(&t_playhead);
    table_follow.bind(move |tables, _| t_weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(t_reader, tables)), t_playhead.log_reads()).unwrap();
    table_follow.follow(1, q_top).unwrap();
    table_follow.follow(2, q_table_selected).unwrap();

    let mut drive_table = |target: i32, seek: bool| -> (StepStats, Option<Message>) {
        if seek {
            t_playhead.seek(target);
        } else {
            t_playhead.play(1.0);
        }
        let mut steps = Vec::new();
        loop {
            t_consumer.advance(target);
            t_consumer.take(target, &mut steps);
            let (stats, message) = t_recv.recv().unwrap();
            if stats.last == target {
                return (stats, message);
            }
        }
    };

    let _ = drive_table(T1 - 1, true);
    let (t1_stats, _) = drive_table(T1, false);
    assert!(t1_stats.evaluated > 0, "entering truncated T1 evaluates Table-scoped subscriptions");
    let (t2_stats, t2_msg) = drive_table(T2, false);
    assert_eq!(t2_stats.touched, 0, "swapping witnesses across T1 -> T2 must not touch Table-only subscriptions");
    assert_eq!(t2_stats.evaluated, 0, "swapping witnesses across T1 -> T2 must perform zero evaluations for Table-only subscriptions");
    assert_eq!(t2_stats.sent, 0, "swapping witnesses across T1 -> T2 must send zero answers for Table-only subscriptions");
    assert_eq!(t2_msg.as_ref().map(|m| m.answers), Some(0), "clock-advance message carries zero answers");
}
