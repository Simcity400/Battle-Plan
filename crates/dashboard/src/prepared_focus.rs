//! National focus prepared on its own: each country's focus order (its
//! completed focuses with their dates, the active focus and pause), when it
//! was observed, and the progress each block starts from. A block is read
//! alone ([`Builder`]) into its focus facts, which fold in block order on the
//! carried state ([`FocusIndex`]); the hourly walk builds the same facts from
//! every hour and folds them at each block's end. Focus titles, icons and
//! descriptions are not prepared: [`art`] evaluates them at the hour a reply
//! asks for, and progress at an hour is read from that hour's block.
use crate::products::{ContentOptions, LoadContext, artwork, national_focus::FocusCompletion};
use crate::{
    Artwork, FocusEntry, FocusOrder, Source,
    metadata::{CountryKeys, check_cancel},
    prepared::{
        frame_changes::FrameChanges,
        series::{self, Accumulator},
    },
};
use dashboard_base::prepared::prepass::{Held, HeldBytes};
use replay::container::Replay;
use replay_data::{rows::CompletedFocusRow, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView, frame::ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

/// Row-page size limits a read, never the number of countries or completed focuses.
const PAGE_ROWS: usize = 128;

/// Published rows and saved state; a continuation of another schema is
/// rebuilt by the pipeline.
const STATE_SCHEMA: u32 = 6;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
struct StateSchema;
impl Serialize for StateSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(STATE_SCHEMA)
    }
}
impl<'de> Deserialize<'de> for StateSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let schema = u32::deserialize(deserializer)?;
        if schema != STATE_SCHEMA {
            return Err(serde::de::Error::custom(
                "Unsupported prepared focus table/schema; rebuild focus pages",
            ));
        }
        Ok(Self)
    }
}

pub use artwork::FocusProjection as Presentation;

/// The four tables a country's focus observation reads, in this order.
const READ_TABLES: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Focus,
    TableId::CompletedFocuses,
];

/// Why a country has no focus observation at an hour (counted per country
/// and hour), or a completion has no event date (counted per completion).
const UNRESOLVED: &str = "Focus completion observation: unresolved country identity";
const NO_FOCUS_ROW: &str = "Focus completion observation: country focus row missing";
const NO_TAG: &str = "Focus completion observation: country tag is empty or not UTF-8";
const INCOMPLETE_ORDER: &str = "Focus completion observation: completed order incomplete";
const INCOMPLETE_TABLES: &str =
    "Focus completion observation: required tables incomplete (counted per hour)";
const UNDATED: &str = "Focus completion observation: completions first seen after the country was not observed have no event dates";
const RESET: &str =
    "Focus completion observation: reset or reordered completion list has no event dates";

/// Whether `count` completed focuses are recorded whole and in order.
fn complete_order(rows: &[CompletedFocusRow], count: i32) -> bool {
    usize::try_from(count).ok() == Some(rows.len())
        && rows.iter().enumerate().all(|(index, row)| {
            usize::try_from(row.ordinal).ok() == Some(index) && !row.token.is_empty()
        })
}

/// The rows of one raw owner in a complete, key-ordered CompletedFocuses table.
fn owner_range(rows: &[CompletedFocusRow], owner: u32) -> std::ops::Range<usize> {
    let start = rows.partition_point(|row| row.country < owner);
    start..start + rows[start..].partition_point(|row| row.country <= owner)
}

/// Text as replies show it: bytes that are not UTF-8 show their replacement
/// text. Never compared: two recorded tokens can share one rendering.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Recorded text by its bytes, shared rather than copied while it repeats.
/// Comparisons use the bytes; saved as a string when it is UTF-8, as its
/// bytes otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded(Arc<[u8]>);

impl Recorded {
    fn text(&self) -> String {
        lossy(&self.0)
    }

    /// `bytes` (nonempty), sharing `held` when it holds the same bytes.
    fn interned(bytes: Option<&[u8]>, held: Option<&Recorded>) -> Option<Self> {
        let bytes = bytes.filter(|bytes| !bytes.is_empty())?;
        Some(
            held.filter(|held| *held.0 == *bytes)
                .cloned()
                .unwrap_or_else(|| Self(Arc::from(bytes))),
        )
    }
}

impl Serialize for Recorded {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(&self.0) {
            Ok(text) => serializer.serialize_str(text),
            Err(_) => self.0[..].serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Recorded {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Text(String),
            Bytes(Vec<u8>),
        }
        Ok(Self(match Form::deserialize(deserializer)? {
            Form::Text(text) => Arc::from(text.into_bytes()),
            Form::Bytes(bytes) => Arc::from(bytes),
        }))
    }
}

/// The recorded bytes of an order's texts that replies show by replacement
/// text: its completed focuses that are not UTF-8 (by position), and its
/// active and continuous focus.
#[derive(Clone, Default, Serialize, Deserialize)]
struct RawOrder {
    invalid: BTreeMap<usize, Recorded>,
    current: Option<Recorded>,
    continuous: Option<Recorded>,
}

impl RawOrder {
    /// The recorded bytes of `entry`, the order's `position`th completion.
    fn token<'a>(&'a self, position: usize, entry: &'a FocusEntry) -> &'a [u8] {
        self.invalid
            .get(&position)
            .map_or(entry.token.as_bytes(), |raw| &raw.0[..])
    }

    fn note(&mut self, position: usize, token: &Recorded) {
        if std::str::from_utf8(&token.0).is_err() {
            self.invalid.insert(position, token.clone());
        }
    }
}

/// One table's refusal at an hour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusTableRefusal")]
pub struct TableRefusal {
    pub table: String,
    pub reason: Option<String>,
    pub missing_rows: Option<String>,
}

/// The refusals of the four focus tables at `frame`'s hour, none when all
/// are complete.
fn table_refusals(frame: &DecodedFrame) -> Vec<TableRefusal> {
    let mut reasons = Vec::new();
    for (table, error) in [
        (
            TableId::Countries,
            frame.complete::<tables::Countries>().err(),
        ),
        (
            TableId::CountryLinks,
            frame.complete::<tables::CountryLinks>().err(),
        ),
        (TableId::Focus, frame.complete::<tables::Focus>().err()),
        (
            TableId::CompletedFocuses,
            frame.complete::<tables::CompletedFocuses>().err(),
        ),
    ] {
        let Some(error) = error else { continue };
        let (reason, missing_rows) = match &*frame.tables[table.index()] {
            TableView::Unavailable { reason } => (Some(reason.clone()), None),
            TableView::Truncated { missing, .. } => (None, Some(missing.to_string())),
            TableView::Complete(_) => (Some(error.to_string()), None),
        };
        reasons.push(TableRefusal {
            table: format!("{table:?}"),
            reason,
            missing_rows,
        });
    }
    reasons
}

/// A country's completed focuses at an hour, its own tokens only (never the
/// whole table they were read from): the list it extends, shared, and the
/// tokens added after it. An unchanged list is the same allocation hour
/// after hour, and an extension holds only its new tokens, so a block's
/// lists take memory for the countries' completions and their changes.
#[derive(Clone)]
struct Completed {
    added: Arc<[Recorded]>,
    before: Option<Arc<Completed>>,
    len: usize,
}

impl Completed {
    /// `rows`' tokens, sharing `held` (the country's list the hour before)
    /// where they begin with its tokens.
    fn read(rows: &[CompletedFocusRow], held: Option<&Completed>) -> Self {
        let token = |row: &CompletedFocusRow| Recorded(Arc::from(&row.token[..]));
        if let Some(held) = held
            && held.len <= rows.len()
            && held
                .tokens()
                .iter()
                .zip(rows)
                .all(|(token, row)| *token.0 == *row.token)
        {
            if held.len == rows.len() {
                return held.clone();
            }
            return Self {
                added: rows[held.len..].iter().map(token).collect(),
                before: Some(Arc::new(held.clone())),
                len: rows.len(),
            };
        }
        Self {
            added: rows.iter().map(token).collect(),
            before: None,
            len: rows.len(),
        }
    }

    /// The tokens in order.
    fn tokens(&self) -> Vec<&Recorded> {
        let mut parts = Vec::new();
        let mut part = Some(self);
        while let Some(at) = part {
            parts.push(&at.added);
            part = at.before.as_deref();
        }
        parts
            .into_iter()
            .rev()
            .flat_map(|added| added.iter())
            .collect()
    }

    fn same(&self, other: &Self) -> bool {
        self.len == other.len
            && (Arc::ptr_eq(&self.added, &other.added)
                || self
                    .tokens()
                    .iter()
                    .zip(other.tokens())
                    .all(|(a, b)| a.0 == b.0))
    }
}

/// A country's focus order as recorded at an hour, compared by its recorded
/// bytes.
#[derive(Clone)]
struct OrderObs {
    completed: Completed,
    current: Option<Recorded>,
    continuous: Option<Recorded>,
    paused: bool,
}

impl OrderObs {
    fn same(&self, other: &Self) -> bool {
        self.paused == other.paused
            && self.current == other.current
            && self.continuous == other.continuous
            && self.completed.same(&other.completed)
    }

    fn current(&self) -> Option<Recorded> {
        self.current.clone()
    }
}

/// A country's focus observation at an hour, and where its completed rows
/// were in that hour's table.
#[derive(Clone)]
struct Obs {
    slot: u32,
    country_id: u32,
    progress: i64,
    order: OrderObs,
    rows: (usize, usize),
}

/// One hour's reading of the four focus tables: their refusals, each tag's
/// observation when all are complete, and the countries without one.
#[derive(Clone, Default)]
struct HourRead {
    reasons: Vec<TableRefusal>,
    tags: BTreeMap<Arc<str>, Obs>,
    tally: BTreeMap<&'static str, u64>,
}

/// Each tag's focus observation at `frame`'s hour (only `filter`'s when
/// given). A tag held by several countries is the one with the highest
/// country id, as hourly preparation always published it. Tags, texts and
/// completion lists `before` holds are shared rather than copied; with
/// `completed_held`, CompletedFocuses is the allocation `before` read.
fn read_hour(
    frame: &DecodedFrame,
    filter: Option<&str>,
    before: &HourRead,
    completed_held: bool,
) -> HourRead {
    let mut read = HourRead {
        reasons: table_refusals(frame),
        ..Default::default()
    };
    let (Ok(countries), Ok(links), Ok(focuses), Ok(completed)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::Focus>(),
        frame.complete::<tables::CompletedFocuses>(),
    ) else {
        read.tally.insert(INCOMPLETE_TABLES, 1);
        return read;
    };
    // A slot's focus row is its owner link's; a later row of one slot wins.
    let mut by_slot = BTreeMap::new();
    for focus in focuses {
        if let Some(slot) = crate::metadata::link_slot(links, focus.country) {
            by_slot.insert(slot, focus);
        }
    }
    let mut refuse = |reason: &'static str| *read.tally.entry(reason).or_default() += 1;
    let mut tags: BTreeMap<Arc<str>, Obs> = BTreeMap::new();
    for country in countries {
        if filter.is_some_and(|tag| tag.as_bytes() != country.tag.as_slice()) {
            continue;
        }
        let Ok(slot) = crate::metadata::mapped_slot(country, links) else {
            refuse(UNRESOLVED);
            continue;
        };
        let Some(focus) = by_slot.get(&slot) else {
            refuse(NO_FOCUS_ROW);
            continue;
        };
        let Some(tag) = std::str::from_utf8(&country.tag)
            .ok()
            .filter(|tag| !tag.is_empty())
        else {
            refuse(NO_TAG);
            continue;
        };
        let range = owner_range(completed, focus.country);
        if !complete_order(&completed[range.clone()], focus.completed_count) {
            refuse(INCOMPLETE_ORDER);
            continue;
        }
        let (tag, held) = match before.tags.get_key_value(tag) {
            Some((held, observation)) => (Arc::clone(held), Some(observation)),
            None => (Arc::from(tag), None),
        };
        let rows = (range.start, range.end);
        let list = match held {
            // The same rows of the same allocation: the same list.
            Some(held) if completed_held && held.rows == rows => held.order.completed.clone(),
            held => Completed::read(
                &completed[range.clone()],
                held.map(|held| &held.order.completed),
            ),
        };
        let observation = Obs {
            slot,
            country_id: country.id,
            progress: focus.progress,
            order: OrderObs {
                completed: list,
                current: Recorded::interned(
                    focus.current.as_deref(),
                    held.and_then(|held| held.order.current.as_ref()),
                ),
                continuous: Recorded::interned(
                    focus.continuous.as_deref(),
                    held.and_then(|held| held.order.continuous.as_ref()),
                ),
                paused: focus.paused != 0,
            },
            rows,
        };
        match tags.entry(tag) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(observation);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if observation.country_id > entry.get().country_id {
                    entry.insert(observation);
                }
            }
        }
    }
    read.tags = tags;
    read
}

/// A run of equal progress begun inside a stretch: its first hour, raw
/// progress and the observation the hour before (raw progress, that hour
/// and its active focus).
struct RunStart {
    first: i32,
    raw: i64,
    previous: (i64, i32, Option<Recorded>),
}

/// A country observed at consecutive recorded hours under one identity.
struct Stretch {
    first: i32,
    last: i32,
    slot: u32,
    country_id: u32,
    raw_first: i64,
    raw: i64,
    /// Its order at `first` and at each hour it changed.
    orders: Vec<(i32, OrderObs)>,
    /// The last run of equal progress begun after `first`.
    run: Option<RunStart>,
}

/// One block's focus facts, read alone (or a part of a block from its first
/// hour read): its recorded hours, the tables' refusals where they change,
/// each country's stretches and the countries without an observation.
#[derive(Default)]
pub(crate) struct Facts {
    /// Consecutive recorded hours, in order.
    runs: Vec<(i32, i32)>,
    /// The refusals at each run's first hour and where they change.
    availability: Vec<(i32, Vec<TableRefusal>)>,
    tags: BTreeMap<Arc<str>, Vec<Stretch>>,
    tally: BTreeMap<&'static str, u64>,
    /// Hours read from a frame.
    #[cfg(test)]
    pub(crate) frames: usize,
}

impl Facts {
    pub(crate) fn last_hour(&self) -> Option<i32> {
        self.runs.last().map(|run| run.1)
    }
}

fn add(tally: &mut BTreeMap<&'static str, u64>, counts: &BTreeMap<&'static str, u64>, times: u64) {
    for (reason, count) in counts {
        let total = tally.entry(reason).or_default();
        *total = total.saturating_add(count.saturating_mul(times));
    }
}

impl Held for Recorded {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.0);
    }
}

/// A completed list: its tokens added after the list before it, and that
/// list, each shared allocation once.
impl Held for Completed {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.added).heap(&self.before);
    }
}

impl Held for OrderObs {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.completed)
            .heap(&self.current)
            .heap(&self.continuous);
    }
}

impl Held for RunStart {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.previous);
    }
}

impl Held for Stretch {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.orders).heap(&self.run);
    }
}

impl Held for TableRefusal {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.table)
            .heap(&self.reason)
            .heap(&self.missing_rows);
    }
}

impl Held for Facts {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.runs)
            .heap(&self.availability)
            .heap(&self.tags)
            .heap(&self.tally);
    }
}

/// Builds one block's [`Facts`] from its hours in order: the hours whose
/// tables it is given, and the quiet hours between them, at which no field
/// it reads changed. An hour whose tables read as the hour before is quiet
/// too, so the facts are the same whether every hour comes with its tables
/// or only the hours a watched field changed.
pub(crate) struct Builder {
    filter: Option<String>,
    facts: Facts,
    last: Option<i32>,
    views: Option<[ViewIdentity; 4]>,
    keys: Option<CountryKeys>,
    read: HourRead,
    /// Tags observed at the last hour, with their open stretch.
    open: BTreeMap<Arc<str>, usize>,
}

impl Builder {
    /// What the facts built so far hold, about: each recorded run, the
    /// refusals where they change and their reasons, each country's
    /// stretches with their order changes, and the focus lists and tokens
    /// those hold (each shared list and token once).
    pub(crate) fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        held.heap(&self.facts).total()
    }

    /// A builder of every country's facts, or only `filter`'s.
    pub(crate) fn new(filter: Option<&str>) -> Self {
        Self {
            filter: filter.map(str::to_owned),
            facts: Facts::default(),
            last: None,
            views: None,
            keys: None,
            read: HourRead::default(),
            open: BTreeMap::new(),
        }
    }

    /// The next recorded hour, with its tables.
    pub(crate) fn hour(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let hour = frame.hour;
        if self.last.is_some_and(|last| last >= hour) {
            return Err(io::Error::other(
                "Focus preparation requires increasing hours in one branch",
            ));
        }
        #[cfg(test)]
        {
            self.facts.frames += 1;
        }
        let consecutive = self.last.and_then(|last| last.checked_add(1)) == Some(hour);
        // The same allocations, or Countries changed only in what the focus
        // observation does not read (its balances), read as the hour before.
        let countries = &frame.tables[TableId::Countries.index()];
        let unchanged = consecutive
            && self.views.as_ref().is_some_and(|views| {
                views[1..]
                    .iter()
                    .zip(&READ_TABLES[1..])
                    .all(|(view, table)| view.is(&frame.tables[table.index()]))
                    && (views[0].is(countries)
                        || self.keys.as_ref().is_some_and(|keys| {
                            frame
                                .complete::<tables::Countries>()
                                .is_ok_and(|rows| keys.same(rows))
                        }))
            });
        if unchanged {
            if let Some(views) = &mut self.views {
                views[0] = countries.identity();
            }
            self.quiet_hours(hour, hour);
            return Ok(());
        }
        let completed_held = self
            .views
            .as_ref()
            .is_some_and(|views| views[3].is(&frame.tables[TableId::CompletedFocuses.index()]));
        let read = read_hour(frame, self.filter.as_deref(), &self.read, completed_held);
        // Keys equal to the last read's are kept, not copied again.
        let held = self
            .views
            .as_ref()
            .is_some_and(|views| views[0].is(countries))
            || self.keys.as_ref().is_some_and(|keys| {
                frame
                    .complete::<tables::Countries>()
                    .is_ok_and(|rows| keys.same(rows))
            });
        if !held {
            self.keys = frame
                .complete::<tables::Countries>()
                .ok()
                .map(CountryKeys::read);
        }
        self.views = Some(READ_TABLES.map(|table| frame.tables[table.index()].identity()));
        self.apply(hour, consecutive, read);
        Ok(())
    }

    /// Recorded hours after the last, at which no field it reads changed.
    pub(crate) fn quiet(&mut self, hours: &[i32]) -> io::Result<()> {
        let (Some(&first), Some(&last)) = (hours.first(), hours.last()) else {
            return Ok(());
        };
        if self.last.and_then(|hour| hour.checked_add(1)) != Some(first)
            || i64::from(last) - i64::from(first) + 1 != hours.len() as i64
        {
            return Err(io::Error::other(
                "Quiet focus hours do not continue the hour before",
            ));
        }
        self.quiet_hours(first, last);
        Ok(())
    }

    fn quiet_hours(&mut self, first: i32, last: i32) {
        if let Some(run) = self.facts.runs.last_mut() {
            run.1 = last;
        }
        let hours = u64::try_from(i64::from(last) - i64::from(first) + 1).unwrap_or(0);
        add(&mut self.facts.tally, &self.read.tally, hours);
        self.last = Some(last);
    }

    fn close(&mut self, tag: &str) {
        if let (Some(index), Some(last)) = (self.open.remove(tag), self.last)
            && let Some(stretch) = self
                .facts
                .tags
                .get_mut(tag)
                .and_then(|stretches| stretches.get_mut(index))
        {
            stretch.last = last;
        }
    }

    fn close_all(&mut self) {
        let open: Vec<Arc<str>> = self.open.keys().cloned().collect();
        for tag in open {
            self.close(&tag);
        }
    }

    fn apply(&mut self, hour: i32, consecutive: bool, read: HourRead) {
        if consecutive {
            if let Some(run) = self.facts.runs.last_mut() {
                run.1 = hour;
            }
        } else {
            self.close_all();
            self.facts.runs.push((hour, hour));
        }
        if !consecutive || read.reasons != self.read.reasons {
            self.facts.availability.push((hour, read.reasons.clone()));
        }
        add(&mut self.facts.tally, &read.tally, 1);
        let absent: Vec<Arc<str>> = self
            .open
            .keys()
            .filter(|tag| !read.tags.contains_key(*tag))
            .cloned()
            .collect();
        for tag in absent {
            self.close(&tag);
        }
        for (tag, observation) in &read.tags {
            let stretches = self.facts.tags.entry(Arc::clone(tag)).or_default();
            if let Some(&index) = self.open.get(tag) {
                let stretch = &mut stretches[index];
                if (stretch.slot, stretch.country_id) == (observation.slot, observation.country_id)
                {
                    let (changed, current) = {
                        let (_, previous) = stretch.orders.last().expect("a stretch has an order");
                        (!previous.same(&observation.order), previous.current())
                    };
                    if changed {
                        stretch.orders.push((hour, observation.order.clone()));
                    }
                    if observation.progress != stretch.raw {
                        stretch.run = Some(RunStart {
                            first: hour,
                            raw: observation.progress,
                            previous: (stretch.raw, hour - 1, current),
                        });
                        stretch.raw = observation.progress;
                    }
                    continue;
                }
                // Another country holds the tag from this hour.
                stretch.last = hour - 1;
            }
            self.open.insert(Arc::clone(tag), stretches.len());
            stretches.push(Stretch {
                first: hour,
                last: hour,
                slot: observation.slot,
                country_id: observation.country_id,
                raw_first: observation.progress,
                raw: observation.progress,
                orders: vec![(hour, observation.order.clone())],
                run: None,
            });
        }
        self.read = read;
        self.last = Some(hour);
    }

    /// The facts of the hours given so far.
    pub(crate) fn finish(mut self) -> Facts {
        self.close_all();
        self.facts
    }
}

/// A country's progress as carried from hour to hour: its identity, the last
/// hour it was observed, the run of equal progress through that hour (first
/// hour, raw progress and the observation before the run: raw progress, its
/// hour and active focus, None when the run began this identity's
/// observations) and its active focus at the last hour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    slot: u32,
    country_id: u32,
    last: i32,
    first: i32,
    raw: i64,
    previous: Option<(i64, i32, Option<Recorded>)>,
    current: Option<Recorded>,
}

/// The progress after `stretch`, carried from `prior` (the country's
/// progress before it).
fn stretch_progress(prior: Option<&Progress>, stretch: &Stretch) -> Progress {
    let same =
        prior.filter(|prior| (prior.slot, prior.country_id) == (stretch.slot, stretch.country_id));
    let consecutive = same.filter(|prior| prior.last.checked_add(1) == Some(stretch.first));
    let (first, raw, previous) = if let Some(run) = &stretch.run {
        (run.first, run.raw, Some(run.previous.clone()))
    } else if let Some(prior) = consecutive.filter(|prior| prior.raw == stretch.raw_first) {
        (prior.first, prior.raw, prior.previous.clone())
    } else {
        (
            stretch.first,
            stretch.raw_first,
            same.map(|prior| (prior.raw, prior.last, prior.current.clone())),
        )
    };
    Progress {
        slot: stretch.slot,
        country_id: stretch.country_id,
        last: stretch.last,
        first,
        raw,
        previous,
        current: stretch.orders.last().and_then(|(_, order)| order.current()),
    }
}

/// The progress gained since the observation before the run, when that was
/// the hour before the run and the same focus was active then and now.
fn daily_gain(progress: &Progress) -> Option<i128> {
    let (raw, last, current) = progress.previous.as_ref()?;
    if last.checked_add(1) != Some(progress.first)
        || progress.current.is_none()
        || progress.current != *current
    {
        return None;
    }
    let gain = i128::from(progress.raw) - i128::from(*raw);
    (gain > 0).then_some(gain)
}

/// A country's published focus order from an hour: its identity, the first
/// hour of this order's consecutive run, the active focus and pause, and its
/// completed focuses as the first `len` rows of a completion chain from
/// row `chain`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderEvent {
    schema: StateSchema,
    slot: u32,
    country_id: u32,
    order_first_hour: i32,
    current: Option<String>,
    continuous: Option<String>,
    paused: bool,
    chain: u64,
    len: u64,
}

/// A tag's carried state: its series key, progress, dated order and the run
/// it began, its completion chain, and what it last published.
#[derive(Clone, Serialize, Deserialize)]
struct Track {
    key: u64,
    progress: Progress,
    order: FocusOrder,
    /// The order's recorded bytes its replacement text cannot tell apart.
    raw: RawOrder,
    order_first_hour: i32,
    /// The first chain row of the order's completed focuses.
    chain: u64,
    /// Chain rows published for this tag.
    rows: u64,
    event: Option<OrderEvent>,
    /// The progress last published at a block's first hour.
    published: Option<Progress>,
}

/// A series call the fold makes at an hour.
enum Call {
    Event(u64, Vec<u8>),
    Observe(u64, Option<i128>, bool),
    Completion(Vec<u8>),
}

/// The block being read by the hourly walk, folded at its last hour.
struct Pending {
    first: i32,
    last: i32,
    builder: Builder,
}

/// One branch's focus state and publication: its series (`focus-{branch}`:
/// key 0 the tables' refusals; for each tag's key `k`, presence runs and order
/// events, `k + 1` completion chain rows, `k + 2` the progress each block
/// starts from), its completion events (`focus-completions-{branch}`) and
/// each tag's carried state.
#[derive(Default, Serialize, Deserialize)]
pub struct FocusIndex {
    schema: StateSchema,
    branch: Option<u64>,
    series: Accumulator,
    completions: Accumulator,
    tracks: BTreeMap<String, Track>,
    reasons: Option<Vec<TableRefusal>>,
    /// The last hour folded.
    frontier: Option<i32>,
    observed: bool,
    unavailable: BTreeMap<String, u64>,
    #[serde(skip)]
    pending: Option<Pending>,
    /// Completion events in fold order.
    #[cfg(test)]
    #[serde(skip)]
    pub(crate) seen: Vec<FocusCompletion>,
}

impl FocusIndex {
    /// One recorded hour of the hourly walk: read into its block's facts,
    /// folded at the block's last hour.
    pub(crate) fn observe_frame(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let bounds = match &self.pending {
            Some(pending) if pending.first <= frame.hour && frame.hour <= pending.last => {
                (pending.first, pending.last)
            }
            _ => replay
                .index
                .iter()
                .find(|block| {
                    block.branch == frame.branch
                        && block.first_hour <= frame.hour
                        && frame.hour <= block.last_hour
                })
                .map_or((frame.hour, frame.hour), |block| {
                    (block.first_hour, block.last_hour)
                }),
        };
        self.observe_hour(frame.branch, bounds, frame, &mut || check_cancel(cancelled))
    }

    /// One recorded hour of the block `bounds` (its first and last hour).
    pub(crate) fn observe_hour(
        &mut self,
        branch: u64,
        bounds: (i32, i32),
        frame: &DecodedFrame,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if self.branch.is_some_and(|held| held != branch)
            || self
                .pending
                .as_ref()
                .map_or(self.frontier, |pending| pending.builder.last)
                .is_some_and(|last| last >= frame.hour)
        {
            return Err(io::Error::other(
                "Focus preparation requires increasing hours in one branch",
            ));
        }
        self.branch = Some(branch);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.first != bounds.0)
        {
            self.fold_pending(check)?;
        }
        let pending = self.pending.get_or_insert_with(|| Pending {
            first: bounds.0,
            last: bounds.1,
            builder: Builder::new(None),
        });
        pending.builder.hour(frame)?;
        if frame.hour >= pending.last {
            self.fold_pending(check)?;
        }
        Ok(())
    }

    fn fold_pending(&mut self, check: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
        if let Some(pending) = self.pending.take() {
            self.fold(pending.builder.finish(), check)?;
        }
        Ok(())
    }

    /// Fold one block's facts read alone, after the blocks before it.
    pub(crate) fn fold_block(
        &mut self,
        branch: u64,
        facts: Facts,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if self.branch.is_some_and(|held| held != branch) {
            return Err(io::Error::other(
                "Focus preparation requires increasing hours in one branch",
            ));
        }
        self.branch = Some(branch);
        self.fold_pending(check)?;
        self.fold(facts, check)
    }

    /// Fold facts that follow the frontier: every series call they make, in
    /// hour order, on the carried state.
    fn fold(&mut self, facts: Facts, check: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
        let Facts {
            runs,
            availability,
            tags,
            tally,
            ..
        } = facts;
        let (Some(&(first, _)), Some(&(_, end))) = (runs.first(), runs.last()) else {
            return Ok(());
        };
        if self.frontier.is_some_and(|frontier| frontier >= first) {
            return Err(io::Error::other(
                "Focus preparation requires increasing hours in one branch",
            ));
        }
        let continues = self.frontier.and_then(|hour| hour.checked_add(1)) == Some(first);
        for (reason, count) in tally {
            let total = self.unavailable.entry(reason.to_owned()).or_default();
            *total = total.saturating_add(count);
        }
        let mut calls: Vec<(i32, Call)> = Vec::new();
        let starts: BTreeSet<i32> = runs.iter().map(|run| run.0).collect();
        for (hour, reasons) in availability {
            let discontinuous = starts.contains(&hour) && !(hour == first && continues);
            let changed = self.reasons.as_ref() != Some(&reasons);
            if changed {
                calls.push((hour, Call::Event(0, serde_json::to_vec(&reasons)?)));
                self.reasons = Some(reasons);
            }
            if changed || discontinuous {
                calls.push((hour, Call::Observe(0, Some(0), true)));
            }
        }
        // A tag observed at the frontier and not at the next hour ends there.
        if continues && let Some(frontier) = self.frontier {
            for (tag, track) in &self.tracks {
                if track.progress.last == frontier
                    && tags
                        .get(tag.as_str())
                        .is_none_or(|stretches| stretches[0].first != first)
                {
                    calls.push((first, Call::Observe(track.key, None, false)));
                }
            }
        }
        let run_end = |hour: i32| -> i32 {
            let index = runs.partition_point(|run| run.0 <= hour);
            runs[index - 1].1
        };
        let mut resets = 0_u64;
        let mut undated = 0_u64;
        for (tag, stretches) in tags {
            check()?;
            self.observed = true;
            let key = match self.tracks.get(&*tag) {
                Some(track) => track.key,
                None => 1 + 3 * self.tracks.len() as u64,
            };
            let mut track = self.tracks.remove(&*tag);
            if let Some(track) = &mut track
                && track.published.as_ref() != Some(&track.progress)
            {
                calls.push((
                    first,
                    Call::Event(key + 2, serde_json::to_vec(&track.progress)?),
                ));
                track.published = Some(track.progress.clone());
            }
            for (position, stretch) in stretches.iter().enumerate() {
                let next = self.fold_stretch(
                    &tag,
                    key,
                    track,
                    stretch,
                    &mut calls,
                    &mut resets,
                    &mut undated,
                )?;
                track = Some(next);
                // Not observed at the next recorded hour: its presence ends.
                if stretch.last < run_end(stretch.last)
                    && stretches
                        .get(position + 1)
                        .is_none_or(|following| following.first != stretch.last + 1)
                {
                    calls.push((stretch.last + 1, Call::Observe(key, None, false)));
                }
            }
            if let Some(track) = track {
                self.tracks.insert(tag.to_string(), track);
            }
        }
        for (reason, count) in [(RESET, resets), (UNDATED, undated)] {
            if count > 0 {
                let total = self.unavailable.entry(reason.to_owned()).or_default();
                *total = total.saturating_add(count);
            }
        }
        calls.sort_by_key(|(hour, _)| *hour);
        let mut calls = calls.into_iter().peekable();
        for &(run_first, run_last) in &runs {
            let mut next = run_first;
            while calls.peek().is_some_and(|(hour, _)| *hour <= run_last) {
                let (hour, call) = calls.next().expect("a peeked call");
                if hour >= next {
                    self.advance(next, hour)?;
                    next = hour.saturating_add(1);
                }
                match call {
                    Call::Event(key, row) => self.series.event_raw(key, hour, row)?,
                    Call::Observe(key, value, starts) => {
                        self.series.observe(key, hour, value, starts)
                    }
                    Call::Completion(row) => self.completions.event_raw(0, hour, row)?,
                }
            }
            if next <= run_last {
                self.advance(next, run_last)?;
            }
        }
        if calls.next().is_some() {
            return Err(io::Error::other("Focus call outside its recorded hours"));
        }
        self.frontier = Some(end);
        Ok(())
    }

    fn advance(&mut self, first: i32, last: i32) -> io::Result<()> {
        self.series.advance_span(first, last)?;
        self.completions.advance_span(first, last)
    }

    /// One stretch of `tag` on its track before it: its order events,
    /// completion chain rows and completions, and its presence's start.
    #[allow(clippy::too_many_arguments)]
    fn fold_stretch(
        &mut self,
        tag: &str,
        key: u64,
        prior: Option<Track>,
        stretch: &Stretch,
        calls: &mut Vec<(i32, Call)>,
        resets: &mut u64,
        undated: &mut u64,
    ) -> io::Result<Track> {
        let progress = stretch_progress(prior.as_ref().map(|track| &track.progress), stretch);
        let same = prior.as_ref().is_some_and(|track| {
            (track.progress.slot, track.progress.country_id) == (stretch.slot, stretch.country_id)
        });
        let consecutive = same
            && prior
                .as_ref()
                .is_some_and(|track| track.progress.last.checked_add(1) == Some(stretch.first));
        let (mut order, mut raw, mut order_first_hour, mut chain, mut rows, mut event, published) =
            match prior {
                Some(track) => (
                    same.then_some(track.order),
                    if same { track.raw } else { RawOrder::default() },
                    track.order_first_hour,
                    track.chain,
                    track.rows,
                    track.event,
                    track.published,
                ),
                None => (None, RawOrder::default(), stretch.first, 0, 0, None, None),
            };
        if !consecutive {
            calls.push((stretch.first, Call::Observe(key, Some(1), true)));
        }
        for (position, (hour, observed)) in stretch.orders.iter().enumerate() {
            let hour = *hour;
            // The order before this one was observed the hour before.
            let continuing = position > 0 || consecutive;
            let tokens = observed.completed.tokens();
            // Compared by the recorded bytes, shown by their text.
            let current = observed.current();
            let continuous = observed.continuous.clone();
            let extends =
                order.as_ref().is_some_and(|before| {
                    before.completed.len() <= tokens.len()
                        && before.completed.iter().zip(&tokens).enumerate().all(
                            |(position, (entry, row))| raw.token(position, entry) == &row.0[..],
                        )
                });
            let unchanged = extends
                && order.as_ref().is_some_and(|before| {
                    before.completed.len() == tokens.len() && before.paused == observed.paused
                })
                && raw.current == current
                && raw.continuous == continuous;
            let mut next = match order.take() {
                Some(before) if extends => before,
                before => {
                    if before.is_some() {
                        *resets += 1;
                    }
                    chain = rows;
                    raw.invalid.clear();
                    for (position, row) in tokens.iter().enumerate() {
                        raw.note(position, row);
                    }
                    let completed: Vec<FocusEntry> = tokens
                        .iter()
                        .enumerate()
                        .map(|(position, row)| FocusEntry {
                            ordinal: position as u32,
                            token: row.text(),
                            completed_by_hour: None,
                        })
                        .collect();
                    for entry in &completed {
                        calls.push((hour, Call::Event(key + 1, serde_json::to_vec(entry)?)));
                        rows += 1;
                    }
                    FocusOrder {
                        completed,
                        current: None,
                        continuous: None,
                        paused: false,
                    }
                }
            };
            for (position, row) in tokens.iter().enumerate().skip(next.completed.len()) {
                raw.note(position, row);
                let entry = FocusEntry {
                    ordinal: position as u32,
                    token: row.text(),
                    completed_by_hour: Some(hour),
                };
                calls.push((hour, Call::Event(key + 1, serde_json::to_vec(&entry)?)));
                rows += 1;
                if continuing {
                    let completion = FocusCompletion {
                        tag: tag.to_owned(),
                        token: entry.token.clone(),
                        hour,
                    };
                    calls.push((hour, Call::Completion(serde_json::to_vec(&completion)?)));
                    #[cfg(test)]
                    self.seen.push(completion);
                } else {
                    *undated += 1;
                }
                next.completed.push(entry);
            }
            next.current = current.as_ref().map(Recorded::text);
            next.continuous = continuous.as_ref().map(Recorded::text);
            next.paused = observed.paused;
            raw.current = current;
            raw.continuous = continuous;
            if !(continuing && unchanged) {
                order_first_hour = hour;
            }
            let published_order = OrderEvent {
                schema: StateSchema,
                slot: stretch.slot,
                country_id: stretch.country_id,
                order_first_hour,
                current: next.current.clone(),
                continuous: next.continuous.clone(),
                paused: next.paused,
                chain,
                len: next.completed.len() as u64,
            };
            if event.as_ref() != Some(&published_order) {
                calls.push((
                    hour,
                    Call::Event(key, serde_json::to_vec(&published_order)?),
                ));
                event = Some(published_order);
            }
            order = Some(next);
        }
        Ok(Track {
            key,
            progress,
            order: order.expect("a stretch has an order"),
            raw,
            order_first_hour,
            chain,
            rows,
            event,
            published,
        })
    }

    /// Fold the block being read, through its last hour given.
    #[cfg(test)]
    pub(crate) fn fold_held(&mut self, cancelled: &AtomicBool) -> io::Result<()> {
        self.fold_pending(&mut || check_cancel(cancelled))
    }

    /// Whether any country was observed, and the countries and completions
    /// without an event date by reason.
    #[cfg(test)]
    pub(crate) fn completion_metadata(&self) -> (bool, &BTreeMap<String, u64>) {
        (self.observed, &self.unavailable)
    }

    /// Bytes buffered for the next flush.
    pub fn buffered_bytes(&self) -> usize {
        self.series.bytes() + self.completions.bytes()
    }

    /// Kept for the hourly walk's call site: completions are folded with the
    /// order ([`Self::observe_frame`]).
    pub(crate) fn observe_completions(
        &mut self,
        _frame: &DecodedFrame,
        _source: &Source,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)
    }

    /// Publish what was folded: a block the hourly walk is still reading is
    /// folded first, through its last hour given.
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        self.fold_pending(&mut || check_cancel(cancelled))?;
        let Some(branch) = self.branch else {
            return Ok(());
        };
        self.completions
            .flush(writer, &format!("focus-completions-{branch}"), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "focus-completions",
            branch,
            &BTreeMap::new(),
            &(
                self.observed,
                &self.unavailable,
                series::LateIndex::default(),
            ),
        )?;
        self.series
            .flush(writer, &format!("focus-{branch}"), 0, cancelled)?;
        let catalogue: BTreeMap<&String, u64> = self
            .tracks
            .iter()
            .map(|(tag, track)| (tag, track.key))
            .collect();
        writer.write_at(
            &format!("focus-{branch}-catalogue"),
            0,
            serde_json::to_vec(&catalogue)?.as_slice(),
        )
    }
}

/// The hourly walk's focus state in its branch state; everything focus
/// carries is in [`FocusIndex`].
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct FocusAccumulator {
    schema: StateSchema,
}

/// The hourly walk's focus observer: each hour goes to [`FocusIndex`].
/// Presentation is not prepared, so it binds no content.
#[derive(Default)]
pub struct FocusEvaluator;

impl FocusEvaluator {
    pub(crate) fn configure(
        &mut self,
        _source: &Source,
        _content: &ContentOptions,
    ) -> io::Result<()> {
        Ok(())
    }

    /// Observe one hour; it never counts a display failure.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_frame(
        &mut self,
        _writer: &mut store::Writer,
        source: &Source,
        _content: Option<&ContentOptions>,
        frame: &DecodedFrame,
        _accumulator: &mut FocusAccumulator,
        index: &mut FocusIndex,
        _preferred: Option<&str>,
        cancelled: &AtomicBool,
        _changes: &FrameChanges,
    ) -> io::Result<(u64, Option<String>)> {
        index.observe_frame(source.replay_ref(), frame, cancelled)?;
        Ok((0, None))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusArt")]
pub struct PresentedFocus {
    pub artwork: Artwork,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusPages")]
pub struct Pages {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<_>")]
    pub sampled: bool,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub first: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub count: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub rows: u64,
}

mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A country's focus at an hour: its order fields hold from `hour` through
/// `lastHour`; progress and its daily gain are those at `at`, the hour asked.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusHour")]
pub struct Hour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub slot: u32,
    pub country_id: u32,
    pub hour: i32,
    pub last_hour: i32,
    pub order_first_hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing: Vec<String>,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
    pub completed: Pages,
    pub at: i32,
    pub progress_raw: String,
    pub daily_gain_raw: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusUnavailableHour")]
pub struct UnavailableHour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub first_hour: i32,
    pub last_hour: i32,
    pub reasons: Vec<TableRefusal>,
}

pub mod art;
pub(crate) mod family;
mod query;
pub use art::{FocusArt, FocusArtFailure, FocusArtMode, FocusArtParams, FocusArtReply};
pub use query::{order_page, query_hour, query_unavailable, sampled_hour, sampled_page};

/// The runner unions these inputs with scripted-text inputs and Clock before decoding.
pub const TABLES: &[TableId] = crate::products::national_focus::FOCUS_COMPLETION_TABLES;
#[cfg(test)]
mod tests;
