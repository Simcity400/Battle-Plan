
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
    absent_none: false,
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
fn a_followed_computed_column_equals_series_state_at_every_step() {
    assert_eq!(dashboard_declare::field(TableId::Countries, "political_power_raw").unwrap(), POWER);
    assert_eq!(dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap(), COMMAND);
    let state = query();
    let identity = Identity { holders: Arc::clone(&state.holders), timeline: Arc::clone(&state.timeline) };
    let followed = Query::Series { series: &NEGATED, measure: &NEGATED.measures[0], countries: vec!["GER".into(), "ITA".into()], top: None, least: false, parts: false, identity };
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
    let Ok(Query::Series { countries, top: None, .. }) = asked(serde_json::json!("every")) else { panic!("every holder's tag, unranked") };
    let tags: Vec<_> = countries.iter().map(|country| country.tag().expect("named by its tag, as a top names its rows")).collect();
    assert_eq!(tags, ["GER", "ITA"]);
    assert!(asked(serde_json::json!("some")).is_err(), "reads names every country or a list");
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
        let query = Query::Series { series, measure, countries: vec!["GER".into()], top: None, least: false, parts: false, identity };

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
    let followed = Query::Series { series, measure, countries: countries.clone(), top: None, least: false, parts: false, identity };
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
