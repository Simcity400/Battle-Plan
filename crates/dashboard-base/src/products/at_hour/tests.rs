use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use dashboard_follow::{
    Answer, State,
    live::{CellValue, NO_ROW, NO_VALUE, NOT_RECORDED, Source as Cell},
};
use playhead::{
    lane::Lane,
    pages::{Pages, Span, WorldPages},
};
use replay::campaign::{Replay, SeekResult};
use replay_data::rows::{CountryLinkRow, CountryResourceRow, CountryRow, TableRows, tables};
use replay_frame::frame::TableView;
use serde::Deserialize;
use serde_json::json;

struct Power;

#[derive(Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PowerParams {
    branch_id: String,
    country: Country,
}

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct PowerReply {
    slot: u32,
    rows: Vec<String>,
    warnings: Vec<String>,
}

crate::reply_warnings!(PowerReply);

fn streamed(data: &PanelData, fresh: bool) -> PanelData {
    data.clone().map(|mut reply| {
        for group in reply["live"]["groups"].as_array_mut().into_iter().flatten() {
            if !fresh && let Some(group) = group.as_object_mut() { group.remove("values"); group.remove("rawValues"); }
        }
        reply
    })
}

fn column(table: TableId, name: &str) -> usize {
    dashboard_declare::field(table, name).unwrap()
}

impl AtHour for Power {
    const KEY: &'static str = "test-power";
    const TITLE: &'static str = "Power";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    type Params = PowerParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId, country];
    type Reply = PowerReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries, TableId::CountryLinks]
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match (step, owner) {
            (0, _) => vec![Read::all(TableId::CountryLinks)],
            (1, Some(owner)) => {
                assert!(held.complete::<tables::CountryLinks>().is_ok(), "the first step's rows are held at the second");
                vec![Read::these(TableId::Countries, [u128::from(owner.id)])]
            }
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.expect("the adapter resolved the holder");
        let links = frame.complete::<tables::CountryLinks>()?;
        let power = column(TableId::Countries, "political_power_raw");
        let coal = u128::from(owner.id) << 32 | 1;
        let mut rows = Vec::new();
        for link in links {
            let row = link.id.to_string();
            at.live.value(&row, "power", Cell { table: TableId::Countries, column: power, key: link.id.into() }, 0.001);
            rows.push(row);
        }
        let surplus = |name| Cell { table: TableId::CountryResources, column: column(TableId::CountryResources, name), key: coal };
        at.live.sum(&owner.id.to_string(), "coal", &[surplus("special_total"), surplus("production_demand")], 1.0);
        Ok(PowerReply { slot: owner.slot, rows, warnings: Vec::new() })
    }
}

const FIRST: i32 = 100;
const LAST: i32 = 130;
const ITA_LEAVES: i32 = 115;
const RESOURCES_GONE: std::ops::RangeInclusive<i32> = 120..=122;

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, (hour != 125).then_some(i64::from(hour / 10) * 1_000));
    if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()]
        && hour < ITA_LEAVES
    {
        rows.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), political_power_raw: Some(i64::from(hour) * 7), ..CountryRow::default() });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()] {
        rows.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
    }
    if !RESOURCES_GONE.contains(&hour) {
        let coal = CountryResourceRow { country: 9001, resource: 1, special_total: i64::from(hour) * 100, production_demand: -i64::from(hour % 4), ..CountryResourceRow::default() };
        *frame.tables[TableId::CountryResources.index()] = TableView::Complete(TableRows::CountryResources(vec![coal]));
    }
    frame
}

fn recording() -> (Source, Arc<dyn Pages>) {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "at-hour".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    (source, pages)
}

fn load() -> LoadContext {
    LoadContext { content: None }
}

fn asked(source: &Source, request: Value) -> Result<Value, dashboard_core::QueryError> {
    let text = (entry::<Power>().registered.load)(source, "lease", request, &load(), &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

fn requested(source: &Source, country: impl Serialize, hour: i32) -> Result<Value, dashboard_core::QueryError> {
    asked(source, json!({ "branchId": "1", "country": country, "hour": hour }))
}

fn refused(error: dashboard_core::QueryError) -> String {
    match error {
        dashboard_core::QueryError::Refused { text } => text,
        dashboard_core::QueryError::Failed { message } => panic!("not a refusal: {message}"),
    }
}

fn followed(source: &Source, panel: &mut dyn Panel, hour: i32) -> PanelData {
    let tables = panel.tables().to_vec();
    let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, hour, Some(&tables)).unwrap() else { panic!("hour {hour} is recorded") };
    panel.data(&frame)
}

fn answered_cells(pages: &Arc<dyn Pages>, live: &Live, hour: i32) -> Vec<Vec<CellValue>> {
    live.groups
        .iter()
        .map(|group| {
            let mut lane = Lane::new(Arc::clone(pages), 0, group.table.index(), &group.columns, false).unwrap();
            lane.seek(hour).unwrap();
            let answer = Answer::parse(&dashboard_follow::answer_at(&group.query(), &lane).unwrap()).unwrap();
            if let Some(reason) = answer.not_recorded {
                return vec![CellValue::Missing(reason); group.keys.len() * group.columns.len()];
            }
            assert_eq!(answer.keys, group.keys);
            (0..answer.values.len())
                .map(|at| match answer.states[at] {
                    State::Value => CellValue::Number(answer.values[at]),
                    State::Missing => CellValue::Missing(NO_VALUE.into()),
                    State::Unrecorded => CellValue::Missing(NOT_RECORDED.into()),
                    State::NoRow => CellValue::Missing(answer.reason(at / answer.columns).unwrap().into()),
                })
                .collect()
        })
        .collect()
}

fn cell_json(value: &CellValue) -> Value {
    match value {
        CellValue::Number(number) => json!(number),
        CellValue::Missing(reason) => json!(reason),
    }
}

#[test]
fn a_rows_at_an_hour_product_answers_a_request_and_its_panel_with_one_reply() {
    let (source, pages) = recording();
    let mut panel = (entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "country": "GER" })).unwrap();
    assert_eq!(panel.tables(), [TableId::Countries, TableId::CountryLinks]);
    let (mut rowless, mut unavailable, mut valueless) = (0, 0, 0);
    for hour in FIRST..=LAST {
        let mut asked = requested(&source, "GER", hour).unwrap();
        let live = asked.as_object_mut().unwrap().remove("live").expect("a request's reply carries its live cells");
        assert_eq!(followed(&source, &mut *panel, hour), Ok(asked.clone()), "the panel carries the structure alone, hour {hour}");
        assert_eq!(asked["slot"], 7);
        assert_eq!(asked["rows"], json!(["11", "9001", "9002"]));
        let named = panel.live().expect("the panel names its live cells").clone();
        let pushed = named.json(&vec![dashboard_follow::live::GroupState::Follow(1); named.groups.len()]);
        for field in ["rows", "kinds", "cells"] {
            assert_eq!(pushed[field], live[field], "{field} at hour {hour}");
        }
        let cells = answered_cells(&pages, &named, hour);
        for (at, (group, values)) in live["groups"].as_array().unwrap().iter().zip(&cells).enumerate() {
            assert_eq!((&group["table"], &group["columns"], &group["keys"]), (&pushed["groups"][at]["table"], &pushed["groups"][at]["columns"], &pushed["groups"][at]["keys"]));
            for (cell, value) in group["values"].as_array().unwrap().iter().zip(values) {
                match (cell, value) {
                    (Value::String(theirs), CellValue::Missing(ours)) if !theirs.starts_with("No ") && theirs != NOT_RECORDED => {
                        let words = named.groups[at].table.words();
                        let said = format!("{}{} not available at this hour: outside country fixture", words[..1].to_uppercase(), &words[1..]);
                        assert!(*theirs == said && *ours == said, "the whole table is missing on both paths, hour {hour}: {theirs} / {ours}");
                        unavailable += 1;
                    }
                    _ => assert_eq!(cell, &cell_json(value), "the request's value is the followed cell's, hour {hour}"),
                }
                rowless += usize::from(*cell == NO_ROW);
                valueless += usize::from(*cell == NO_VALUE);
            }
        }
    }
    assert_eq!(rowless, (ITA_LEAVES - FIRST) as usize + 2 * (LAST - ITA_LEAVES + 1) as usize, "a cell whose row left is missing by name");
    assert_eq!(unavailable, 2 * RESOURCES_GONE.count(), "a cell whose table is unavailable is missing, never the value before");
    assert_eq!(valueless, 1, "a value not recorded is missing, never zero");
    let at = |hour| requested(&source, "GER", hour).unwrap()["live"]["groups"].clone();
    assert_eq!(at(125).as_array().unwrap().iter().flat_map(|group| group["values"].as_array().unwrap().clone()).filter(|value| *value == NO_VALUE).count(), 1);
    assert_eq!(requested(&source, 7, 110).unwrap(), requested(&source, "GER", 110).unwrap(), "a slot asks the country holding its tag");
    assert_eq!(followed(&source, &mut *panel, 101), followed(&source, &mut *panel, 128));
}

#[test]
fn a_refused_reply_is_the_same_named_refusal_on_both_paths() {
    let (source, _) = recording();
    let unheld = "Power country is unavailable at this hour";
    assert_eq!(refused(requested(&source, "FRA", 110).unwrap_err()), unheld);
    assert_eq!(refused(requested(&source, 8, ITA_LEAVES).unwrap_err()), unheld, "a country holding no tag at the hour is not recorded there");
    assert!(requested(&source, 8, ITA_LEAVES - 1).is_ok());
    let mut panel = (entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "country": "FRA" })).unwrap();
    assert_eq!(followed(&source, &mut *panel, 110), Err(unheld.to_owned()));
    assert!(panel.live().is_none(), "a refused reply names no cells");
    assert_eq!(refused(requested(&source, "GER", LAST + 50).unwrap_err()), "Power hour is not recorded");
    assert!(asked(&source, json!({ "branchId": "1", "country": "GER" })).unwrap_err().to_string().contains("names no hour"));
    assert!((entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "country": "GER", "hour": 3 })).is_err(), "a panel takes the head's hour");
}

struct Scoped;

#[derive(Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopedParams {
    branch_id: String,
}

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct ScopedReply {
    scoped: bool,
    warnings: Vec<String>,
}

crate::reply_warnings!(ScopedReply);

impl AtHour for Scoped {
    const KEY: &'static str = "test-scoped";
    const TITLE: &'static str = "Scoped";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    type Params = ScopedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = ScopedReply;
    type Context = ();

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(_frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(ScopedReply { scoped: engine_executable::executable_tables::current().is_some(), warnings: Vec::new() })
    }
}

#[test]
fn a_panel_made_on_the_panels_thread_reads_with_its_recordings_executable_tables_in_scope() {
    let executable = engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (source, _) = recording();
    let holders = Arc::clone(&crate::queries::query(&source, 1).unwrap().holders);
    let mut panel = Followed::<Scoped> {
        params: ScopedParams { branch_id: "1".into() },
        tables: vec![TableId::Countries],
        holders,
        context: (),
        content_warnings: Vec::new(),
        live: None,
        answered: None,
        executable: Some(executable),
        product: PhantomData,
    };
    let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, FIRST, Some(&[TableId::Countries])).unwrap() else { panic!("hour {FIRST} is recorded") };
    let data = std::thread::scope(|threads| threads.spawn(|| panel.data(&frame)).join().unwrap());
    assert_eq!(data.unwrap()["scoped"], true, "a panel reading recorded keywords or registries would find them withheld");
}

struct Static;

impl AtHour for Static {
    const KEY: &'static str = "test-static";
    const TITLE: &'static str = "Static";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    const REPLY_NAMES_NO_HOUR: bool = true;
    type Params = ScopedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = ScopedReply;
    type Context = std::sync::atomic::AtomicUsize;

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        Vec::new()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(0.into())
    }

    fn reply(_frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        at.context.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(ScopedReply { scoped: false, warnings: Vec::new() })
    }
}

#[test]
fn dated_read_in_a_gap_refuses_the_hour_between_two_recorded_stretches() {
    let segments = [(1, FIRST..=FIRST + 5), (2, FIRST + 10..=LAST)].map(|(branch, hours)| {
        records(branch, hours.map(|hour| DecodedFrame { branch, ..frame(hour) }).collect())
    });
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap().into_timelines(&Default::default()).unwrap();
    let token = AtomicBool::new(false);
    let source = Source::from_replay(crate::SourceKey { source_id: "gapped".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &token).unwrap();
    let panel = exact_panel::<Power>(&source, &load(), &json!({"panel": "test-power", "branchId":"1","country":"GER"})).unwrap();
    let exact = panel.exact().unwrap();
    assert!((exact.read)(FIRST + 5, &token).is_ok());
    assert_eq!((exact.read)(FIRST + 8, &token), Err("Power hour is not recorded".into()));
    assert!((exact.read)(FIRST + 10, &token).is_ok());
}

struct Planned;

#[derive(Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedParams {
    branch_id: String,
}

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct PlannedReply {
    tags: Vec<String>,
    power: Option<Vec<Option<i64>>>,
    hour: Option<i32>,
    warnings: Vec<String>,
}

crate::reply_warnings!(PlannedReply);

fn linked(plan: &[(Vec<Read>, i32)]) -> bool {
    plan.get(1).is_some_and(|(reads, _)| !reads.is_empty())
}

impl AtHour for Planned {
    const KEY: &'static str = "test-planned";
    const TITLE: &'static str = "Planned";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::Boundaries;
    type Params = PlannedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = PlannedReply;
    type Context = ();

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries, TableId::CountryLinks]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Countries)],
            1 if held.complete::<tables::Countries>().is_ok_and(|rows| rows.iter().any(|row| row.tag == b"ITA")) => vec![Read::all(TableId::CountryLinks)],
            _ => Vec::new(),
        }
    }

    fn projection(_params: &Self::Params, table: TableId, plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        (table == TableId::Countries && !linked(plan)).then(|| field_columns(table, &["canonical_slot", "tag"]))
    }

    fn clock_dependent(plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> bool {
        linked(plan)
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let countries = frame.complete::<tables::Countries>()?;
        let linked = frame.complete::<tables::CountryLinks>().is_ok();
        let power = column(TableId::Countries, "political_power_raw");
        for row in countries {
            at.live.value(&String::from_utf8_lossy(&row.tag), "power", Cell { table: TableId::Countries, column: power, key: row.id.into() }, 1.0);
        }
        Ok(PlannedReply {
            tags: countries.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect(),
            power: linked.then(|| countries.iter().map(|row| row.political_power_raw).collect()),
            hour: linked.then_some(at.hour),
            warnings: Vec::new(),
        })
    }
}

#[test]
fn a_followed_reply_reads_each_table_under_the_projection_its_final_plan_asks() {
    let (source, _) = recording();
    let panel = exact_panel::<Planned>(&source, &load(), &json!({"branchId": "1"})).unwrap();
    let reader = panel.exact().unwrap();
    let token = false.into();
    for hour in (FIRST..=LAST).chain([FIRST, ITA_LEAVES - 1, LAST]) {
        let read = (reader.advance)(hour, hour == FIRST, &token, None);
        assert_eq!(streamed(&read.value, read.fresh), streamed(&(reader.cold)(hour, &token), read.fresh), "hour {hour}");
        let value = read.value.as_ref().as_ref().unwrap();
        if hour < ITA_LEAVES {
            assert_eq!(value["hour"], hour, "a widened projection's power and hour, never the narrow one's defaults");
        } else {
            assert_eq!(value["power"], Value::Null);
        }
    }
}

struct Clocked;

const CLOCKED_FROM: i32 = 105;

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct ClockedReply {
    tags: Vec<String>,
    linked: bool,
    hour: i32,
    warnings: Vec<String>,
}

crate::reply_warnings!(ClockedReply);

impl AtHour for Clocked {
    const KEY: &'static str = "test-clocked";
    const TITLE: &'static str = "Clocked";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::EveryHour("test fixture reads CountryLinks starting at CLOCKED_FROM and stamps at.hour");
    type Params = PlannedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = ClockedReply;
    type Context = ();

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries, TableId::CountryLinks]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Countries)],
            1 if held.hour >= CLOCKED_FROM => vec![Read::all(TableId::CountryLinks)],
            _ => Vec::new(),
        }
    }

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        (table == TableId::Countries).then(|| field_columns(table, &["canonical_slot", "tag"]))
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(ClockedReply {
            tags: frame.complete::<tables::Countries>()?.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect(),
            linked: frame.complete::<tables::CountryLinks>().is_ok(),
            hour: at.hour,
            warnings: Vec::new(),
        })
    }
}

#[test]
fn a_reply_that_reads_the_hour_plans_its_stages_again_at_each_hour() {
    let (source, _) = recording();
    let panel = exact_panel::<Clocked>(&source, &load(), &json!({"branchId": "1"})).unwrap();
    let reader = panel.exact().unwrap();
    let token = false.into();
    for hour in FIRST..ITA_LEAVES {
        let read = (reader.advance)(hour, hour == FIRST, &token, None);
        assert_eq!(streamed(&read.value, read.fresh), streamed(&(reader.cold)(hour, &token), read.fresh), "hour {hour}: the links from hour {CLOCKED_FROM}, no table having changed");
    }
}

struct Dropping;

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct DroppingReply {
    tags: Vec<String>,
    coal: bool,
    warnings: Vec<String>,
}

crate::reply_warnings!(DroppingReply);

impl AtHour for Dropping {
    const KEY: &'static str = "test-dropping";
    const TITLE: &'static str = "Dropping";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    type Params = PlannedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = DroppingReply;
    type Context = AtomicBool;

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries, TableId::CountryResources]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Countries)],
            1 if held.complete::<tables::Countries>().is_ok_and(|rows| rows.iter().all(|row| row.political_power_raw.is_none())) => vec![Read::all(TableId::CountryResources)],
            _ => Vec::new(),
        }
    }

    fn projection(_params: &Self::Params, table: TableId, plan: &[(Vec<Read>, i32)], widened: &Self::Context) -> Option<Vec<usize>> {
        let coal = linked(plan);
        if coal { widened.store(true, std::sync::atomic::Ordering::Relaxed); }
        match table {
            TableId::Countries if !widened.load(std::sync::atomic::Ordering::Relaxed) => Some(field_columns(table, &["canonical_slot", "tag"])),
            TableId::CountryResources if coal => Some(field_columns(table, &["special_total"])),
            _ => None,
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(AtomicBool::new(false))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(DroppingReply {
            tags: frame.complete::<tables::Countries>()?.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect(),
            coal: frame.complete::<tables::CountryResources>().is_ok(),
            warnings: Vec::new(),
        })
    }
}

#[test]
fn a_table_the_final_plan_drops_is_released_never_read_again_and_again() {
    let (source, _) = recording();
    let panel = exact_panel::<Dropping>(&source, &load(), &json!({"branchId": "1"})).unwrap();
    let reader = panel.exact().unwrap();
    let token = false.into();
    for hour in FIRST..=FIRST + 2 {
        let read = (reader.advance)(hour, hour == FIRST, &token, None);
        assert_eq!(streamed(&read.value, read.fresh), streamed(&(reader.cold)(hour, &token), read.fresh), "hour {hour}");
        assert_eq!(read.value.as_ref().as_ref().unwrap()["coal"], false, "hour {hour}: coal left the final plan");
    }
}

struct Steady;

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct SteadyReply {
    tags: Vec<String>,
    warnings: Vec<String>,
}

crate::reply_warnings!(SteadyReply);

impl AtHour for Steady {
    const KEY: &'static str = "test-steady";
    const TITLE: &'static str = "Steady";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    type Params = PlannedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = SteadyReply;
    type Context = ();

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Countries]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let countries = frame.complete::<tables::Countries>()?;
        let power = column(TableId::Countries, "political_power_raw");
        for row in countries {
            at.live.value(&String::from_utf8_lossy(&row.tag), "power", Cell { table: TableId::Countries, column: power, key: row.id.into() }, 1.0);
        }
        Ok(SteadyReply { tags: countries.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect(), warnings: Vec::new() })
    }
}

#[test]
fn a_reply_made_again_the_same_keeps_the_data_it_made() {
    let (source, _) = recording();
    let panel = exact_panel::<Steady>(&source, &load(), &json!({"branchId": "1"})).unwrap();
    let reader = panel.exact().unwrap();
    let token = false.into();
    let first = (reader.advance)(FIRST, true, &token, None);
    assert!(first.fresh);
    assert_eq!(*first.value, (reader.cold)(FIRST, &token));
    let mut kept = first.value.clone();
    for hour in FIRST + 1..ITA_LEAVES {
        let read = (reader.advance)(hour, false, &token, None);
        assert!(Arc::ptr_eq(&read.value, &kept), "hour {hour}: the same tags, the data kept");
        assert!(!read.fresh, "hour {hour}: the kept data's values are another hour's");
        assert_eq!(streamed(&read.value, false), streamed(&(reader.cold)(hour, &token), false), "hour {hour}: the same tags");
        let whole = (reader.advance)(hour, true, &token, None);
        assert!(whole.fresh);
        assert_eq!(*whole.value, (reader.cold)(hour, &token), "hour {hour}: asked again at the same hour, its power current");
        kept = whole.value;
    }
    assert_eq!((reader.read)(ITA_LEAVES - 1, &token), (reader.cold)(ITA_LEAVES - 1, &token), "a complete read is current");
    let left = (reader.advance)(ITA_LEAVES, false, &token, None);
    assert!(!Arc::ptr_eq(&left.value, &first.value));
    assert!(left.fresh);
    assert_eq!(*left.value, (reader.cold)(ITA_LEAVES, &token), "another reply: its data made again, current");
    let sought = (reader.advance)(FIRST + 3, true, &token, None);
    assert_eq!(*sought.value, (reader.cold)(FIRST + 3, &token), "a seed is made whole");
}

#[test]
fn incremental_reads_include_cells_outside_the_staged_joins() {
    let (source, _) = recording();
    let params = json!({"branchId": "1", "country": "GER"});
    let panel = exact_panel::<Power>(&source, &load(), &params).unwrap();
    let other = exact_panel::<Power>(&source, &load(), &params).unwrap();
    let reader = panel.exact().unwrap();
    assert!(Arc::ptr_eq(&reader.advance, &other.exact().unwrap().advance));
    let token = false.into();
    for hour in (FIRST..=LAST).chain([FIRST, ITA_LEAVES, LAST]) {
        let read = (reader.advance)(hour, hour == FIRST, &token, None);
        let value = read.value;
        assert_eq!(streamed(&value, read.fresh), streamed(&(reader.cold)(hour, &token), read.fresh), "hour {hour}");
        let again = (other.exact().unwrap().advance)(hour, hour == FIRST, &token, None);
        assert!(Arc::ptr_eq(&again.value, &value));
        assert_eq!(again.rows, 0);
    }
}

#[test]
fn a_keyed_read_names_the_rows_it_chose() {
    let (source, _) = recording();
    let pool = placed::Pool::of(&source).unwrap();
    let mut count = 0;
    let run = pool.pages.run_of(0).unwrap();
    let mode = incremental::capture_mode(&pool.pages).unwrap();
    let reads = [Read::these(TableId::Countries, [9001, 9002]), Read::all(TableId::CountryLinks)];
    for hour in [FIRST, ITA_LEAVES] {
        let mut input = incremental::Input { pool: &pool, run, hour, mode, count: &mut count, step: None, placed: None };
        let frame = input.frame(&reads, |_| None, 1).unwrap();
        assert_eq!(*frame.tables[TableId::Countries.index()].keys(), Keys::These(vec![9001, 9002]), "hour {hour}");
        assert_eq!(*frame.tables[TableId::CountryLinks.index()].keys(), Keys::All, "hour {hour}");
    }
}

struct Stepped {
    hour: i32,
    rows: u64,
    fresh: bool,
    pending: bool,
    waited: bool,
    value: Arc<PanelData>,
    lanes: Vec<Declared>,
}

fn stepped(pages: &Arc<dyn Pages>, exact: &dashboard_follow::panel::AtHour, every: i32, join: i32, skip: usize) -> Vec<Stepped> {
    let (send, played) = std::sync::mpsc::channel();
    let advance = exact.advance.clone();
    let (mut due, mut first, mut seeded) = (0, true, false);
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        let read = (step.last() >= join).then(|| due += 1).filter(|_| first || (due - 1) % skip == 0).map(|()| {
            let token = AtomicBool::new(false);
            let mut read = advance(step.last(), first || step.reset(), &token, Some(dashboard_follow::panel::ProductStep { step, lanes, held: None, wake: None }));
            let waited = !seeded && read.pending;
            if waited { read = advance(step.last(), true, &token, None); }
            seeded = true;
            first = read.pending;
            Stepped { hour: step.last(), rows: read.rows, fresh: read.fresh, pending: read.pending, waited, value: read.value, lanes: read.lanes }
        });
        let _ = send.send((step.last(), read));
    });
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 1, ring: 3, speed: f64::from(every), rate: 1.0, ..playhead::Settings::default() };
    let playhead = playhead::Playhead::new(Arc::clone(pages), playhead::Timeline::whole(&**pages).unwrap(), settings, Some(hook)).unwrap();
    let mut declared = BTreeMap::<usize, Declared>::new();
    let reader = playhead.declare(declared.values().cloned().collect(), None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(FIRST);
    let (mut head, mut out) = (FIRST, Vec::new());
    loop {
        let mut next = None;
        playhead
            .drive_until(&mut consumer, head, || match played.try_recv() {
                Ok(item) => {
                    next = Some(item);
                    std::ops::ControlFlow::Break(Ok(()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => std::ops::ControlFlow::Continue(()),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    std::ops::ControlFlow::Break(Err("step hook channel disconnected".into()))
                }
            })
            .unwrap_or_else(|error| panic!("no step at head {head}: {error}"));
        let (last, read) = next.unwrap();
        if let Some(read) = read {
            let before = declared.clone();
            for lane in &read.lanes {
                let held = declared.entry(lane.table).or_insert_with(|| lane.clone());
                held.columns.extend(&lane.columns);
                held.columns.sort_unstable();
                held.columns.dedup();
            }
            if declared != before { playhead.redeclare(reader, declared.values().cloned().collect()).unwrap(); }
            out.push(read);
        }
        if last >= LAST { break; }
        if last >= head { head = (head + every).min(LAST); }
    }
    drop(consumer);
    playhead.finish();
    out
}

#[test]
fn a_followed_product_reads_its_tables_at_each_step_from_the_playheads_lanes() {
    let (source, pages) = recording();
    let token = AtomicBool::new(false);
    let cases = [(1, FIRST, 1), (3, FIRST, 1), (2, FIRST + 7, 1), (1, FIRST, 3), (4, ITA_LEAVES, 2)];
    for (every, join, skip) in cases {
        let panel = exact_panel::<Power>(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "country": "GER" })).unwrap();
        let exact = panel.exact().unwrap();
        let stepped = stepped(&pages, exact, every, join, skip);
        assert!(stepped[0].waited || (every, join, skip) != cases[0], "a table its step has no lane for, and no placed lane near, waits for one, never read cold beside the step");
        assert!(stepped.iter().skip(1).all(|read| !read.waited));
        let reads: Vec<&Stepped> = stepped.iter().filter(|read| !read.pending).collect();
        assert!(reads.len() > 2, "steps of {every} hours from {join}, every {skip}");
        let first: Vec<usize> = reads[0].lanes.iter().map(|lane| lane.table).collect();
        assert!([TableId::Countries, TableId::CountryLinks].iter().all(|table| first.contains(&table.index())), "a product's lanes are the tables its reads take: {first:?}");
        assert!(reads[0].lanes.iter().all(|lane| lane.keyed));
        for read in &reads {
            assert_eq!(streamed(&read.value, read.fresh), streamed(&(exact.cold)(read.hour, &token), read.fresh), "steps of {every} hours from {join}, every {skip}: hour {}", read.hour);
        }
        assert!(reads.iter().all(|read| read.lanes.iter().any(|lane| lane.table == TableId::CountryResources.index())), "a live cell no stage reads is read from a lane of its table");
    }
    for (every, join, skip) in cases {
        let panel = exact_panel::<Planned>(&source, &load(), &json!({ "branchId": "1" })).unwrap();
        let exact = panel.exact().unwrap();
        for read in stepped(&pages, exact, every, join, skip).into_iter().filter(|read| !read.pending) {
            assert_eq!(streamed(&read.value, read.fresh), streamed(&(exact.cold)(read.hour, &token), read.fresh), "planned, steps of {every} hours from {join}, every {skip}: hour {}", read.hour);
        }
    }
}

struct Partial;

#[derive(Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
struct PartialReply {
    keys: Vec<String>,
    missing: Option<u64>,
    warnings: Vec<String>,
}

crate::reply_warnings!(PartialReply);

fn partly_held(view: &TableView) -> (Vec<String>, Option<u64>) {
    let keys = |rows: &[CountryResourceRow]| rows.iter().map(|row| format!("{}:{}", row.country, row.resource)).collect();
    match view {
        TableView::Complete(TableRows::CountryResources(rows)) => (keys(rows), None),
        TableView::Truncated { rows: TableRows::CountryResources(rows), missing } => (keys(rows), Some(u64::from(*missing))),
        TableView::Unavailable { reason } => (vec![reason.clone()], None),
        _ => (vec!["neither rows nor a reason".into()], None),
    }
}

impl AtHour for Partial {
    const KEY: &'static str = "test-partial";
    const TITLE: &'static str = "Partial";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::InputsOnly;
    type Params = PlannedParams;
    const FIELDS: crate::products::Fields = crate::fields![branchId];
    type Reply = PartialReply;
    type Context = ();

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::CountryResources]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let (keys, missing) = partly_held(frame.tables[TableId::CountryResources.index()].held());
        Ok(PartialReply { keys, missing, warnings: Vec::new() })
    }
}

#[test]
fn a_table_read_in_part_is_the_rows_its_hour_read_on_every_path() {
    let mut partly = 110..=113;
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if partly.contains(&hour) {
                let coal = CountryResourceRow { country: 9001, resource: 1, special_total: i64::from(hour), ..CountryResourceRow::default() };
                let oil = CountryResourceRow { country: 9001, resource: 2, special_total: 5, ..CountryResourceRow::default() };
                let rows = if hour % 2 == 0 { vec![coal, oil] } else { vec![oil] };
                *frame.tables[TableId::CountryResources.index()] = TableView::Truncated { rows: TableRows::CountryResources(rows), missing: 3 };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "at-hour-partly".into(), revision: "1".into() }, "partly.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let recorded = |hour: i32| {
        let frame = replay_lanes::at::frame(source.replay_ref().recording(), 0, hour, &[replay_lanes::at::Read::all(TableId::CountryResources)]).unwrap();
        let (keys, missing) = partly_held(frame.tables[TableId::CountryResources.index()].held());
        (json!(keys), json!(missing))
    };
    let said = |value: &Value| (value["keys"].clone(), value["missing"].clone());
    assert!(partly.all(|hour| recorded(hour).1 == 3), "the recording reads those hours in part");
    let panel = exact_panel::<Partial>(&source, &load(), &json!({ "branchId": "1" })).unwrap();
    let exact = panel.exact().unwrap();
    let token = AtomicBool::new(false);
    for hour in FIRST..=LAST {
        let text = (entry::<Partial>().registered.load)(&source, "lease", json!({ "branchId": "1", "hour": hour }), &load(), &token).unwrap();
        assert_eq!(said(&serde_json::from_str::<Value>(&text).unwrap()["data"]), recorded(hour), "a request at hour {hour}");
        assert_eq!(said(&(exact.cold)(hour, &token).unwrap()), recorded(hour), "an exact read at hour {hour}");
    }
    for every in [1, 3] {
        for read in stepped(&pages, exact, every, FIRST, 1).into_iter().filter(|read| !read.pending) {
            assert_eq!(said(read.value.as_ref().as_ref().unwrap()), recorded(read.hour), "steps of {every} hours: hour {}", read.hour);
        }
    }
}

#[test]
fn at_hour_live_cells_apply_witness_rule_on_cold_and_incremental_across_truncated_swap() {
    struct CoalScopes;

    impl AtHour for CoalScopes {
        const KEY: &'static str = "test-coal-scopes";
        const TITLE: &'static str = "Test coal scopes";
        const CONTENT_DEPENDENT: bool = false;
        const CLOCK: Clock = Clock::InputsOnly;
        type Params = PowerParams;
        const FIELDS: crate::products::Fields = crate::fields![branchId, country];
        type Reply = PowerReply;
        type Context = ();

        fn validate(params: &Self::Params) -> Result<(), &'static str> {
            Power::validate(params)
        }

        fn owner(params: &Self::Params) -> Option<&Country> {
            Power::owner(params)
        }

        fn tables(_params: &Self::Params) -> Vec<TableId> {
            vec![TableId::CountryResources]
        }

        fn context(_source: &Source, _load: &LoadContext) -> io::Result<()> {
            Ok(())
        }

        fn reply(_frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, ()>) -> io::Result<PowerReply> {
            let owner = at.owner.expect("the adapter resolved the holder");
            let coal = u128::from(owner.id) << 32 | 1;
            let surplus = |name| Cell { table: TableId::CountryResources, column: column(TableId::CountryResources, name), key: coal };
            at.live.value(&owner.id.to_string(), "special", surplus("special_total"), 1.0);
            at.live.sum(&owner.id.to_string(), "coal", &[surplus("special_total"), surplus("production_demand")], 1.0);
            Ok(PowerReply { slot: owner.slot, rows: vec![owner.id.to_string()], warnings: Vec::new() })
        }
    }

    let row_trunc = "Country resources read only in part at this hour (1 rows not read)";
    let table_trunc = "Country resources read only in part at this hour (1 rows not read), and which rows were read is not held at the head";
    let coal_ger = CountryResourceRow { country: 9001, resource: 1, special_total: 400, production_demand: -10, ..CountryResourceRow::default() };
    let coal_ita = CountryResourceRow { country: 9002, resource: 1, special_total: 500, production_demand: -20, ..CountryResourceRow::default() };
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut f = frame(hour);
            match hour {
                110 => {
                    *f.tables[TableId::CountryResources.index()] = TableView::Truncated { rows: TableRows::CountryResources(vec![coal_ger.clone()]), missing: 1 };
                }
                111 => {
                    *f.tables[TableId::CountryResources.index()] = TableView::Truncated { rows: TableRows::CountryResources(vec![coal_ita.clone()]), missing: 1 };
                }
                _ => {
                    *f.tables[TableId::CountryResources.index()] = TableView::Complete(TableRows::CountryResources(vec![coal_ger.clone(), coal_ita.clone()]));
                }
            }
            f
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "at-hour-trunc-live".into(), revision: "1".into() }, "trunc-live.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let panel = exact_panel::<CoalScopes>(&source, &load(), &json!({ "branchId": "1", "country": "GER" })).unwrap();
    let exact = panel.exact().unwrap();
    let token = AtomicBool::new(false);

    let groups = |reply: &Value| -> (Value, Value) {
        let all = reply["live"]["groups"].as_array().unwrap();
        let row_group = all.iter().find(|group| group["columns"].as_array().unwrap().len() == 1).unwrap()["values"].clone();
        let table_group = all.iter().find(|group| group["columns"].as_array().unwrap().len() == 2).unwrap()["values"].clone();
        (row_group, table_group)
    };

    for (hour, expected_row, expected_table) in [
        (109, json!([400]), json!([400, -10])),
        (110, json!([400]), json!([table_trunc, table_trunc])),
        (111, json!([row_trunc]), json!([table_trunc, table_trunc])),
        (112, json!([400]), json!([400, -10])),
    ] {
        let cold = (exact.cold)(hour, &token).unwrap();
        let (got_row, got_table) = groups(&cold);
        assert_eq!(got_row, expected_row, "Row-scoped live.value at hour {hour}");
        assert_eq!(got_table, expected_table, "Table-scoped live.sum at hour {hour}");
        let adv = (exact.advance)(hour, true, &token, None);
        assert_eq!(adv.value.as_ref().as_ref().unwrap(), &cold, "incremental with values equals cold at hour {hour}");
    }

    let reads = stepped(&pages, exact, 1, 108, 1);
    for read in reads.into_iter().filter(|read| !read.pending) {
        let cold = (exact.cold)(read.hour, &token);
        assert_eq!(streamed(&read.value, read.fresh), streamed(&cold, read.fresh), "stepped equals cold at hour {}", read.hour);
    }
}

#[test]
fn a_quiet_advance_on_a_populated_product_visits_zero_rows() {
    struct Quiet;
    struct Expiring;

    #[derive(Serialize, PartialEq)]
    #[cfg_attr(any(test, feature = "export-types"), derive(TS))]
    #[serde(rename_all = "camelCase")]
    struct ExpiryReply {
        expired: bool,
        count: usize,
        warnings: Vec<String>,
    }
    crate::reply_warnings!(ExpiryReply);

    impl AtHour for Quiet {
        const KEY: &'static str = "test-quiet";
        const TITLE: &'static str = "Quiet";
        const CONTENT_DEPENDENT: bool = false;
        const CLOCK: Clock = Clock::InputsOnly;
        type Params = PlannedParams;
        const FIELDS: crate::products::Fields = crate::fields![branchId];
        type Reply = SteadyReply;
        type Context = std::sync::atomic::AtomicUsize;

        fn tables(_params: &Self::Params) -> Vec<TableId> {
            vec![TableId::CountryLinks]
        }

        fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
            Ok(0.into())
        }

        fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
            at.context.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let links = frame.complete::<tables::CountryLinks>()?;
            Ok(SteadyReply { tags: links.iter().map(|row| row.id.to_string()).collect(), warnings: Vec::new() })
        }
    }

    const EXPIRES_AT: i32 = FIRST + 6;

    impl AtHour for Expiring {
        const KEY: &'static str = "test-expiring";
        const TITLE: &'static str = "Expiring";
        const CONTENT_DEPENDENT: bool = false;
        const CLOCK: Clock = Clock::Boundaries;
        type Params = PlannedParams;
        const FIELDS: crate::products::Fields = crate::fields![branchId];
        type Reply = ExpiryReply;
        type Context = std::sync::atomic::AtomicUsize;

        fn tables(_params: &Self::Params) -> Vec<TableId> {
            vec![TableId::CountryLinks]
        }

        fn valid_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
            if frame.hour < EXPIRES_AT { EXPIRES_AT - 1 } else { i32::MAX }
        }

        fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
            Ok(0.into())
        }

        fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
            let count = at.context.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let links = frame.complete::<tables::CountryLinks>()?;
            assert!(!links.is_empty());
            Ok(ExpiryReply { expired: frame.hour >= EXPIRES_AT, count, warnings: Vec::new() })
        }
    }

    let frames: Vec<DecodedFrame> = (FIRST..=LAST)
        .map(|hour| {
            let mut f = frame(FIRST);
            f.hour = hour;
            f
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "at-hour-quiet".into(), revision: "1".into() }, "quiet.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

    let run_declared = |exact: &dashboard_follow::panel::AtHour| -> Vec<Stepped> {
        let seed = (exact.advance)(FIRST, true, &AtomicBool::new(false), None);
        assert!(seed.rows > 0, "initial seed reads populated rows");
        let (send, played) = std::sync::mpsc::channel();
        let advance = exact.advance.clone();
        let hook: playhead::Hook = Box::new(move |step, lanes| {
            let token = AtomicBool::new(false);
            let read = advance(step.last(), step.reset(), &token, Some(dashboard_follow::panel::ProductStep { step, lanes, held: None, wake: None }));
            let _ = send.send((step.last(), Stepped { hour: step.last(), rows: read.rows, fresh: read.fresh, pending: read.pending, waited: false, value: read.value, lanes: read.lanes }));
        });
        let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 1, ring: 3, speed: 1.0, rate: 1.0, ..playhead::Settings::default() };
        let playhead = playhead::Playhead::new(Arc::clone(&pages), playhead::Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap();
        let reader = playhead.declare(seed.lanes, None).unwrap();
        let mut consumer = playhead.consumer(reader);
        playhead.seek(FIRST);
        let (mut head, mut out) = (FIRST, Vec::new());
        loop {
            let mut next = None;
            playhead
                .drive_until(&mut consumer, head, || match played.try_recv() {
                    Ok(item) => {
                        next = Some(item);
                        std::ops::ControlFlow::Break(Ok(()))
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => std::ops::ControlFlow::Continue(()),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => std::ops::ControlFlow::Break(Err("step hook channel disconnected".into())),
                })
                .unwrap();
            let (last, read) = next.unwrap();
            out.push(read);
            if last >= LAST { break; }
            if last >= head { head = (head + 1).min(LAST); }
        }
        drop(consumer);
        playhead.finish();
        out
    };

    let quiet_panel = exact_panel::<Quiet>(&source, &load(), &json!({ "branchId": "1" })).unwrap();
    let quiet_reads = run_declared(quiet_panel.exact().unwrap());
    assert!(quiet_reads.len() >= 15);
    assert!(quiet_reads.iter().all(|r| !r.pending));
    for read in &quiet_reads[2..] {
        assert_eq!(read.rows, 0, "quiet hour {} must visit zero rows", read.hour);
        assert!(Arc::ptr_eq(&read.value, &quiet_reads[1].value), "quiet hour {} reuses the held reply", read.hour);
    }

    let expiring_panel = exact_panel::<Expiring>(&source, &load(), &json!({ "branchId": "1" })).unwrap();
    let expiring_reads = run_declared(expiring_panel.exact().unwrap());
    assert!(expiring_reads.iter().all(|r| !r.pending));
    let before = expiring_reads.iter().find(|r| r.hour == EXPIRES_AT - 1).unwrap();
    let at_expiry = expiring_reads.iter().find(|r| r.hour == EXPIRES_AT).unwrap();
    let after = expiring_reads.iter().find(|r| r.hour == EXPIRES_AT + 1).unwrap();
    assert_eq!(before.rows, 0);
    assert_eq!(before.value.as_ref().as_ref().unwrap()["expired"], false);
    assert_eq!(before.value.as_ref().as_ref().unwrap()["count"], 1);
    assert_eq!(at_expiry.rows, 0);
    assert_eq!(at_expiry.value.as_ref().as_ref().unwrap()["expired"], true);
    assert_eq!(at_expiry.value.as_ref().as_ref().unwrap()["count"], 2);
    assert_eq!(after.rows, 0);
    assert!(Arc::ptr_eq(&after.value, &at_expiry.value));
}
