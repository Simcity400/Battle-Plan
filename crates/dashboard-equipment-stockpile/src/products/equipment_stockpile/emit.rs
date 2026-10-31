//! The stockpile's emit: each holder's daily stock and production lines and its logistics
//! record as snapshot events on the family's series publication, each stream with its coverage,
//! the tables' read schedules, and each variant's names and class and each logistics token's
//! spelling as they are first read or change.
//!
//! A read that changes a holder's stream writes one event: a keyframe of every row, or a delta
//! of the rows changed since the event before, whose first cell is its keyframe's hour. A chain
//! is a keyframe and at most `CHAIN - 1` deltas. A stream's coverage starting (at a read of a
//! usable table by a resolved holder) always writes a keyframe; its holder's identity changing,
//! its table stopping being usable or an hour not walked ends it.

use super::columnar::{self, Facts, Holders, Read, Totals, Variant};
use dashboard_base::{
    catalogue,
    products::equipment::{self, Domain, VariantHour, archetype_of},
    source::Source,
};
use dashboard_core::config::ContentOptions;
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::{self, Carry},
    codec::Decode,
    family::{Actors, Directory, Place},
    hash::FastMap,
};
use installed_content_core::equipment::Database;
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::TableId,
};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;

/// A holder's keys: its daily and logistics streams, its read counts, and each stream's
/// coverage as the evidence of a key of its own.
pub const DAILY: u64 = 0;
pub const LOGISTICS: u64 = 1;
pub const COUNTS: u64 = 2;
pub const STOCK_HOURS: u64 = 3;
pub const LINES_HOURS: u64 = 4;
pub const LOGISTICS_HOURS: u64 = 5;

/// The family's keys: each stream's read schedule, each variant's names and class (by
/// variant), the logistics tokens' spellings and the hours every name is forgotten.
pub const DAILY_READS: u64 = 0;
pub const LOGISTICS_READS: u64 = 1;
pub const NAMES: u64 = 2;
pub const CLASSES: u64 = 3;
pub const TOKENS: u64 = 4;
pub const FORGETS: u64 = 5;

/// Event kinds.
pub mod kind {
    /// `[Unsigned slot, Unsigned id, rows…]`: every row of the stream.
    pub const KEYFRAME: u64 = 0;
    /// `[Integer keyframe hour, rows…]`: the rows changed since the event before; a removed
    /// row is absent with null values.
    pub const DELTA: u64 = 1;
    /// The 11 read counts, each Unsigned, as of the holder's latest read of each table.
    pub const COUNTS: u64 = 2;
    /// `[Text RawVariant JSON]`, on the variant's NAMES key.
    pub const NAME: u64 = 3;
    /// `[]`: every variant's names are unknown from here until its next NAME.
    pub const FORGET: u64 = 4;
    /// `[Text token, Text archetype | Null, Unsigned domain | Null]`, on the variant's CLASSES
    /// key, where its latest nonempty token changes.
    pub const CLASS: u64 = 5;
    /// `[Unsigned token, Text spelling, Text archetype | Null, Unsigned domain | Null]`.
    pub const TOKEN: u64 = 6;
}

/// Why a stream's coverage is unknown.
pub mod reason {
    /// Its table does not hold every row.
    pub const TABLE: u64 = 1;
    /// Countries or CountryLinks does not, so no tag resolves.
    pub const COUNTRIES: u64 = 2;
    /// No country the links resolve holds the tag.
    pub const UNRESOLVED: u64 = 3;
    /// It is usable, but not read since it could be.
    pub const UNREAD: u64 = 4;
}

/// The schedules' evidence: every walked hour is known, as the engine requires of a run.
const WALKED: carry::Read = carry::Read {
    holder: None,
    read: None,
    identity_first: None,
    presented: false,
};

/// Events in a chain, its keyframe's included.
pub const CHAIN: u8 = 16;
/// Cells of a daily and of a logistics row.
pub const DAILY_CELLS: usize = 8;
pub const LOGISTICS_CELLS: usize = 5;

/// A domain's number in classes and tokens.
pub fn domain(domain: Domain) -> u64 {
    match domain {
        Domain::Land => 0,
        Domain::Air => 1,
        Domain::Naval => 2,
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// An equipment token's type as content resolves it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Kind {
    key: String,
    domain: Domain,
}

/// The recorded mod stack's equipment types, shared by every branch of one preparation; none
/// where content is unavailable, which resolves no type.
#[derive(Clone, Default)]
pub struct Content {
    database: Option<Arc<Database>>,
    kinds: Option<Arc<catalogue::KeyedResults<String, Kind>>>,
}

impl Content {
    pub fn load(source: &Source, options: Option<&ContentOptions>) -> Self {
        let Some((catalogue, database)) = options
            .and_then(|options| catalogue::catalogue(source, options).ok())
            .and_then(|catalogue| {
                equipment::units(source, &catalogue)
                    .ok()
                    .map(|database| (catalogue, database))
            })
        else {
            return Self::default();
        };
        let kinds = catalogue
            .content
            .layers
            .derived::<catalogue::KeyedResults<String, Kind>>();
        Self {
            database: Some(database),
            kinds: Some(kinds),
        }
    }

    /// Content whose authored types are those given, by token: every other is unresolved.
    #[cfg(test)]
    pub(crate) fn resolved(kinds: &[(&str, &str, Domain)]) -> Self {
        let results = catalogue::KeyedResults::default();
        for &(token, key, domain) in kinds {
            results
                .get_or_init(&token.to_owned(), |_| {
                    Ok(Kind {
                        key: key.into(),
                        domain,
                    })
                })
                .unwrap();
        }
        Self {
            database: Some(Arc::new(Database::default())),
            kinds: Some(Arc::new(results)),
        }
    }

    fn kind(&self, token: &str) -> Option<Kind> {
        let (database, kinds) = self.database.as_ref().zip(self.kinds.as_ref())?;
        kinds
            .get_or_init(&token.to_owned(), |token| {
                let (key, _, domain) = archetype_of(database, token).map_err(io::Error::other)?;
                Ok(Kind { key, domain })
            })
            .ok()
            .map(|kind| (*kind).clone())
    }
}

/// A stream's coverage from an hour on: known as of a holder (its slot and id packed), or
/// unknown for a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Known(u64),
    Unknown(u64),
}

pub(crate) fn packed((slot, id): (u32, u32)) -> u64 {
    u64::from(slot) << 32 | u64::from(id)
}

/// A coverage key's open evidence: its status and the first hour not yet appended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Coverage {
    open: Option<(Status, i32)>,
}

impl Coverage {
    fn known(self) -> bool {
        matches!(self.open, Some((Status::Known(_), _)))
    }

    fn unread(self) -> bool {
        matches!(self.open, Some((Status::Unknown(reason::UNREAD), _)))
    }

    /// `status` from `hour` on; one held already changes nothing.
    fn set(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        hour: i32,
        status: Status,
    ) -> io::Result<()> {
        if let Some((held, from)) = self.open {
            if held == status {
                return Ok(());
            }
            if from < hour {
                append(writer, key, held, from, hour - 1)?;
            }
        }
        self.open = Some((status, hour));
        Ok(())
    }

    /// Appends what is open through `last`, which stays open.
    fn cut(&mut self, writer: &mut Writer<'_>, key: KeyHandle, last: i32) -> io::Result<()> {
        if let Some((held, from)) = &mut self.open
            && *from <= last
        {
            append(writer, key, *held, *from, last)?;
            *from = last + 1;
        }
        Ok(())
    }
}

fn append(
    writer: &mut Writer<'_>,
    key: KeyHandle,
    status: Status,
    first: i32,
    last: i32,
) -> io::Result<()> {
    let (known, holder) = match status {
        Status::Known(holder) => (Knownness::Known, Some(holder)),
        Status::Unknown(reason) => (Knownness::Unknown(reason), None),
    };
    writer.evidence(
        key,
        Evidence {
            span: Span::new(first, last)?,
            known,
            supplying_read: None,
            holder,
            identity_first: None,
        },
        None,
    )
}

/// A variant's daily row: its stock (absent where it has no stock row, unknown where its sum
/// does not fit), and its lines' units and IC a day and lowest and highest cost per unit
/// (absent where it has no line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Daily {
    stock: Option<Option<i64>>,
    lines: Option<[Option<i64>; 4]>,
}

impl Daily {
    const NONE: Self = Self {
        stock: None,
        lines: None,
    };
}

/// An archetype token's logistics row: its balance, stock and available stock.
type Logistics = [Option<i64>; 3];

fn amount(value: Option<i64>) -> Cell {
    value.map_or(Cell::Null, |value| Cell::Integer(i128::from(value)))
}

/// A stream's row.
trait Row: Copy {
    /// The cells of the row of `key`; none is the row a delta removes.
    fn cells(row: Option<&Self>, key: u64, out: &mut Vec<Cell>);
}

impl Row for Daily {
    fn cells(row: Option<&Self>, key: u64, out: &mut Vec<Cell>) {
        let stock = row.and_then(|row| row.stock);
        let lines = row.and_then(|row| row.lines);
        let line = |at: usize| amount(lines.and_then(|lines| lines[at]));
        out.extend([
            Cell::Unsigned(key),
            amount(stock.flatten()),
            Cell::Boolean(stock.is_some()),
            Cell::Boolean(lines.is_some()),
            line(0),
            line(1),
            line(2),
            line(3),
        ]);
    }
}

impl Row for Logistics {
    fn cells(row: Option<&Self>, key: u64, out: &mut Vec<Cell>) {
        let value = |at: usize| amount(row.and_then(|row| row[at]));
        out.extend([
            Cell::Unsigned(key),
            value(0),
            value(1),
            value(2),
            Cell::Boolean(row.is_some()),
        ]);
    }
}

/// A stream as its last event left it, by key, and its chain: the keyframe's hour and the
/// events in it; none where the next event is a keyframe.
#[derive(Debug)]
struct Stream<R> {
    rows: BTreeMap<u64, R>,
    chain: Option<(i32, u8)>,
}

impl<R> Default for Stream<R> {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            chain: None,
        }
    }
}

impl<R: Row> Stream<R> {
    /// Writes the event of a read at `hour` that changed `changed` (ascending), a keyframe
    /// where `starts` or the chain needs one.
    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        hour: i32,
        (slot, id): (u32, u32),
        changed: &[u64],
        starts: bool,
        cells: &mut Vec<Cell>,
    ) -> io::Result<()> {
        cells.clear();
        let shape = match self.chain {
            Some((first, events)) if !starts && events < CHAIN => {
                engine_allocation::reserve(cells, 1 + changed.len() * DAILY_CELLS)?;
                cells.push(Cell::Integer(i128::from(first)));
                for &entry in changed {
                    R::cells(self.rows.get(&entry), entry, cells);
                }
                self.chain = Some((first, events + 1));
                kind::DELTA
            }
            _ => {
                engine_allocation::reserve(cells, 2 + self.rows.len() * DAILY_CELLS)?;
                cells.push(Cell::Unsigned(u64::from(slot)));
                cells.push(Cell::Unsigned(u64::from(id)));
                for (&entry, row) in &self.rows {
                    R::cells(Some(row), entry, cells);
                }
                self.chain = Some((hour, 1));
                kind::KEYFRAME
            }
        };
        let fields = writer.cells(cells.drain(..))?;
        event(writer, key, hour, Some(hour), shape, fields)
    }
}

fn event(
    writer: &mut Writer<'_>,
    key: KeyHandle,
    hour: i32,
    read: Option<i32>,
    kind: u64,
    fields: Blob,
) -> io::Result<()> {
    let sequence = writer.sequence()?;
    writer.event(Event {
        key,
        native_hour: hour,
        read_hour: read,
        sequence,
        kind,
        fields,
    })
}

fn mismatch() -> io::Error {
    invalid("A stockpile read changed totals it did not hold")
}

/// Sets one part of `variant`'s row, adding the row or dropping it where it holds no part. A
/// change of a part not held, or to what it holds, is not the projector's.
fn set_part<P: Copy + PartialEq>(
    rows: &mut BTreeMap<u64, Daily>,
    variant: u64,
    value: Option<P>,
    part: fn(&mut Daily) -> &mut Option<P>,
) -> io::Result<()> {
    match rows.entry(variant) {
        Entry::Occupied(mut held) => {
            let row = held.get_mut();
            if *part(row) == value {
                return Err(mismatch());
            }
            *part(row) = value;
            if *row == Daily::NONE {
                held.remove();
            }
        }
        Entry::Vacant(free) => {
            if value.is_none() {
                return Err(mismatch());
            }
            let mut row = Daily::NONE;
            *part(&mut row) = value;
            free.insert(row);
        }
    }
    Ok(())
}

fn stock_part(row: &mut Daily) -> &mut Option<Option<i64>> {
    &mut row.stock
}

fn lines_part(row: &mut Daily) -> &mut Option<[Option<i64>; 4]> {
    &mut row.lines
}

/// `totals` applied to one part of `rows`, noting each variant it changed.
fn apply_part<const N: usize, P: Copy + PartialEq>(
    rows: &mut BTreeMap<u64, Daily>,
    totals: &Totals<u64, N>,
    value: fn([Option<i64>; N]) -> P,
    part: fn(&mut Daily) -> &mut Option<P>,
    changed: &mut Vec<u64>,
) -> io::Result<()> {
    let set = match totals {
        Totals::Changed(changes) => {
            engine_allocation::reserve(changed, changes.len())?;
            for &(variant, values) in changes {
                set_part(rows, variant, values.map(value), part)?;
                changed.push(variant);
            }
            return Ok(());
        }
        // A whole read sets what it gives and ends what it does not, walked in key order
        // beside what is held: a segment's first read, mostly what is held.
        Totals::Whole(whole) => {
            let mut held = rows
                .iter_mut()
                .filter_map(|(&variant, row)| (*part(row)).map(|held| (variant, held)))
                .peekable();
            let mut given = whole
                .iter()
                .map(|&(variant, values)| (variant, value(values)))
                .peekable();
            let mut set = Vec::new();
            loop {
                let next = match (held.peek(), given.peek()) {
                    (None, None) => break,
                    (Some(&(variant, _)), None) => (variant, None),
                    (None, Some(&(variant, new))) => (variant, Some(new)),
                    (Some(&(was, old)), Some(&(variant, new))) => match was.cmp(&variant) {
                        std::cmp::Ordering::Less => (was, None),
                        std::cmp::Ordering::Greater => (variant, Some(new)),
                        std::cmp::Ordering::Equal => {
                            held.next();
                            given.next();
                            if old != new {
                                engine_allocation::reserve(&mut set, 1)?;
                                set.push((variant, Some(new)));
                            }
                            continue;
                        }
                    },
                };
                match next.1 {
                    None => held.next(),
                    Some(_) => given.next(),
                };
                engine_allocation::reserve(&mut set, 1)?;
                set.push(next);
            }
            set
        }
    };
    engine_allocation::reserve(changed, set.len())?;
    for (variant, new) in set {
        set_part(rows, variant, new, part)?;
        changed.push(variant);
    }
    Ok(())
}

/// Sets the logistics row of `token`, or ends it; a change to what it holds, or of a row not
/// held, is not the projector's.
fn set_logistics(
    rows: &mut BTreeMap<u64, Logistics>,
    token: u64,
    values: Option<Logistics>,
    changed: &mut Vec<u64>,
) -> io::Result<()> {
    match (rows.entry(token), values) {
        (Entry::Occupied(mut held), Some(values)) if *held.get() != values => {
            held.insert(values);
        }
        (Entry::Occupied(held), None) => {
            held.remove();
        }
        (Entry::Vacant(free), Some(values)) => {
            free.insert(values);
        }
        _ => return Err(mismatch()),
    }
    engine_allocation::reserve(changed, 1)?;
    changed.push(token);
    Ok(())
}

/// `totals` applied to a holder's logistics rows, noting each token it changed.
fn apply_logistics(
    rows: &mut BTreeMap<u64, Logistics>,
    totals: &Totals<u32, 3>,
    changed: &mut Vec<u64>,
) -> io::Result<()> {
    match totals {
        Totals::Changed(changes) => {
            for &(token, values) in changes {
                set_logistics(rows, u64::from(token), values, changed)?;
            }
        }
        // As a daily part's whole read.
        Totals::Whole(whole) => {
            let mut set = Vec::new();
            {
                let mut held = rows.iter().peekable();
                let mut given = whole
                    .iter()
                    .map(|&(token, values)| (u64::from(token), values))
                    .peekable();
                loop {
                    let next = match (held.peek(), given.peek()) {
                        (None, None) => break,
                        (Some(&(&token, _)), None) => (token, None),
                        (None, Some(&(token, new))) => (token, Some(new)),
                        (Some(&(&was, &old)), Some(&(token, new))) => match was.cmp(&token) {
                            std::cmp::Ordering::Less => (was, None),
                            std::cmp::Ordering::Greater => (token, Some(new)),
                            std::cmp::Ordering::Equal => {
                                held.next();
                                given.next();
                                if old != new {
                                    engine_allocation::reserve(&mut set, 1)?;
                                    set.push((token, Some(new)));
                                }
                                continue;
                            }
                        },
                    };
                    match next.1 {
                        None => held.next().map(drop),
                        Some(_) => given.next().map(drop),
                    };
                    engine_allocation::reserve(&mut set, 1)?;
                    set.push(next);
                }
            }
            for (token, values) in set {
                set_logistics(rows, token, values, changed)?;
            }
        }
    }
    Ok(())
}

/// One holder tag: its keys, its identity as of the last whole hour, each stream and each
/// coverage (stock, lines, logistics), and the counts of its last COUNTS event.
#[derive(Debug)]
struct Holder {
    keys: [KeyHandle; 6],
    identity: Option<(u32, u32)>,
    /// The last whole hour that resolved it, by its epoch.
    seen: u64,
    daily: Stream<Daily>,
    logistics: Stream<Logistics>,
    coverage: [Coverage; 3],
    counts: [u64; 11],
}

impl Holder {
    fn new(keys: [KeyHandle; 6]) -> Self {
        Self {
            keys,
            identity: None,
            seen: 0,
            daily: Stream::default(),
            logistics: Stream::default(),
            coverage: [Coverage::default(); 3],
            counts: [0; 11],
        }
    }

    /// Coverage `at` of `stream` is no longer known: its rows go, and its next event is a
    /// keyframe.
    fn forget(&mut self, stream: usize) {
        match stream {
            0 | 1 => {
                self.daily.chain = None;
                self.daily.rows.retain(|_, row| {
                    if stream == 0 {
                        row.stock = None;
                    } else {
                        row.lines = None;
                    }
                    *row != Daily::NONE
                });
            }
            _ => {
                self.logistics.chain = None;
                self.logistics.rows.clear();
            }
        }
    }
}

/// The variants named since the last forget, and their latest nonempty tokens.
#[derive(Default)]
struct Names {
    generation: u64,
    /// Whether a variant was named since the last forget.
    named: bool,
    variants: FastMap<u64, Named>,
    /// Tokens by number, and their numbers.
    tokens: Vec<Box<str>>,
    numbered: FastMap<Box<str>, u32>,
}

/// A variant's last naming: the forget generation it is of, its operands' digest and its
/// latest nonempty token's number, `u32::MAX` for none.
#[derive(Clone, Copy)]
struct Named {
    generation: u64,
    digest: u128,
    token: u32,
}

const NO_TOKEN: u32 = u32::MAX;

/// A stream's read schedule: the hour its next read is due on its schedule.
fn period(table: TableId, format: FormatVersion) -> i32 {
    match cadence::cadence_in(table, format) {
        Cadence::Daily(_) => cadence::STAGGER_PERIOD,
        Cadence::Hourly | Cadence::Sliced => 1,
        Cadence::Static => 0,
    }
}

/// The family's emit on one branch.
pub struct Emit {
    content: Content,
    /// Each stream's period by its table's cadence: 0 where only off-slot reads read it.
    periods: [i32; 2],
    actors: Actors,
    carry: Carry,
    family: Option<[KeyHandle; 4]>,
    /// By actor.
    holders: Vec<Holder>,
    /// Each resolved holder by canonical slot, ascending.
    slots: Vec<(u32, u32)>,
    countries: bool,
    usable: [bool; 3],
    /// The holders waiting for a read of each stream, which its next read must give.
    waiting: [usize; 3],
    epoch: u64,
    last: Option<i32>,
    /// Whether the schedules' evidence is open: from the first hour after a break.
    walking: bool,
    /// Each schedule's next read.
    next: [Option<i32>; 2],
    names: Names,
    /// Tokens whose spelling was written.
    spelled: FastMap<u32, ()>,
    cells: Vec<Cell>,
    changed: Vec<u64>,
}

impl Emit {
    /// The emit of a branch of a recording in `format`, its types resolved by `content`.
    pub fn new(format: FormatVersion, content: Content) -> Self {
        Self {
            content,
            periods: [
                period(TableId::Stockpiles, format),
                period(TableId::LogisticsEquipment, format),
            ],
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            family: None,
            holders: Vec::new(),
            slots: Vec::new(),
            countries: false,
            usable: [false; 3],
            waiting: [0; 3],
            epoch: 0,
            last: None,
            walking: false,
            next: [None; 2],
            names: Names::default(),
            spelled: FastMap::default(),
            cells: Vec::new(),
            changed: Vec::new(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors.key(writer, place, (RunMode::Exact, 0), None)
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 4]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 4];
        for (key, measure) in keys
            .iter_mut()
            .zip([DAILY_READS, LOGISTICS_READS, TOKENS, FORGETS])
        {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: columnar::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("stockpile.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(invalid("A stockpile hour does not follow the hour before"));
            }
            // An hour not walked ends every coverage, schedule and name.
            Some(last) => self.close(writer, last)?,
            None => {}
        }
        self.last = Some(at);
        let keys = self.family(writer)?;
        if !self.walking {
            for key in &keys[..2] {
                self.carry.evidence(writer, *key, at, Some(WALKED))?;
            }
            self.walking = true;
        }
        let Some(facts) = hour.facts else {
            return self.schedule(writer, at, [false; 2]);
        };
        let Facts {
            reads,
            countries,
            usable,
            holders,
            evidence,
            variants,
            rows,
            ..
        } = *facts;
        self.names(writer, at, evidence, variants)?;
        let whole = holders.is_some();
        if let Some(holders) = holders {
            self.identify(writer, at, &holders)?;
        }
        if whole || countries != self.countries || usable != self.usable {
            self.countries = countries;
            self.usable = usable;
            for index in 0..self.holders.len() {
                self.cover(writer, index, at)?;
            }
        }
        for read in &rows {
            self.read(writer, at, reads, read)?;
        }
        // Every holder waiting for a read was read.
        for (stream, due) in [(0, reads[0]), (1, reads[0]), (2, reads[1])] {
            if due && self.waiting[stream] != 0 {
                return Err(invalid(
                    "A stockpile read lacks a holder whose rows it needs",
                ));
            }
        }
        self.schedule(writer, at, reads)
    }

    /// Ends what is open after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        self.walking = false;
        self.next = [None; 2];
        for holder in &mut self.holders {
            for (stream, coverage) in holder.coverage.iter_mut().enumerate() {
                coverage.cut(writer, holder.keys[STOCK_HOURS as usize + stream], last)?;
                coverage.open = None;
            }
            holder.daily = Stream::default();
            holder.logistics = Stream::default();
        }
        self.waiting = [0; 3];
        self.forget(writer, last + 1)
    }

    /// The tables' read schedules at `at`: a read off schedule starts one, a read due and
    /// missed ends it.
    fn schedule(&mut self, writer: &mut Writer<'_>, at: i32, reads: [bool; 2]) -> io::Result<()> {
        let keys = self.family(writer)?;
        for stream in 0..2 {
            let period = self.periods[stream];
            if reads[stream] {
                if self.next[stream] != Some(at) {
                    self.carry
                        .value(writer, keys[stream], at, Some(i128::from(period)), true)?;
                }
                self.next[stream] = (period > 0).then(|| at + period);
            } else if self.next[stream] == Some(at) {
                self.carry.value(writer, keys[stream], at, None, false)?;
                self.next[stream] = None;
            }
        }
        Ok(())
    }

    /// The hour's naming: a complete read's variants named where their operands changed or
    /// were forgotten, or every name forgotten.
    fn names(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        evidence: VariantHour,
        variants: Vec<Variant>,
    ) -> io::Result<()> {
        match evidence {
            VariantHour::Carried { complete: true } => Ok(()),
            VariantHour::Carried { complete: false } | VariantHour::Incomplete => {
                self.forget(writer, at)
            }
            VariantHour::Read => {
                for variant in variants {
                    self.name(writer, at, variant)?;
                }
                Ok(())
            }
        }
    }

    fn name(&mut self, writer: &mut Writer<'_>, at: i32, variant: Variant) -> io::Result<()> {
        let generation = self.names.generation;
        let named = self.names.variants.get(&variant.id).copied();
        if named
            .is_some_and(|named| named.generation == generation && named.digest == variant.digest)
        {
            return Ok(());
        }
        let place = |measure| Place {
            actor: FAMILY,
            measure,
            dimension: Some(variant.id),
        };
        let key = self.key(writer, place(NAMES))?;
        let json = serde_json::to_vec(&variant.raw)?;
        let text = writer.blob(&mut json.as_slice(), json.len() as u64)?;
        let fields = writer.cells([Cell::Text(text)])?;
        event(writer, key, at, Some(at), kind::NAME, fields)?;
        self.names.named = true;
        let mut token = named.map_or(NO_TOKEN, |named| named.token);
        let spelling = variant.raw.token.as_str();
        let held = self.names.tokens.get(token as usize).map(|held| &**held);
        if !spelling.is_empty() && held != Some(spelling) {
            token = match self.names.numbered.get(spelling) {
                Some(&number) => number,
                None => {
                    let number = u32::try_from(self.names.tokens.len())
                        .ok()
                        .filter(|&number| number != NO_TOKEN)
                        .ok_or_else(|| invalid("Stockpile variant tokens exhausted"))?;
                    engine_allocation::reserve(&mut self.names.tokens, 1)?;
                    engine_allocation::reserve_map(&mut self.names.numbered, 1)?;
                    self.names.tokens.push(spelling.into());
                    self.names.numbered.insert(spelling.into(), number);
                    number
                }
            };
            let key = self.key(writer, place(CLASSES))?;
            let class = self.content.kind(spelling);
            let spelled = writer.text(spelling)?;
            let cells = self.classified(writer, class)?;
            let fields = writer.cells([Cell::Text(spelled), cells[0], cells[1]])?;
            event(writer, key, at, Some(at), kind::CLASS, fields)?;
        }
        engine_allocation::reserve_map(&mut self.names.variants, 1)?;
        self.names.variants.insert(
            variant.id,
            Named {
                generation,
                digest: variant.digest,
                token,
            },
        );
        Ok(())
    }

    /// A type's archetype and domain cells, null where content does not resolve it.
    fn classified(&self, writer: &mut Writer<'_>, kind: Option<Kind>) -> io::Result<[Cell; 2]> {
        Ok(match kind {
            Some(kind) => [
                Cell::Text(writer.text(&kind.key)?),
                Cell::Unsigned(domain(kind.domain)),
            ],
            None => [Cell::Null, Cell::Null],
        })
    }

    /// Every name is unknown from `at`, where any is known.
    fn forget(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        if !self.names.named {
            return Ok(());
        }
        self.names.generation += 1;
        self.names.named = false;
        let keys = self.family(writer)?;
        event(writer, keys[3], at, None, kind::FORGET, Blob::EMPTY)
    }

    /// Each holder's identity from a whole hour's holders: every tag they resolve, and none
    /// for every other.
    fn identify(&mut self, writer: &mut Writer<'_>, at: i32, holders: &Holders) -> io::Result<()> {
        self.epoch += 1;
        self.slots.clear();
        engine_allocation::reserve(&mut self.slots, holders.identities.len())?;
        for (tag, &identity) in &holders.identities {
            let actor = self.actors.actor(writer, tag.as_bytes(), at)?;
            let index =
                usize::try_from(actor).map_err(|_| invalid("Stockpile holders exhausted"))?;
            if index == self.holders.len() {
                let mut keys = [0; 6];
                for (measure, key) in keys.iter_mut().enumerate() {
                    *key = self.key(
                        writer,
                        Place {
                            actor,
                            measure: measure as u64,
                            dimension: None,
                        },
                    )?;
                }
                engine_allocation::reserve(&mut self.holders, 1)?;
                self.holders.push(Holder::new(keys));
            }
            let holder = self
                .holders
                .get_mut(index)
                .ok_or_else(|| invalid("A stockpile holder's actor is out of order"))?;
            holder.seen = self.epoch;
            holder.identity = Some(identity);
            self.slots.push((identity.0, actor as u32));
        }
        for holder in &mut self.holders {
            if holder.seen != self.epoch {
                holder.identity = None;
            }
        }
        self.slots.sort_unstable();
        Ok(())
    }

    /// Each of the holder's coverages from `at`, as the hour's countries, tables and its
    /// identity leave it: a known one goes on while all three hold.
    fn cover(&mut self, writer: &mut Writer<'_>, index: usize, at: i32) -> io::Result<()> {
        let (countries, usable) = (self.countries, self.usable);
        let holder = &mut self.holders[index];
        for stream in 0..3 {
            let coverage = holder.coverage[stream];
            let status = match holder.identity {
                _ if !countries => Status::Unknown(reason::COUNTRIES),
                None => Status::Unknown(reason::UNRESOLVED),
                Some(_) if !usable[stream] => Status::Unknown(reason::TABLE),
                Some(identity) => match coverage.open {
                    Some((Status::Known(known), _)) if known == packed(identity) => continue,
                    _ => Status::Unknown(reason::UNREAD),
                },
            };
            if coverage.known() {
                holder.forget(stream);
            }
            let unread = Status::Unknown(reason::UNREAD);
            match (coverage.unread(), status == unread) {
                (false, true) => self.waiting[stream] += 1,
                (true, false) => self.waiting[stream] -= 1,
                _ => {}
            }
            let key = holder.keys[STOCK_HOURS as usize + stream];
            holder.coverage[stream].set(writer, key, at, status)?;
        }
        Ok(())
    }

    /// A holder slot's read, for every tag it holds.
    fn read(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        reads: [bool; 2],
        read: &Read,
    ) -> io::Result<()> {
        let mut index = self.slots.partition_point(|&(slot, _)| slot < read.slot);
        if self
            .slots
            .get(index)
            .is_none_or(|&(slot, _)| slot != read.slot)
        {
            return Err(invalid("A stockpile read names a slot no holder holds"));
        }
        while let Some(&(slot, actor)) = self.slots.get(index)
            && slot == read.slot
        {
            let holder = actor as usize;
            if reads[0] && (read.stock.is_some() || read.lines.is_some()) {
                self.daily(writer, at, holder, read)?;
            }
            if reads[1]
                && let Some(totals) = &read.logistics
            {
                self.logistics(writer, at, holder, totals)?;
            }
            self.count(writer, at, holder, read)?;
            index += 1;
        }
        if reads[1] && read.logistics.is_some() {
            self.spell(writer, at, &read.spellings)?;
        }
        Ok(())
    }

    /// Starts the holder's coverage of `stream` at its read, where it waits for one: whether it
    /// did. A read of a stream that is neither known nor waiting is not the projector's.
    fn start(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        index: usize,
        stream: usize,
        whole: bool,
    ) -> io::Result<bool> {
        let holder = &mut self.holders[index];
        let identity = holder
            .identity
            .ok_or_else(|| invalid("A stockpile read names an unresolved holder"))?;
        let coverage = &mut holder.coverage[stream];
        match coverage.open {
            Some((Status::Known(known), _)) if known == packed(identity) => Ok(false),
            Some((Status::Unknown(reason::UNREAD), _)) if whole => {
                let key = holder.keys[STOCK_HOURS as usize + stream];
                coverage.set(writer, key, at, Status::Known(packed(identity)))?;
                self.waiting[stream] -= 1;
                Ok(true)
            }
            _ => Err(invalid(
                "A stockpile read is of a holder whose table it cannot read",
            )),
        }
    }

    fn daily(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        index: usize,
        read: &Read,
    ) -> io::Result<()> {
        let mut starts = false;
        if read.stock.is_some() {
            let whole = matches!(read.stock, Some(Totals::Whole(_)));
            starts |= self.start(writer, at, index, 0, whole)?;
        }
        if read.lines.is_some() {
            let whole = matches!(read.lines, Some(Totals::Whole(_)));
            starts |= self.start(writer, at, index, 1, whole)?;
        }
        let mut changed = std::mem::take(&mut self.changed);
        changed.clear();
        let holder = &mut self.holders[index];
        if let Some(totals) = &read.stock {
            apply_part(
                &mut holder.daily.rows,
                totals,
                |[amount]| amount,
                stock_part,
                &mut changed,
            )?;
        }
        if let Some(totals) = &read.lines {
            apply_part(
                &mut holder.daily.rows,
                totals,
                |values| values,
                lines_part,
                &mut changed,
            )?;
        }
        changed.sort_unstable();
        changed.dedup();
        let result = if starts || !changed.is_empty() {
            let identity = holder
                .identity
                .ok_or_else(|| invalid("Unresolved holder"))?;
            holder.daily.write(
                writer,
                holder.keys[DAILY as usize],
                at,
                identity,
                &changed,
                starts,
                &mut self.cells,
            )
        } else {
            Ok(())
        };
        self.changed = changed;
        result
    }

    fn logistics(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        index: usize,
        totals: &Totals<u32, 3>,
    ) -> io::Result<()> {
        let starts = self.start(writer, at, index, 2, matches!(totals, Totals::Whole(_)))?;
        let mut changed = std::mem::take(&mut self.changed);
        changed.clear();
        let holder = &mut self.holders[index];
        apply_logistics(&mut holder.logistics.rows, totals, &mut changed)?;
        let result = if starts || !changed.is_empty() {
            let identity = holder
                .identity
                .ok_or_else(|| invalid("Unresolved holder"))?;
            holder.logistics.write(
                writer,
                holder.keys[LOGISTICS as usize],
                at,
                identity,
                &changed,
                starts,
                &mut self.cells,
            )
        } else {
            Ok(())
        };
        self.changed = changed;
        result
    }

    /// The tokens a logistics read spelled, each written once.
    fn spell(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        spellings: &[(u32, String)],
    ) -> io::Result<()> {
        let key = self.family(writer)?[2];
        for (token, spelling) in spellings {
            if self.spelled.contains_key(token) {
                continue;
            }
            let class = self.content.kind(spelling);
            let text = writer.text(spelling)?;
            let cells = self.classified(writer, class)?;
            let fields = writer.cells([
                Cell::Unsigned(u64::from(*token)),
                Cell::Text(text),
                cells[0],
                cells[1],
            ])?;
            event(writer, key, at, Some(at), kind::TOKEN, fields)?;
            engine_allocation::reserve_map(&mut self.spelled, 1)?;
            self.spelled.insert(*token, ());
        }
        Ok(())
    }

    /// The read's counts of each table it read, written where the holder's change.
    fn count(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        index: usize,
        read: &Read,
    ) -> io::Result<()> {
        let holder = &mut self.holders[index];
        let mut counts = holder.counts;
        let parts = [
            (read.stock.is_some(), 0..1),
            (read.lines.is_some(), 1..7),
            (read.logistics.is_some(), 7..11),
        ];
        for (read_now, part) in parts {
            if read_now {
                for field in part {
                    counts[field] = read.counts[field] as u64;
                }
            }
        }
        if counts == holder.counts {
            return Ok(());
        }
        holder.counts = counts;
        let key = holder.keys[COUNTS as usize];
        let fields = writer.cells(counts.map(Cell::Unsigned))?;
        event(writer, key, at, Some(at), kind::COUNTS, fields)
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        for holder in &mut self.holders {
            for (stream, coverage) in holder.coverage.iter_mut().enumerate() {
                coverage.cut(writer, holder.keys[STOCK_HOURS as usize + stream], last)?;
            }
        }
        Ok(())
    }

    /// What the emit goes on with after a cut: each holder's identity, coverage, streams and
    /// counts, the schedules, and each variant's and token's naming; never what it published.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut o = Out(out);
        o.hour(self.last);
        o.u8(u8::from(self.walking));
        o.u8(u8::from(self.countries));
        for usable in self.usable {
            o.u8(u8::from(usable));
        }
        for next in self.next {
            o.hour(next);
        }
        o.var(self.names.generation);
        o.u8(u8::from(self.names.named));
        o.var(self.names.tokens.len() as u64);
        for token in &self.names.tokens {
            o.bytes(token.as_bytes());
        }
        let mut variants: Vec<_> = self.names.variants.iter().collect();
        variants.sort_unstable_by_key(|(id, _)| **id);
        o.var(variants.len() as u64);
        for (&id, named) in variants {
            o.var(id);
            o.var(named.generation);
            o.0.extend_from_slice(&named.digest.to_le_bytes());
            o.var(u64::from(named.token));
        }
        let mut spelled: Vec<u32> = self.spelled.keys().copied().collect();
        spelled.sort_unstable();
        o.var(spelled.len() as u64);
        for token in spelled {
            o.var(u64::from(token));
        }
        o.var(self.holders.len() as u64);
        for holder in &self.holders {
            match holder.identity {
                Some((slot, id)) => {
                    o.u8(1);
                    o.var(u64::from(slot));
                    o.var(u64::from(id));
                }
                None => o.u8(0),
            }
            for coverage in holder.coverage {
                match coverage.open {
                    None => o.u8(0),
                    Some((status, from)) => {
                        let (flag, value) = match status {
                            Status::Known(holder) => (1, holder),
                            Status::Unknown(reason) => (2, reason),
                        };
                        o.u8(flag);
                        o.var(value);
                        o.zig(i64::from(from));
                    }
                }
            }
            o.chain(holder.daily.chain);
            o.var(holder.daily.rows.len() as u64);
            for (&variant, row) in &holder.daily.rows {
                o.var(variant);
                match row.stock {
                    Some(stock) => {
                        o.u8(1);
                        o.amount(stock);
                    }
                    None => o.u8(0),
                }
                match row.lines {
                    Some(lines) => {
                        o.u8(1);
                        for value in lines {
                            o.amount(value);
                        }
                    }
                    None => o.u8(0),
                }
            }
            o.chain(holder.logistics.chain);
            o.var(holder.logistics.rows.len() as u64);
            for (&token, row) in &holder.logistics.rows {
                o.var(token);
                for &value in row {
                    o.amount(value);
                }
            }
            for count in holder.counts {
                o.var(count);
            }
        }
        self.carry.save(o.0)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        format: FormatVersion,
        content: Content,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut emit = Self::new(format, content);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut family = [0; 4];
        for (key, measure) in family
            .iter_mut()
            .zip([DAILY_READS, LOGISTICS_READS, TOKENS, FORGETS])
        {
            *key = directory.key(FAMILY, measure, None).ok_or_else(damaged)?;
        }
        emit.family = Some(family);
        let mut d = Decode::new(bytes);
        emit.last = hour(&mut d)?;
        emit.walking = d.flag()?;
        emit.countries = d.flag()?;
        for usable in &mut emit.usable {
            *usable = d.flag()?;
        }
        for next in &mut emit.next {
            *next = hour(&mut d)?;
        }
        emit.names.generation = d.var()?;
        emit.names.named = d.flag()?;
        for number in 0..count(&mut d)? {
            let length = usize::try_from(d.var()?).map_err(|_| damaged())?;
            let token: Box<str> = std::str::from_utf8(d.bytes(length)?)
                .map_err(|_| damaged())?
                .into();
            emit.names.numbered.insert(token.clone(), number as u32);
            emit.names.tokens.push(token);
        }
        for _ in 0..count(&mut d)? {
            let id = d.var()?;
            let generation = d.var()?;
            let digest = u128::from_le_bytes(d.array()?);
            let token = u32::try_from(d.var()?).map_err(|_| damaged())?;
            emit.names.variants.insert(
                id,
                Named {
                    generation,
                    digest,
                    token,
                },
            );
        }
        for _ in 0..count(&mut d)? {
            let token = u32::try_from(d.var()?).map_err(|_| damaged())?;
            emit.spelled.insert(token, ());
        }
        let holders = count(&mut d)?;
        if holders as u64 != emit.actors.len() {
            return Err(damaged());
        }
        for actor in 0..holders as u64 {
            let mut keys = [0; 6];
            for (measure, key) in keys.iter_mut().enumerate() {
                *key = directory
                    .key(actor, measure as u64, None)
                    .ok_or_else(damaged)?;
            }
            let mut holder = Holder::new(keys);
            holder.identity = match d.u8()? {
                0 => None,
                1 => Some((u32_of(&mut d)?, u32_of(&mut d)?)),
                _ => return Err(damaged()),
            };
            for coverage in &mut holder.coverage {
                coverage.open = match d.u8()? {
                    0 => None,
                    flag @ (1 | 2) => {
                        let value = d.var()?;
                        let status = if flag == 1 {
                            Status::Known(value)
                        } else {
                            Status::Unknown(value)
                        };
                        Some((status, hour_of(&mut d)?))
                    }
                    _ => return Err(damaged()),
                };
            }
            holder.daily.chain = chain(&mut d)?;
            for _ in 0..count(&mut d)? {
                let variant = d.var()?;
                let stock = match d.u8()? {
                    0 => None,
                    1 => Some(amount_of(&mut d)?),
                    _ => return Err(damaged()),
                };
                let lines = match d.u8()? {
                    0 => None,
                    1 => Some([
                        amount_of(&mut d)?,
                        amount_of(&mut d)?,
                        amount_of(&mut d)?,
                        amount_of(&mut d)?,
                    ]),
                    _ => return Err(damaged()),
                };
                holder.daily.rows.insert(variant, Daily { stock, lines });
            }
            holder.logistics.chain = chain(&mut d)?;
            for _ in 0..count(&mut d)? {
                let token = d.var()?;
                let values = [amount_of(&mut d)?, amount_of(&mut d)?, amount_of(&mut d)?];
                holder.logistics.rows.insert(token, values);
            }
            for count in &mut holder.counts {
                *count = d.var()?;
            }
            if let Some((slot, _)) = holder.identity {
                emit.slots.push((slot, actor as u32));
            }
            for (stream, coverage) in holder.coverage.iter().enumerate() {
                emit.waiting[stream] += usize::from(coverage.unread());
            }
            emit.holders.push(holder);
        }
        emit.slots.sort_unstable();
        emit.carry = Carry::load(&bytes[d.position()..])?;
        Ok(emit)
    }
}

/// The saved emit's encoding: varints, zigzag hours and amounts, flags.
struct Out<'a>(&'a mut Vec<u8>);

impl Out<'_> {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn var(&mut self, mut value: u64) {
        while value >= 0x80 {
            self.0.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    fn zig(&mut self, value: i64) {
        self.var(((value << 1) ^ (value >> 63)) as u64);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.var(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
    }

    fn hour(&mut self, hour: Option<i32>) {
        match hour {
            Some(hour) => {
                self.u8(1);
                self.zig(i64::from(hour));
            }
            None => self.u8(0),
        }
    }

    fn amount(&mut self, amount: Option<i64>) {
        match amount {
            Some(amount) => {
                self.u8(1);
                self.zig(amount);
            }
            None => self.u8(0),
        }
    }

    fn chain(&mut self, chain: Option<(i32, u8)>) {
        self.hour(chain.map(|(first, _)| first));
        if let Some((_, events)) = chain {
            self.u8(events);
        }
    }
}

fn damaged() -> io::Error {
    invalid("A saved stockpile emit is damaged")
}

fn count(d: &mut Decode<'_>) -> io::Result<usize> {
    usize::try_from(d.var()?).map_err(|_| damaged())
}

fn u32_of(d: &mut Decode<'_>) -> io::Result<u32> {
    u32::try_from(d.var()?).map_err(|_| damaged())
}

fn hour_of(d: &mut Decode<'_>) -> io::Result<i32> {
    i32::try_from(d.zig()?).map_err(|_| damaged())
}

fn hour(d: &mut Decode<'_>) -> io::Result<Option<i32>> {
    match d.u8()? {
        0 => Ok(None),
        1 => hour_of(d).map(Some),
        _ => Err(damaged()),
    }
}

fn amount_of(d: &mut Decode<'_>) -> io::Result<Option<i64>> {
    match d.u8()? {
        0 => Ok(None),
        1 => d.zig().map(Some),
        _ => Err(damaged()),
    }
}

fn chain(d: &mut Decode<'_>) -> io::Result<Option<(i32, u8)>> {
    let Some(first) = hour(d)? else {
        return Ok(None);
    };
    let events = d.u8()?;
    if events == 0 || events > CHAIN {
        return Err(damaged());
    }
    Ok(Some((first, events)))
}

#[cfg(test)]
pub(crate) mod tests;
