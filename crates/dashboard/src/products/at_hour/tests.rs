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
use serde_json::json;

/// A country's links and political power at an hour: its links read whole first, then its own
/// row alone. Its structure is the slot and the countries linked; each linked country's power and
/// the holder's coal surplus are live cells.
struct Power;

#[derive(Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PowerParams {
    branch_id: String,
    tag: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct PowerReply {
    slot: u32,
    /// The raw ids linked at the hour, each a row of live cells.
    rows: Vec<String>,
    warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerReply);

fn column(table: TableId, name: &str) -> usize {
    dashboard_declare::field(table, name).unwrap()
}

impl AtHour for Power {
    const KEY: &'static str = "test-power";
    const TITLE: &'static str = "Power";
    const CONTENT_DEPENDENT: bool = false;
    type Params = PowerParams;
    type Reply = PowerReply;
    type Context = ();

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        Some(&params.tag)
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
/// ITA's country row leaves at this hour; its link stays.
const ITA_LEAVES: i32 = 115;
/// Country resources are not available at these hours.
const RESOURCES_GONE: std::ops::RangeInclusive<i32> = 120..=122;

/// GER's power moves every ten hours and has no value at hour 125; ITA's moves every hour until
/// its row leaves; GER's coal moves every hour and is unavailable for three.
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

/// The synthetic recording, and its pages as the playhead reads them.
fn recording() -> (Source, Arc<dyn Pages>) {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let source = Source::from_replay(crate::SourceKey { source_id: "at-hour".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    (source, pages)
}

fn load() -> LoadContext {
    LoadContext { content: None }
}

/// The request path's reply data to `request`, or its error.
fn asked(source: &Source, request: Value) -> Result<Value, dashboard_core::QueryError> {
    let text = (entry::<Power>().registered.load)(source, "lease", request, &load(), &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

fn requested(source: &Source, tag: &str, hour: i32) -> Result<Value, dashboard_core::QueryError> {
    asked(source, json!({ "branchId": "1", "tag": tag, "hour": hour }))
}

fn refused(error: dashboard_core::QueryError) -> String {
    match error {
        dashboard_core::QueryError::Refused { text } => text,
        dashboard_core::QueryError::Failed { message } => panic!("not a refusal: {message}"),
    }
}

/// The panel's data over the whole tables at `hour`, as rows at the head hold them.
fn followed(source: &Source, panel: &mut dyn Panel, hour: i32) -> PanelData {
    let tables = panel.tables().to_vec();
    let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, hour, Some(&tables)).unwrap() else { panic!("hour {hour} is recorded") };
    panel.data(&frame)
}

/// Each group of `live` as the follow layer answers it at `hour`, from a lane of its own sought there.
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

/// A request's cell as JSON: its number, or why it has none.
fn cell_json(value: &CellValue) -> Value {
    match value {
        CellValue::Number(number) => json!(number),
        CellValue::Missing(reason) => json!(reason),
    }
}

#[test]
fn a_rows_at_an_hour_product_answers_a_request_and_its_panel_with_one_reply() {
    let (source, pages) = recording();
    let mut panel = (entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "tag": "GER" })).unwrap();
    assert_eq!(panel.tables(), [TableId::Countries, TableId::CountryLinks]);
    let (mut rowless, mut unavailable, mut valueless) = (0, 0, 0);
    for hour in FIRST..=LAST {
        let mut asked = requested(&source, "GER", hour).unwrap();
        let live = asked.as_object_mut().unwrap().remove("live").expect("a request's reply carries its live cells");
        assert_eq!(followed(&source, &mut *panel, hour), Ok(asked.clone()), "the panel carries the structure alone, hour {hour}");
        assert_eq!(asked["slot"], 7);
        assert_eq!(asked["rows"], json!(["11", "9001", "9002"]));
        let named = panel.live().expect("the panel names its live cells").clone();
        // What the panel's host pushes names the same cells, each group followed instead of valued.
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
    // Link 11 has no country row at any hour; ITA's row leaves.
    assert_eq!(rowless, (ITA_LEAVES - FIRST) as usize + 2 * (LAST - ITA_LEAVES + 1) as usize, "a cell whose row left is missing by name");
    assert_eq!(unavailable, 2 * RESOURCES_GONE.count(), "a cell whose table is unavailable is missing, never the value before");
    assert_eq!(valueless, 1, "a value not recorded is missing, never zero");
    let at = |hour| requested(&source, "GER", hour).unwrap()["live"]["groups"].clone();
    assert_eq!(at(125).as_array().unwrap().iter().flat_map(|group| group["values"].as_array().unwrap().clone()).filter(|value| *value == NO_VALUE).count(), 1);
    // The host pushes a panel only when its data differs: the structure holds while the numbers move.
    assert_eq!(followed(&source, &mut *panel, 101), followed(&source, &mut *panel, 128));
}

#[test]
fn a_refused_reply_is_the_same_named_refusal_on_both_paths() {
    let (source, _) = recording();
    let unheld = "Power country is unavailable at this hour";
    assert_eq!(refused(requested(&source, "FRA", 110).unwrap_err()), unheld);
    let mut panel = (entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "tag": "FRA" })).unwrap();
    assert_eq!(followed(&source, &mut *panel, 110), Err(unheld.to_owned()));
    assert!(panel.live().is_none(), "a refused reply names no cells");
    assert_eq!(refused(requested(&source, "GER", LAST + 50).unwrap_err()), "Power hour is not recorded");
    assert!(asked(&source, json!({ "branchId": "1", "tag": "GER" })).unwrap_err().to_string().contains("names no hour"));
    assert!((entry::<Power>().panel)(&source, &load(), &json!({ "panel": "test-power", "branchId": "1", "tag": "GER", "hour": 3 })).is_err(), "a panel takes the head's hour");
}

#[test]
fn every_rows_at_an_hour_product_is_registered_moved_and_shown_as_a_panel() {
    for entry in crate::products::AT_HOUR {
        let key = entry.registered.key;
        assert!(crate::products::find(key).is_some(), "{key} answers requests");
        assert!(crate::products::MOVED.contains(&key), "{key} is answered through the query layer");
    }
    assert!(crate::products::find(super::super::army::TemplateRoster::KEY).is_some());
}

/// Whether executable tables are in scope where its reply is made.
struct Scoped;

#[derive(Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopedParams {
    branch_id: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ScopedReply {
    scoped: bool,
    warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ScopedReply);

impl AtHour for Scoped {
    const KEY: &'static str = "test-scoped";
    const TITLE: &'static str = "Scoped";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ScopedParams;
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
    let holders = Arc::clone(&crate::answer::query(&source, 1).unwrap().holders);
    let mut panel = Followed::<Scoped> {
        params: ScopedParams { branch_id: "1".into() },
        tables: vec![TableId::Countries],
        holders,
        context: (),
        content_warnings: Vec::new(),
        live: None,
        executable: Some(executable),
        product: PhantomData,
    };
    let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, FIRST, Some(&[TableId::Countries])).unwrap() else { panic!("hour {FIRST} is recorded") };
    // The host's panels thread is a plain thread: no recording's tables are in scope there.
    let data = std::thread::scope(|threads| threads.spawn(|| panel.data(&frame)).join().unwrap());
    assert_eq!(data.unwrap()["scoped"], true, "a panel reading recorded keywords or registries would find them withheld");
}