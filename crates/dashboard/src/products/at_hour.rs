//! Products that show rows at an hour: a reply is one function of a typed frame of a few tables
//! and the recorded content ([`AtHour::reply`]). The same function answers a request for an hour,
//! from the rows the product names read at that hour ([`AtHour::reads`]), and a panel at the head
//! under the product's key (`{"panel": <key>, ..params}`), from the playhead's rows, pushed only
//! when the reply differs. The adapter resolves the hour's run and the asked tag's holder by the
//! one identity rule; a product never resolves either.
//!
//! A reply carries structure (rows, names, art) and the keys it shows. Numbers that move every
//! hour are not its: the product names them as live cells as it makes the reply ([`At::live`]),
//! each by the row and column the structure shows it in. A request's reply carries their values at
//! its hour (`live`, [`Live::json`]); a panel's carries where each group of them is followed, and
//! the follow layer pushes the rows that change, so the panel is pushed when the structure
//! changes, not each hour.

use super::{LoadContext, Registered, ReplyWarnings};
use crate::{
    Source,
    catalogue::{OwnRefusals, own_refusals},
};
use dashboard_follow::{
    live::{Cells, Group, GroupState, Live},
    panel::{Panel, PanelData},
};
use dashboard_query::{
    Query,
    identity::{Holder, Holders},
};
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    marker::PhantomData,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

/// Which rows of a table a read keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Keys {
    All,
    /// These keys, ascending, each once.
    These(Vec<u128>),
    /// Keys from the first through the second: every row of an owner whose key holds it in its
    /// high bits.
    Range(u128, u128),
    /// Keys within any of these ranges, ascending and apart: every row of several owners.
    Ranges(Vec<(u128, u128)>),
}

/// One table read at the hour: the rows kept, whole (a typed row holds every field, so a column
/// read alone would show the others as recorded zeros).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub table: TableId,
    pub keys: Keys,
}

impl Read {
    /// Every row of `table`.
    pub fn all(table: TableId) -> Self {
        Self { table, keys: Keys::All }
    }

    /// The rows of `table` keyed by `keys`.
    pub fn these(table: TableId, keys: impl IntoIterator<Item = u128>) -> Self {
        let mut keys: Vec<u128> = keys.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        Self { table, keys: Keys::These(keys) }
    }
}

/// The holder of the tag a reply is about at its hour: canonical slot and raw country id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub slot: u32,
    pub id: u32,
}

/// What a reply is given beside its frame and params.
pub struct At<'a, C> {
    /// The asked tag's holder, when the product names one ([`AtHour::owner`]).
    pub owner: Option<Owner>,
    /// Every tag's holders on the timeline, for a reply about several tags; each is resolved at
    /// `hour` by the one identity rule, never by the product.
    pub holders: &'a Holders,
    pub hour: i32,
    pub context: &'a C,
    pub cancelled: &'a AtomicBool,
    /// Where the reply names the numbers it shows that move with the playhead: none of them is in
    /// the reply itself.
    pub live: &'a Cells,
}

/// Where a reply's reads are asked ([`AtHour::reads_in`]): the timeline's holders, the hour, its
/// run and the reply's context.
pub struct ReadsAt<'a, C> {
    pub holders: &'a Holders,
    pub context: &'a C,
    pub run: u32,
    pub hour: i32,
}

/// A product that shows rows at an hour. Its params carry the request's `branchId`; a request adds
/// `hour`, a panel takes the head's.
pub trait AtHour: 'static {
    const KEY: &'static str;
    /// Its name in the adapter's refusals ("<title> hour is not recorded").
    const TITLE: &'static str;
    /// Whether any reply field reads localisation, artwork or content definitions.
    const CONTENT_DEPENDENT: bool;
    const INTERACTIVE: bool = true;
    type Params: DeserializeOwned + TS + Send + 'static;
    type Reply: Serialize + TS + ReplyWarnings + 'static;
    /// What a reply needs of the recorded content, made once per request or per follow.
    type Context: Send + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    /// TypeScript declarations it introduces, parameters and reply first.
    fn types() -> Vec<String> {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    /// The tag whose holder the reply is about; refused by name where no country holds it.
    fn owner(_params: &Self::Params) -> Option<&str> {
        None
    }

    /// Every table a reply may read: what its panel declares to the playhead.
    fn tables(params: &Self::Params) -> Vec<TableId>;

    /// The reads of step `step` given the rows the steps before read (`held`); none ends the
    /// steps. Each step's tables are read at once; a table read again replaces its rows. The rows
    /// read must hold every row `reply` looks at, so a frame of whole tables gives the same reply.
    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let _ = (owner, held);
        if step == 0 { Self::tables(params).into_iter().map(Read::all).collect() } else { Vec::new() }
    }

    /// [`AtHour::reads`] knowing where the reply is asked: for a product whose keys are the asked
    /// tags' holders at the hour, or whose rows are found through an index of the run the context holds.
    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let _ = at;
        Self::reads(params, owner, step, held)
    }

    /// The fields a request reads of `table`'s rows: every one unless the product names fewer, for
    /// a table whose other fields it never looks at (they hold their row's defaults, never decoded).
    fn columns(table: TableId) -> Option<Vec<usize>> {
        let _ = table;
        None
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context>;

    /// The context of a reply to `params`: for a product whose context holds what the recording's
    /// open made for the asked timeline.
    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let _ = params;
        Self::context(source, load)
    }

    /// The reply at `frame`'s hour: missing data is a named reason in it, never zero; a reply
    /// that cannot be made is a named refusal ([`dashboard_core::refused`]).
    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply>;
}

fn holder<P: AtHour>(holders: &Holders, params: &P::Params, hour: i32) -> io::Result<Option<Owner>> {
    let Some(tag) = P::owner(params) else { return Ok(None) };
    match holders.at(tag, hour).map(|run| run.holder) {
        Some(Holder::Found { slot, id }) => Ok(Some(Owner { slot, id })),
        Some(Holder::Unresolved) | None => Err(dashboard_core::refused(format!("{} country is unavailable at this hour", P::TITLE))),
    }
}

fn empty(branch: u64, hour: i32) -> DecodedFrame {
    DecodedFrame { branch, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() }
}

/// `reads` after `hour` of run `run` of `query`'s recording, each table in the fields `columns`
/// names, every other table not selected.
/// `reads` as the replay layer reads them, each table's fields `fields` names.
fn converted<'a>(reads: &'a [Read], fields: &'a [Option<Vec<usize>>]) -> Vec<replay::at::Read<'a>> {
    reads
        .iter()
        .zip(fields)
        .map(|(read, fields)| replay::at::Read {
            table: read.table,
            columns: fields.as_deref(),
            keys: match &read.keys {
                Keys::All => replay::at::Keys::All,
                Keys::These(keys) => replay::at::Keys::These(keys),
                Keys::Range(lo, hi) => replay::at::Keys::Range(*lo, *hi),
                Keys::Ranges(ranges) => replay::at::Keys::Ranges(ranges),
            },
        })
        .collect()
}

fn frame_at(query: &Query<'_>, run: u32, hour: i32, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>) -> io::Result<DecodedFrame> {
    let fields: Vec<Option<Vec<usize>>> = reads.iter().map(|read| columns(read.table)).collect();
    replay::at::frame(query.replay.recording(), run, hour, &converted(reads, &fields))
}

/// [`frame`]s at hours a reply asks in turn, of whatever tables and keys each asks, through one
/// seeker: each table restored once and read forward between the hours, never restored an hour.
/// A reply naming many things at their own hours asks them in hour order.
pub struct Frames<'q> {
    query: &'q Query<'q>,
    seeker: replay::at::Seeker<'q>,
}

impl<'q> Frames<'q> {
    pub fn new(query: &'q Query<'q>) -> io::Result<Self> {
        Ok(Self { query, seeker: replay::at::Seeker::new(query.replay.recording())? })
    }

    /// [`frame`] of `reads` at `hour`.
    pub fn frame(&mut self, hour: i32, reads: &[Read], title: &str) -> io::Result<DecodedFrame> {
        let run = self.query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{title} hour is not recorded")))?.run;
        let fields: Vec<Option<Vec<usize>>> = vec![None; reads.len()];
        dashboard_query::cost::storage(|| self.seeker.frame(run, hour, &converted(reads, &fields)))
    }
}

/// `reads` after `hour` of `query`'s timeline, their time counted as storage: for a product that
/// reads rows at hours of its own choosing. Refused by name (`title`) where no run holds the hour.
pub fn frame(query: &Query<'_>, hour: i32, reads: &[Read], title: &str) -> io::Result<DecodedFrame> {
    frame_with(query, hour, reads, |_| None, title)
}

/// [`frame`] reading only the fields `columns` names of a table it names any of.
pub fn frame_with(query: &Query<'_>, hour: i32, reads: &[Read], columns: fn(TableId) -> Option<Vec<usize>>, title: &str) -> io::Result<DecodedFrame> {
    let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{title} hour is not recorded")))?.run;
    dashboard_query::cost::storage(|| frame_at(query, run, hour, reads, columns))
}

/// `P`'s reply at `hour` of `query`'s timeline: the hour's run and the holder resolved, then the
/// content (`context`), then the rows it names read step by step, their time counted as storage.
pub fn answer<P: AtHour>(query: &Query<'_>, params: &P::Params, hour: i32, context: impl FnOnce() -> io::Result<P::Context>, cancelled: &AtomicBool) -> io::Result<P::Reply> {
    answered::<P>(query, params, hour, context, cancelled).map(|(reply, _)| reply)
}

/// Whether `read` holds every row of `group`.
fn covers(read: &Read, group: &Group) -> bool {
    match &read.keys {
        Keys::All => true,
        Keys::These(keys) => group.keys.iter().all(|key| keys.binary_search(key).is_ok()),
        Keys::Range(low, high) => group.keys.iter().all(|key| low <= key && key <= high),
        Keys::Ranges(ranges) => group.keys.iter().all(|key| ranges.iter().any(|(low, high)| low <= key && key <= high)),
    }
}

/// [`answer`], and the live cells it names with their values at `hour` ([`Live::json`]), none when
/// it names none. A table holding a cell's row the product did not read is read again at the hour,
/// the rows of every group of it, as that read replaces the product's.
fn answered<P: AtHour>(query: &Query<'_>, params: &P::Params, hour: i32, context: impl FnOnce() -> io::Result<P::Context>, cancelled: &AtomicBool) -> io::Result<(P::Reply, Option<Value>)> {
    let run = query.timeline.at(hour).ok_or_else(|| dashboard_core::refused(format!("{} hour is not recorded", P::TITLE)))?.run;
    let owner = holder::<P>(&query.holders, params, hour)?;
    let context = context()?;
    let mut frame = empty(query.timeline.id, hour);
    let mut held: HashMap<TableId, Read> = HashMap::new();
    for step in 0.. {
        crate::metadata::check_cancel(cancelled)?;
        let reads = P::reads_in(params, owner, step, &frame, &ReadsAt { holders: &query.holders, context: &context, run, hour });
        if reads.is_empty() {
            break;
        }
        let read = dashboard_query::cost::storage(|| frame_at(query, run, hour, &reads, P::columns))?;
        for asked in reads {
            frame.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
            held.insert(asked.table, asked);
        }
    }
    let cells = Cells::default();
    let reply = P::reply(&frame, params, &At { owner, holders: &query.holders, hour, context: &context, cancelled, live: &cells })?;
    let live = cells.finish();
    if live.is_empty() {
        return Ok((reply, None));
    }
    let mut unread: BTreeMap<TableId, BTreeSet<u128>> = BTreeMap::new();
    for group in live.groups.iter().filter(|group| !held.get(&group.table).is_some_and(|read| covers(read, group))) {
        unread.entry(group.table).or_default();
    }
    for group in &live.groups {
        if let Some(keys) = unread.get_mut(&group.table) {
            keys.extend(&group.keys);
        }
    }
    if !unread.is_empty() {
        let reads: Vec<Read> = unread.into_iter().map(|(table, keys)| Read { table, keys: Keys::These(keys.into_iter().collect()) }).collect();
        let read = dashboard_query::cost::storage(|| frame_at(query, run, hour, &reads, P::columns))?;
        for asked in &reads {
            frame.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
        }
    }
    let states: Vec<GroupState> = live.values(&frame).into_iter().map(GroupState::Values).collect();
    Ok((reply, Some(live.json(&states))))
}

/// A reply and the live cells it names, as the UI reads them: the reply's fields and `live`.
#[derive(Serialize)]
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

/// The warnings every reply ends with on both paths: what it refused while reading, the content's
/// warnings, then what it drew in the recording's style.
fn finish(warnings: &mut Vec<String>, own: &OwnRefusals, content: &[String]) {
    own.diagnostics().append_to(warnings);
    warnings.extend(content.iter().cloned());
    warnings.extend(own.fallback_line());
}

/// The content's warnings a reply under `load` carries: its changes since the recording, when the
/// product reads content, and the presentation's.
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

/// A request `{..params, hour}` answered as every product's is, in the transport's envelope.
fn load<P: AtHour>(source: &Source, lease_id: &str, mut request: Value, load: &LoadContext, cancelled: &AtomicBool) -> Result<String, dashboard_core::QueryError> {
    let hour = request_hour(&mut request).ok_or_else(|| format!("Invalid {} request: it names no hour", P::KEY))?;
    let params = P::Params::deserialize(&request).map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    super::reply_json(source, lease_id, P::KEY, P::CONTENT_DEPENDENT, load, cancelled, || {
        let query = crate::answer::query(source, dashboard_query::branch(&request)?)?;
        answered::<P>(&query, &params, hour, || P::context_for(source, load, &params), cancelled).map(|(reply, live)| WithLive { reply, live })
    })
}

/// `P` at the head: its reply as the panel's data, made from the playhead's rows.
struct Followed<P: AtHour> {
    params: P::Params,
    tables: Vec<TableId>,
    holders: Arc<Holders>,
    context: P::Context,
    /// The content's warnings as of the follow: a new binding follows again.
    content_warnings: Vec<String>,
    /// The live cells the last data names.
    live: Option<Live>,
    /// The recording's executable tables: the panels' thread has none in scope of its own.
    executable: Option<engine_executable::executable_tables::Tables>,
    product: PhantomData<fn() -> P>,
}

impl<P: AtHour> Panel for Followed<P> {
    fn tables(&self) -> &[TableId] {
        &self.tables
    }

    fn data(&mut self, frame: &DecodedFrame) -> PanelData {
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
        serde_json::to_value(&reply).map_err(|error| error.to_string())
    }

    fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }
}

/// `P`'s panel for the follow query `asked` (`{"panel": <key>, ..params}`), its content as `load`
/// binds it.
fn panel<P: AtHour>(source: &Source, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let mut asked = asked.clone();
    if let Some(fields) = asked.as_object_mut() {
        fields.remove("panel");
    }
    let params = P::Params::deserialize(&asked).map_err(|error| format!("Invalid {} panel: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    let branch = dashboard_query::branch(&asked).map_err(|error| error.to_string())?;
    let holders = Arc::clone(&crate::answer::query(source, branch).map_err(|error| error.to_string())?.holders);
    let context = P::context_for(source, load, &params).map_err(|error| error.to_string())?;
    Ok(Box::new(Followed::<P> { tables: P::tables(&params), params, holders, context, content_warnings: content_warnings::<P>(source, load), live: None, executable: source.tables(), product: PhantomData }))
}

#[cfg(any(test, feature = "export-types"))]
fn types<P: AtHour>() -> Vec<String> {
    let mut out = P::types();
    out.push(format!("type {0}AtHour = {0} & {{ hour: number }};", <P::Params as TS>::name()));
    out
}

/// A rows-at-an-hour product registered: its request entry and its panel.
#[derive(Clone, Copy)]
pub struct Entry {
    pub registered: Registered,
    pub panel: fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>,
}

pub const fn entry<P: AtHour>() -> Entry {
    Entry {
        registered: Registered {
            key: P::KEY,
            content_dependent: P::CONTENT_DEPENDENT,
            interactive: P::INTERACTIVE,
            replays: false,
            load: load::<P>,
            #[cfg(any(test, feature = "export-types"))]
            types: types::<P>,
        },
        panel: panel::<P>,
    }
}

/// `own` then each of `at_hour`'s registrations.
pub(crate) const fn joined<const N: usize>(own: &[Registered], at_hour: &[Entry]) -> [Registered; N] {
    assert!(own.len() + at_hour.len() == N);
    let mut out = [own[0]; N];
    let mut at = 1;
    while at < own.len() {
        out[at] = own[at];
        at += 1;
    }
    let mut at = 0;
    while at < at_hour.len() {
        out[own.len() + at] = at_hour[at].registered;
        at += 1;
    }
    out
}

/// `own` then each of `at_hour`'s keys.
pub(crate) const fn keys<const N: usize>(own: &[&'static str], at_hour: &[Entry]) -> [&'static str; N] {
    assert!(own.len() + at_hour.len() == N);
    let mut out = [""; N];
    let mut at = 0;
    while at < own.len() {
        out[at] = own[at];
        at += 1;
    }
    let mut at = 0;
    while at < at_hour.len() {
        out[own.len() + at] = at_hour[at].registered.key;
        at += 1;
    }
    out
}

/// The panel the follow query `asked` names, when a rows-at-an-hour product has its key.
pub fn panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    let kind = asked.get("panel")?.as_str()?;
    let entry = super::AT_HOUR.iter().find(|entry| entry.registered.key == kind)?;
    Some((entry.panel)(source, load, asked))
}

#[cfg(test)]
mod tests;
