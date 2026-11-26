
use super::{LoadContext, PanelKind, PanelReader, Registered, ReplyWarnings};
use crate::{
    Source,
    catalogue::{OwnRefusals, own_refusals},
};
use dashboard_follow::{
    live::{Cells, Group, GroupState, Live},
    panel::{Declared, Panel, PanelData},
};
use dashboard_query::{
    Query,
    identity::{Country, Holder, Holders},
};
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    marker::PhantomData,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

mod incremental;
mod placed;
mod validity;
pub use validity::{ReadDates, ScriptedInputs, reads_through};
pub use incremental::{Edits, Held};

pub use replay_frame::frame::Keys;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub table: TableId,
    pub keys: Keys,
}

impl Read {
    pub fn all(table: TableId) -> Self {
        Self { table, keys: Keys::All }
    }

    pub fn these(table: TableId, keys: impl IntoIterator<Item = u128>) -> Self {
        let mut keys: Vec<u128> = keys.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        Self { table, keys: Keys::These(keys) }
    }
}

pub fn field_mask(table: TableId, fields: &[&str]) -> u64 {
    fields.iter().map(|field| 1u64 << dashboard_declare::field(table, field).expect("a declared product field")).fold(0, |a, b| a | b)
}

pub fn field_columns(table: TableId, fields: &[&str]) -> Vec<usize> {
    fields.iter().map(|field| dashboard_declare::field(table, field).expect("a declared product field")).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub slot: u32,
    pub id: u32,
}

pub struct At<'a, C> {
    pub owner: Option<Owner>,
    pub holders: &'a Holders,
    pub hour: i32,
    pub context: &'a C,
    pub cancelled: &'a AtomicBool,
    pub live: &'a Cells,
}

pub struct ReadsAt<'a, C> {
    pub holders: &'a Holders,
    pub context: &'a C,
    pub run: u32,
    pub hour: i32,
    pub since: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    InputsOnly,
    Boundaries,
    EveryHour(&'static str),
}

pub trait AtHour: 'static {
    const KEY: &'static str;
    const TITLE: &'static str;
    const CONTENT_DEPENDENT: bool;
    const INTERACTIVE: bool = true;
    const FIELDS: super::Fields;
    type Params: DeserializeOwned + Send + 'static;
    type Reply: Serialize + ReplyWarnings + PartialEq + Send + 'static;
    type Context: Send + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String>
    where
        Self::Params: TS,
        Self::Reply: TS,
    {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    fn owner(_params: &Self::Params) -> Option<&Country> {
        None
    }

    fn tables(params: &Self::Params) -> Vec<TableId>;

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let _ = (owner, held);
        if step == 0 { Self::tables(params).into_iter().map(Read::all).collect() } else { Vec::new() }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let _ = at;
        Self::reads(params, owner, step, held)
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        (Self::reads_in(params, owner, stage, held, at), i32::MAX)
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let _ = table;
        None
    }

    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> { Self::columns(table) }

    fn plan_changes(_stage: usize, edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool { !edits.tables.is_empty() || !edits.replaced.is_empty() }

    fn compares_previous(_table: TableId) -> bool { false }


    const CLOCK: Clock;

    const REPLY_NAMES_NO_HOUR: bool = false;

    fn valid_through(_frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
        i32::MAX
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, _reply: &Self::Reply, partial: bool) -> i32 {
        if partial { frame.hour } else { Self::valid_through(frame, params, at) }
    }

    fn holds_refusal(_frame: &DecodedFrame) -> bool { false }

    fn context_revision(_context: &Self::Context) -> u64 { 0 }

    fn clock_dependent(_plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> bool {
        matches!(Self::CLOCK, Clock::EveryHour(_))
    }

    fn advance(_frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>, _edits: &Edits, _live: &Live, _data: &mut Held<'_>) -> io::Result<bool> {
        Ok(false)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context>;

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let _ = params;
        Self::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply>;
}

fn holder<P: AtHour>(holders: &Holders, params: &P::Params, hour: i32) -> io::Result<Option<Owner>> {
    let Some(country) = P::owner(params) else { return Ok(None) };
    match holders.at(country, hour).map(|run| run.holder) {
        Some(Holder::Found { slot, id }) => Ok(Some(Owner { slot, id })),
        Some(Holder::Unresolved) | None => Err(dashboard_core::refused(holders.not_known(country, hour).unwrap_or_else(|| format!("{} country is unavailable at this hour", P::TITLE)))),
    }
}

fn empty(branch: u64, hour: i32) -> DecodedFrame {
    DecodedFrame { branch, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() }
}

fn converted<'a>(reads: &'a [Read], fields: &'a [Option<Vec<usize>>]) -> Vec<replay_lanes::at::Read<'a>> {
    reads
        .iter()
        .zip(fields)
        .map(|(read, fields)| replay_lanes::at::Read {
            table: read.table,
            columns: fields.as_deref(),
            keys: match &read.keys {
                Keys::All => replay_lanes::at::Keys::All,
                Keys::These(keys) => replay_lanes::at::Keys::These(keys),
                Keys::Range(lo, hi) => replay_lanes::at::Keys::Range(*lo, *hi),
                Keys::Ranges(ranges) => replay_lanes::at::Keys::Ranges(ranges),
            },
        })
        .collect()
}

thread_local! { static READ_ROWS: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }

pub fn counted<T>(work: impl FnOnce() -> T) -> (T, u64) {
    let before = READ_ROWS.replace(Some(0));
    let value = work();
    let rows = READ_ROWS.replace(before).unwrap();
    (value, rows)
}

pub fn count_rows(rows: impl FnOnce() -> u64) {
    if let Some(n) = READ_ROWS.get() { READ_ROWS.set(Some(n + rows())); }
}

fn count_frame(frame: &DecodedFrame, reads: &[Read]) {
    count_rows(|| reads.iter().map(|r| match frame.tables[r.table.index()].held() {
        replay_frame::frame::TableView::Complete(rows) | replay_frame::frame::TableView::Truncated { rows, .. } => rows.len() as u64,
        _ => 0,
    }).sum());
}

fn frame_at(query: &Query<'_>, run: u32, hour: i32, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>) -> io::Result<DecodedFrame> {
    let fields: Vec<Option<Vec<usize>>> = reads.iter().map(|read| columns(read.table)).collect();
    replay_lanes::at::frame(query.replay.recording(), run, hour, &converted(reads, &fields)).inspect(|frame| count_frame(frame, reads))
}

pub struct Frames<'q> {
    query: &'q Query<'q>,
    seeker: replay_lanes::at::Seeker<'q>,
}

impl<'q> Frames<'q> {
    pub fn new(query: &'q Query<'q>) -> io::Result<Self> {
        Ok(Self { query, seeker: replay_lanes::at::Seeker::new(query.replay.recording())? })
    }

    pub fn frame(&mut self, hour: i32, reads: &[Read], title: &str) -> io::Result<DecodedFrame> {
        self.frame_with(hour, reads, |_| None, title)
    }

    pub fn frame_with(&mut self, hour: i32, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>, title: &str) -> io::Result<DecodedFrame> {
        let run = self.query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{title} hour is not recorded")))?.run;
        let fields: Vec<_> = reads.iter().map(|read| columns(read.table)).collect();
        dashboard_query::cost::storage(|| self.seeker.frame(run, hour, &converted(reads, &fields))).inspect(|frame| count_frame(frame, reads))
    }
}

pub fn frame(query: &Query<'_>, hour: i32, reads: &[Read], title: &str) -> io::Result<DecodedFrame> {
    frame_with(query, hour, reads, |_| None, title)
}

pub fn frame_with(query: &Query<'_>, hour: i32, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>, title: &str) -> io::Result<DecodedFrame> {
    let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{title} hour is not recorded")))?.run;
    dashboard_query::cost::storage(|| frame_at(query, run, hour, reads, columns))
}

fn covers(read: &Read, group: &Group) -> bool {
    match &read.keys {
        Keys::All => true,
        Keys::These(keys) => group.keys.iter().all(|key| keys.binary_search(key).is_ok()),
        Keys::Range(low, high) => group.keys.iter().all(|key| low <= key && key <= high),
        Keys::Ranges(ranges) => group.keys.iter().all(|key| ranges.iter().any(|(low, high)| low <= key && key <= high)),
    }
}

fn answered<P: AtHour, C: std::borrow::Borrow<P::Context>>(query: &Query<'_>, pool: &Arc<placed::Pool>, params: &P::Params, hour: i32, context: impl FnOnce() -> io::Result<C>, cancelled: &AtomicBool) -> io::Result<(P::Reply, Option<Value>)> {
    let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{} hour is not recorded", P::TITLE)))?.run;
    let owner = holder::<P>(&query.holders, params, hour)?;
    let held = context()?;
    let context = held.borrow();
    let read_run = pool.pages.run_of(run).ok_or_else(|| dashboard_core::refused(format!("{} hour is in a run the recording cannot read", P::TITLE)))?;
    let mut count = 0;
    let mut input = incremental::Input { pool, run: read_run, hour, mode: incremental::capture_mode(&pool.pages)?, count: &mut count, step: None, placed: None };
    let mut answer = AnswerFrame::new(query.timeline.id, hour);
    let mut read_frame = |reads: &[Read]| dashboard_query::cost::storage(|| input.frame(reads, P::columns, query.timeline.id));
    for step in 0.. {
        crate::metadata::check_cancel(cancelled)?;
        let reads = P::reads_in(params, owner, step, &answer.frame, &ReadsAt { holders: &query.holders, context, run, hour, since: None });
        if reads.is_empty() {
            break;
        }
        answer.read(reads, &mut read_frame)?;
    }
    let cells = Cells::default();
    let reply = P::reply(&answer.frame, params, &At { owner, holders: &query.holders, hour, context, cancelled, live: &cells })?;
    let live = answer.live(&cells.finish(), &mut read_frame)?;
    Ok((reply, live))
}

struct AnswerFrame {
    frame: DecodedFrame,
    held: HashMap<TableId, Read>,
}

impl AnswerFrame {
    #[inline(never)]
    fn new(branch: u64, hour: i32) -> Self {
        Self { frame: empty(branch, hour), held: HashMap::new() }
    }

    #[inline(never)]
    fn read(&mut self, reads: Vec<Read>, read_frame: &mut impl FnMut(&[Read]) -> io::Result<DecodedFrame>) -> io::Result<()> {
        let read = read_frame(&reads)?;
        for asked in reads {
            self.frame.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
            self.held.insert(asked.table, asked);
        }
        Ok(())
    }

    #[inline(never)]
    fn live(&mut self, live: &Live, read_frame: &mut impl FnMut(&[Read]) -> io::Result<DecodedFrame>) -> io::Result<Option<Value>> {
        if live.is_empty() {
            return Ok(None);
        }
        let reads = supplemental(live, |group| self.held.get(&group.table).is_some_and(|read| covers(read, group)));
        if !reads.is_empty() {
            self.read(reads, read_frame)?;
        }
        let states: Vec<GroupState> = live.values(&self.frame).into_iter().map(GroupState::Values).collect();
        Ok(Some(live.json(&states)))
    }
}

fn supplemental(live: &Live, covered: impl Fn(&Group) -> bool) -> Vec<Read> {
    let mut unread: BTreeMap<TableId, BTreeSet<u128>> = BTreeMap::new();
    for group in live.groups.iter().filter(|group| !covered(group)) {
        unread.entry(group.table).or_default();
    }
    for group in &live.groups {
        if let Some(keys) = unread.get_mut(&group.table) {
            keys.extend(&group.keys);
        }
    }
    unread.into_iter().map(|(table, keys)| Read::these(table, keys)).collect()
}

#[derive(Serialize, PartialEq)]
struct WithLive<R> {
    #[serde(flatten)]
    reply: R,
    #[serde(skip_serializing_if = "Option::is_none")]
    live: Option<Value>,
}

impl<R: ReplyWarnings> ReplyWarnings for WithLive<R> {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        self.reply.warnings_mut()
    }
}

fn finish(warnings: &mut Vec<String>, own: &OwnRefusals, content: &[String]) {
    own.diagnostics().append_to(warnings);
    warnings.extend(content.iter().cloned());
    warnings.extend(own.fallback_line());
}

fn content_warnings<P: AtHour>(source: &Source, load: &LoadContext) -> Vec<String> {
    let mut warnings = Vec::new();
    if P::CONTENT_DEPENDENT {
        source.append_changed_warnings(&mut warnings, load.content.as_ref());
    }
    source.append_presentation_warnings(&mut warnings, load.content.as_ref());
    warnings
}

fn request_hour(request: &mut Value) -> Option<i32> {
    let hour = request.as_object_mut()?.remove("hour")?;
    i32::try_from(hour.as_i64()?).ok()
}

fn load<P: AtHour>(source: &Source, lease_id: &str, mut request: Value, load: &LoadContext, cancelled: &AtomicBool) -> Result<String, dashboard_core::QueryError> {
    let hour = request_hour(&mut request).ok_or_else(|| format!("Invalid {} request: it names no hour", P::KEY))?;
    let params = dashboard_query::named::named::<P::Params>(&request).map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    super::reply_json(source, lease_id, P::KEY, P::CONTENT_DEPENDENT, load, cancelled, || {
        let query = crate::queries::query(source, dashboard_query::branch(&request)?)?;
        let pool = placed::Pool::of(source)?;
        answered::<P, _>(&query, &pool, &params, hour, || P::context_for(source, load, &params), cancelled).map(|(reply, live)| WithLive { reply, live })
    })
}

pub fn exact_panel<P: AtHour>(source: &Source, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String>
where P::Params: Sync, P::Context: Sync {
    let mut request = asked.clone();
    if let Some(fields) = request.as_object_mut() { fields.remove("panel"); }
    let params = dashboard_query::named::named_fields::<P::Params>(&request).map_err(|reason| format!("Invalid {} panel: {reason}", P::KEY))?;
    P::validate(&params)?;
    let branch = dashboard_query::branch(asked).map_err(|e| e.to_string())?;
    let query = crate::queries::query(source, branch).map_err(|e| e.to_string())?;
    let context = Arc::new(P::context_for(source, load, &params).map_err(|e| e.to_string())?);
    let params = Arc::new(params);
    let warnings = content_warnings::<P>(source, load);
    let pool = placed::Pool::of(source).map_err(|e| e.to_string())?;
    let advance = shared_reader(source, load, P::KEY, asked, || incremental::reader::<P>(source, pool.clone(), params.clone(), context.clone(), Query { replay: query.replay, holders: query.holders.clone(), timeline: query.timeline.clone() }, warnings.clone()));
    let (replay, holders, timeline, executable) = (source.replay(), query.holders, query.timeline, source.tables());
    let cold: dashboard_follow::panel::ExactReader = Arc::new(move |hour, cancelled| {
        let query = Query { replay: &replay, timeline: timeline.clone(), holders: holders.clone() };
        let mut own = OwnRefusals::default();
        let (mut reply, live) = engine_executable::executable_tables::scope(executable, || own_refusals(&mut own, ||
            answered::<P, _>(&query, &pool, &params, hour, || Ok(context.as_ref()), cancelled))).map_err(|e| e.to_string())?;
        finish(reply.warnings_mut(), &own, &warnings);
        serde_json::to_value(WithLive { reply, live }).map_err(|e| e.to_string())
    });
    Ok(product_panel(advance, cold))
}

pub fn interval_panel(
    executable: Option<engine_executable::executable_tables::Tables>,
    read: impl Fn(i32, &AtomicBool) -> io::Result<(Value, i32, u64)> + Send + Sync + 'static,
) -> Box<dyn Panel> {
    dated_interval(executable, read, |_, _| {})
}

fn dated_interval(
    executable: Option<engine_executable::executable_tables::Tables>,
    read: impl Fn(i32, &AtomicBool) -> io::Result<(Value, i32, u64)> + Send + Sync + 'static,
    date: impl Fn(&mut Arc<PanelData>, i32) + Send + Sync + 'static,
) -> Box<dyn Panel> {
    use dashboard_follow::panel::{ExactReader, ProductRead, ProductReader};
    let read = Arc::new(move |hour, cancelled: &AtomicBool| {
        engine_executable::executable_tables::scope(executable, || read(hour, cancelled))
            .map_err(|error| error.to_string())
    });
    let cold_read = Arc::clone(&read);
    let cold: ExactReader = Arc::new(move |hour, cancelled| cold_read(hour, cancelled).map(|(data, _, _)| data));
    let held = Mutex::new(None::<(std::ops::RangeInclusive<i32>, Arc<PanelData>, u64)>);
    let advance: ProductReader = Arc::new(move |hour, seed, cancelled, _| {
        let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !seed && let Some((interval, value, rows)) = &mut *held && interval.contains(&hour) {
            date(value, hour);
            return ProductRead { value: value.clone(), rows: *rows, fresh: false, pending: false, live: None, lanes: Vec::new() };
        }
        let (value, rows) = match read(hour, cancelled) {
            Ok((reply, through, rows)) => {
                let value = Arc::new(Ok(reply));
                *held = Some((hour..=through, value.clone(), rows));
                (value, rows)
            }
            Err(error) => {
                *held = None;
                (Arc::new(Err(error)), 0)
            }
        };
        ProductRead { value, rows, fresh: true, pending: false, live: None, lanes: Vec::new() }
    });
    product_panel(advance, cold)
}

pub fn at_hour(params: &mut Value, hour: i32) {
    params["hour"] = hour.into();
}

pub fn warnings_through<'a>(hour: i32, reasons: impl IntoIterator<Item = &'a str>) -> i32 {
    let stamp = format!("hour {hour} ");
    if reasons.into_iter().any(|reason| reason.contains(&stamp)) { hour } else { i32::MAX }
}

pub fn before(next: Option<i32>) -> i32 {
    next.map_or(i32::MAX, |n| n.saturating_sub(1))
}

pub fn day_panel<P: super::Product>(
    source: &Arc<Source>, load: &LoadContext, asked: &Value,
    start: &'static str, end: &'static str, through: fn(&P::Reply) -> i32,
) -> Result<Box<dyn Panel>, String> {
    dated_day_panel::<P>(source, load, asked, start, end, move |_, reply| through(reply), |_, _| {})
}

pub fn dated_day_panel<P: super::Product>(
    source: &Arc<Source>, load: &LoadContext, asked: &Value,
    start: &'static str, end: &'static str,
    through: impl Fn(&P::Params, &P::Reply) -> i32 + Send + Sync + 'static,
    date_held: impl Fn(&mut Arc<PanelData>, i32) + Send + Sync + 'static,
) -> Result<Box<dyn Panel>, String> {
    let first = dashboard_query::branch(asked).and_then(|branch| crate::queries::query(source, branch))
        .map_err(|e| e.to_string())?.timeline.first;
    let day_start = |hour: i32| hour.div_euclid(24).saturating_mul(24);
    let Some(tile_first) = asked[start].as_i64() else {
        return dated_product::<P>(source, load, asked, move |request, hour| {
            request[start] = first.max(day_start(hour)).into();
            request[end] = hour.into();
        }, through, date_held);
    };
    let tile_first = i32::try_from(tile_first).map_err(|_| "Invalid history start")?;
    let tile_last = asked[end].as_i64().and_then(|n| i32::try_from(n).ok()).ok_or("Invalid history end")?;
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Expected history parameters")?.remove("panel");
    let source = Arc::clone(source);
    let load = LoadContext { content: load.content.clone() };
    Ok(interval_panel(source.tables(), move |hour, cancelled| {
        let today = day_start(hour);
        if today <= tile_first {
            return Ok((Value::Null, day_start(tile_first).saturating_add(23), 0));
        }
        let mut request = params.clone();
        let last = tile_last.min(today.saturating_sub(1));
        request[end] = last.into();
        let (reply, rows) = counted(|| {
            crate::metadata::check_cancel(cancelled)?;
            super::register::<P>().answer(&source, request, &load, cancelled)
        });
        Ok((reply?, if last == tile_last { i32::MAX } else { today.saturating_add(23) }, rows))
    }))
}

pub fn product_interval_panel<P: super::Product>(
    source: &Arc<Source>, load: &LoadContext, asked: &Value,
    date: impl Fn(&mut Value, i32) + Send + Sync + 'static,
    valid_through: fn(&P::Reply) -> i32,
) -> Result<Box<dyn Panel>, String> {
    dated_product::<P>(source, load, asked, date, move |_, reply| valid_through(reply), |_, _| {})
}

pub fn dated_product<P: super::Product>(
    source: &Arc<Source>, load: &LoadContext, asked: &Value,
    date: impl Fn(&mut Value, i32) + Send + Sync + 'static,
    valid_through: impl Fn(&P::Params, &P::Reply) -> i32 + Send + Sync + 'static,
    date_held: impl Fn(&mut Arc<PanelData>, i32) + Send + Sync + 'static,
) -> Result<Box<dyn Panel>, String> {
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Expected panel parameters")?.remove("panel");
    let source = Arc::clone(source);
    let load = LoadContext { content: load.content.clone() };
    let mut warnings = Vec::new();
    if P::CONTENT_DEPENDENT {
        source.append_changed_warnings(&mut warnings, load.content.as_ref());
    }
    source.append_presentation_warnings(&mut warnings, load.content.as_ref());
    Ok(dated_interval(source.tables(), move |hour, cancelled| {
        let mut request = params.clone();
        date(&mut request, hour);
        let params = dashboard_query::named::named::<P::Params>(&request)?;
        P::validate(&params).map_err(dashboard_core::invalid)?;
        let mut own = OwnRefusals::default();
        #[cfg(any(test, feature = "test-support"))]
        crate::test_fixtures::product_called();
        let (reply, rows) = counted(|| own_refusals(&mut own, || P::load(&source, &params, &load, cancelled)));
        let mut reply = reply?;
        finish(reply.warnings_mut(), &own, &warnings);
        let through = valid_through(&params, &reply);
        Ok((serde_json::to_value(reply)?, through, rows))
    }, date_held))
}

pub fn product_panel(advance: dashboard_follow::panel::ProductReader, cold: dashboard_follow::panel::ExactReader) -> Box<dyn Panel> {
    let current = advance.clone();
    let read = Arc::new(move |hour, cancelled: &AtomicBool| current(hour, true, cancelled, None).value.as_ref().clone());
    Box::new(dashboard_follow::panel::AtHour { read, cold, advance })
}

pub fn shared_reader(source: &Source, load: &LoadContext, product: &str, params: &Value, make: impl FnOnce() -> dashboard_follow::panel::ProductReader) -> dashboard_follow::panel::ProductReader {
    type WeakReader = std::sync::Weak<dyn Fn(i32, bool, &AtomicBool, Option<dashboard_follow::panel::ProductStep<'_>>) -> dashboard_follow::panel::ProductRead + Send + Sync>;
    #[derive(Default)]
    struct Shared(HashMap<String, WeakReader>);
    let mut args = params.clone();
    if let Some(obj) = args.as_object_mut() { obj.remove("panel"); }
    let key = format!("{product} {args} {:?}", load.content.as_ref().map(|c| (&c.options.game_dir, &c.options.user_dir, &c.options.cache_dir, c.allow_unverified, Arc::as_ptr(c.presentation.binding()), c.presentation.identity())));
    let shared = source.state::<Shared>();
    let mut cache = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.0.retain(|_, r| r.strong_count() > 0);
    if let Some(reader) = cache.0.get(&key).and_then(WeakReader::upgrade) { return reader; }
    let reader = make();
    cache.0.insert(key, Arc::downgrade(&reader));
    reader
}

struct Followed<P: AtHour> {
    params: P::Params,
    tables: Vec<TableId>,
    holders: Arc<Holders>,
    context: P::Context,
    content_warnings: Vec<String>,
    live: Option<Live>,
    answered: Option<(Option<Owner>, Vec<replay_frame::frame::SharedView>)>,
    executable: Option<engine_executable::executable_tables::Tables>,
    product: PhantomData<fn() -> P>,
}

impl<P: AtHour> Panel for Followed<P> {
    fn tables(&self) -> &[TableId] {
        &self.tables
    }

    fn unchanged(&mut self, frame: &DecodedFrame) -> bool {
        let Some((owner, views)) = &self.answered else { return false };
        skips_unchanged::<P>()
            && holder::<P>(&self.holders, &self.params, frame.hour).ok().is_some_and(|held| held == *owner)
            && self.tables.iter().zip(views).all(|(table, view)| replay_frame::frame::SharedView::ptr_eq(&frame.tables[table.index()], view))
    }

    fn data(&mut self, frame: &DecodedFrame) -> PanelData {
        self.answered = None;
        let held = holder::<P>(&self.holders, &self.params, frame.hour).ok();
        let cancelled = AtomicBool::new(false);
        let mut own = OwnRefusals::default();
        let cells = Cells::default();
        self.live = None;
        let made = engine_executable::executable_tables::scope(self.executable, || {
            own_refusals(&mut own, || {
                let owner = holder::<P>(&self.holders, &self.params, frame.hour)?;
                P::reply(frame, &self.params, &At { owner, holders: &self.holders, hour: frame.hour, context: &self.context, cancelled: &cancelled, live: &cells })
            })
        });
        let mut reply = made.map_err(|error| error.to_string())?;
        finish(reply.warnings_mut(), &own, &self.content_warnings);
        self.live = Some(cells.finish()).filter(|live| !live.is_empty());
        let data = serde_json::to_value(&reply).map_err(|error| error.to_string())?;
        if skips_unchanged::<P>() && let Some(owner) = held {
            self.answered = Some((owner, self.tables.iter().map(|table| frame.tables[table.index()].clone()).collect()));
        }
        Ok(data)
    }

    fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }
}

pub fn frame_panel<P: AtHour>(source: &Source, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let mut asked = asked.clone();
    if let Some(fields) = asked.as_object_mut() {
        fields.remove("panel");
    }
    let params = dashboard_query::named::named::<P::Params>(&asked).map_err(|error| format!("Invalid {} panel: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    let branch = dashboard_query::branch(&asked).map_err(|error| error.to_string())?;
    let holders = Arc::clone(&crate::queries::query(source, branch).map_err(|error| error.to_string())?.holders);
    let context = P::context_for(source, load, &params).map_err(|error| error.to_string())?;
    Ok(Box::new(Followed::<P> { tables: P::tables(&params), params, holders, context, content_warnings: content_warnings::<P>(source, load), live: None, answered: None, executable: source.tables(), product: PhantomData }))
}

#[cfg(any(test, feature = "export-types"))]
fn types<P: AtHour>() -> Vec<String>
where
    P::Params: TS,
    P::Reply: TS,
{
    let mut out = P::types();
    out.push(format!("type {0}AtHour = {0} & {{ hour: number }};", <P::Params as TS>::name()));
    out
}

const fn skips_unchanged<P: AtHour>() -> bool {
    P::REPLY_NAMES_NO_HOUR && matches!(P::CLOCK, Clock::InputsOnly)
}

pub const fn whole_frame<P: AtHour>() -> PanelReader {
    PanelReader {
        kind: PanelKind::WholeFrame { every_frame: !skips_unchanged::<P>() },
        open: |source, load, asked| frame_panel::<P>(source, load, asked),
    }
}

pub const fn exact<P: AtHour>() -> PanelReader
where P::Params: Sync, P::Context: Sync {
    let kind = if matches!(P::CLOCK, Clock::EveryHour(_)) { PanelKind::ExactHourly } else { PanelKind::Exact };
    PanelReader { kind, open: |source, load, asked| exact_panel::<P>(source, load, asked) }
}

pub const fn entry<P: AtHour>() -> Registered {
    Registered {
        panel: None,
        key: P::KEY,
        content_dependent: P::CONTENT_DEPENDENT,
        interactive: P::INTERACTIVE,
        replays: false,
        load: load::<P>,
        fields: P::FIELDS,
        read_hour: true,
        #[cfg(any(test, feature = "export-types"))]
        types: None,
        #[cfg(any(test, feature = "export-types"))]
        params: dashboard_query::fields::walk::<P::Params>,
    }
}

#[cfg(any(test, feature = "export-types"))]
pub const fn entry_for_export<P: AtHour>() -> Registered
where P::Params: TS, P::Reply: TS {
    Registered { types: Some(types::<P>), ..entry::<P>() }
}

#[cfg(test)]
mod tests;
