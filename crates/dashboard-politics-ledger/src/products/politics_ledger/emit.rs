//! The ledger's emit: the accounting of each projected hour ([`facts::Hour`], [`super::ledger`]),
//! written as the family's series publication. A country's native rows are recorded once, as the
//! read that first named their identity's tag saw them, each as the reply shows it
//! ([`super::records`]), and each item once, named at its first payment; its spending coverage,
//! holds and spending parts as runs and events that change only when they do; the census as the
//! branch's, and each tag's presence in it where that changes. An hour that changes nothing of a
//! country appends nothing.
//!
//! The walk's lens folds most countries of a segment itself ([`super::lens`]): the emit folds
//! each until the lens binds it, two days into the segment, where the two hold it alike; from
//! there it only replays what the lens hands it, writing the country's rows, items and parts,
//! and adds the rollups the lens settles to those it held before. It folds the country itself
//! again from where the lens hands it back. A reply reads everything back from the publication
//! ([`super::query`]).

use super::facts::{self, Hour};
use super::ledger::{self, Country, Field, Ledger, Out, Raw};
use super::lens::{self, Bind, Op, Piece, Taken};
use super::packed;
use super::tally;
use super::*;
use dashboard_base::prepared::codec;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_series::hash::FastMap;
use dashboard_series::{
    Blob, Cell, Event, Evidence, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{Actors, Directory, Place},
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 10;

/// Each country's measures: its power rows by currency, at their read; its governments, daily
/// rows and costs, at their native hour; whether its spending in each currency is recorded, with
/// the reasons; its hold snapshots; its spending parts, by dimension.
pub const POWER: [u64; 2] = [0, 1];
pub const GOVERNMENTS: u64 = 2;
pub const DAILY: u64 = 3;
pub const COSTS: u64 = 4;
pub const COVERED: [u64; 2] = [7, 8];
pub const COVER_REASONS: u64 = 9;
pub const HOLDS: u64 = 10;
pub const PARTS: u64 = 13;
/// Each country's settled totals by item and period ([`super::tally`]), at the period's last
/// hour; its payments read after their day settled, at their read; each item as replies show
/// it ([`records::ItemRecord`]), at its first payment; whether the census lists it present and
/// refused, where that changes.
pub const ROLLUP: u64 = 15;
pub const LATE: u64 = 16;
pub const ITEMS: u64 = 17;
pub const PRESENT: u64 = 18;
/// The branch's measures, under the actor with the empty name: whether the census is complete,
/// and every prepared hour.
pub const CENSUS: u64 = 20;
pub const ACCEPTED: u64 = 21;

/// Event kinds.
pub const POWER_KIND: u64 = 1;
pub const GOVERNMENT_KIND: u64 = 2;
pub const DAILY_KIND: u64 = 3;
pub const COST_KIND: u64 = 4;
pub const COVER_KIND: u64 = 6;
pub const HOLD_KIND: u64 = 7;
pub const PRESENCE_KIND: u64 = 9;
/// A rollup's kind is this plus its level.
pub const ROLLUP_KIND: u64 = 10;

/// What a power row or cost is to a reply's payments ([`records`]).
pub const PAYMENT: u64 = 0;
pub const BALANCE: u64 = 1;
pub const SKIPPED: u64 = 2;
pub const UNMATCHED: u64 = 3;
pub const CREDIT: u64 = 4;

/// Family scalars: the first prepared hour; the native day from which days are still open; the
/// branch's warnings; the recorded content's power warnings, once an item reached them; the
/// first hour the census listed its tags; and by actor, each country's warnings and its
/// categories' first hours.
pub const FIRST: u64 = 0;
pub const SETTLED: u64 = 1;
pub const WARNINGS: u64 = 2;
pub const POWER_WARNINGS: u64 = 3;
pub const PRESENCE_FIRST: u64 = 4;
pub const TAG_WARNINGS: u64 = 1 << 24;
pub const TAG_CATEGORIES: u64 = 2 << 24;
/// Each category's spelling (`Currency:category`), by its part dimension.
pub const CATEGORY_NAMES: u64 = 3 << 24;

/// The parts every country has, by dimension.
pub const PART_FIELDS: [&str; 10] = [
    "Political:income",
    "Political:costs",
    "Political:wrapping",
    "Political:income-total",
    "Command:income",
    "Command:income-total",
    "Political:zero",
    "Political:spending-total",
    "Command:zero",
    "Command:spending-total",
];
/// A category's part dimension: this plus its number.
pub const CATEGORY_PARTS: u64 = 64;
pub const ZERO: [u64; 2] = [6, 8];
pub const TOTAL: [u64; 2] = [7, 9];
/// Fixed5 values: runs of these measures carry five decimals.
pub const DECIMALS: u8 = 5;

/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// The branch's actor.
pub const BRANCH: &[u8] = b"";
pub(super) const CURRENCIES: [&str; 2] = ["Political", "Command"];

/// The measures a tag's keys are cached for: every one below [`CENSUS`].
const MEASURES: usize = CENSUS as usize;

/// An item the lens names that the emit has not.
const UNNAMED: u32 = u32::MAX;

fn invalid(what: &str) -> io::Error {
    dashboard_core::invalid(format!("Politics ledger emit: {what}"))
}

/// `what`, met while a lens folds countries the emit cannot publish without: the fold is wrong,
/// never the recorded data, so it ends the family rather than refuse its hour.
fn unrefused(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::InvalidData {
        io::Error::other(format!("Politics ledger lens: {error}"))
    } else {
        error
    }
}

fn lens_failure(what: &str) -> io::Error {
    io::Error::other(format!("Politics ledger lens: {what}"))
}

fn text(writer: &mut Writer, bytes: &[u8]) -> io::Result<Cell> {
    if bytes.is_empty() {
        return Ok(Cell::Text(dashboard_series::Blob::EMPTY));
    }
    Ok(Cell::Text(writer.bytes(bytes)?))
}

/// A tag's actor, and its series keys as interned by measure and by part dimension: an
/// account finds each without hashing.
struct Keys {
    actor: u64,
    keys: [Option<u64>; MEASURES],
    parts: Vec<Option<u64>>,
}

impl Keys {
    fn of(actor: u64) -> Self {
        Self {
            actor,
            keys: [None; MEASURES],
            parts: Vec::new(),
        }
    }
}

/// The accounting written into the series.
struct Direct<'a> {
    writer: &'a mut Writer,
    actors: &'a mut Actors,
    carry: &'a mut Carry,
    keys: &'a mut Vec<Keys>,
    /// The last record written, and its blob: a row recorded twice is written once.
    record: &'a mut (Vec<u8>, Blob),
}

macro_rules! direct {
    ($emit:ident, $writer:expr) => {
        Direct {
            writer: $writer,
            actors: &mut $emit.actors,
            carry: &mut $emit.carry,
            keys: &mut $emit.keys,
            record: &mut $emit.record,
        }
    };
}

impl Direct<'_> {
    fn key(&mut self, tag: u32, measure: u64, dimension: Option<u64>) -> io::Result<u64> {
        let keys = self
            .keys
            .get_mut(tag as usize)
            .ok_or_else(|| invalid("a tag without keys"))?;
        let held = match dimension {
            None => keys.keys.get(measure as usize).copied().flatten(),
            Some(dimension) if measure == PARTS => keys.parts.get(dimension as usize).copied().flatten(),
            Some(_) => None,
        };
        if let Some(key) = held {
            return Ok(key);
        }
        let decimals = if measure == PARTS { DECIMALS } else { 0 };
        let key = self.actors.key(
            self.writer,
            Place {
                actor: keys.actor,
                measure,
                dimension,
            },
            (RunMode::Exact, decimals),
            Some(DOMAIN),
        )?;
        match dimension {
            None => {
                if let Some(slot) = keys.keys.get_mut(measure as usize) {
                    *slot = Some(key);
                }
            }
            Some(dimension) if measure == PARTS => {
                let at = usize::try_from(dimension).map_err(|_| invalid("part dimension"))?;
                if at >= keys.parts.len() {
                    keys.parts.resize(at + 1, None);
                }
                keys.parts[at] = Some(key);
            }
            Some(_) => {}
        }
        Ok(key)
    }

    fn record(&mut self, bytes: &[u8]) -> io::Result<Blob> {
        if !bytes.is_empty() && self.record.0 == bytes {
            return Ok(self.record.1);
        }
        let blob = self.writer.bytes(bytes)?;
        self.record.0.clear();
        self.record.0.extend_from_slice(bytes);
        self.record.1 = blob;
        Ok(blob)
    }

    fn put(
        &mut self,
        key: u64,
        (native, read): (i32, Option<i32>),
        kind: u64,
        fields: Blob,
    ) -> io::Result<u64> {
        let sequence = self.writer.sequence()?;
        self.writer.event(Event {
            key,
            native_hour: native,
            read_hour: read,
            sequence,
            kind,
            fields,
        })?;
        Ok(sequence)
    }

    /// `tag`'s periods `closed`, settled at `at`.
    fn closed(&mut self, tag: u32, at: i32, closed: Vec<(usize, i32, tally::Totals)>) -> io::Result<()> {
        for (level, index, totals) in closed {
            let key = self.key(tag, ROLLUP, None)?;
            let size = tally::PERIOD_DAYS[level];
            let last = (index * size + size) * 24 - 1;
            let kind = ROLLUP_KIND + level as u64;
            let fields = self.writer.cells(tally::cells(&totals))?;
            self.put(key, (last, Some(at)), kind, fields)?;
        }
        Ok(())
    }
}

impl Out for Direct<'_> {
    fn keeps(&self, _: u32) -> bool {
        true
    }

    fn tagged(&mut self, tag: u32, spelling: &str, hour: i32) -> io::Result<()> {
        let actor = self.actors.actor(self.writer, spelling.as_bytes(), hour)?;
        if self.keys.len() != tag as usize {
            return Err(invalid("tags numbered out of order"));
        }
        self.keys.push(Keys::of(actor));
        Ok(())
    }

    fn event(
        &mut self,
        tag: u32,
        measure: u64,
        hours: (i32, Option<i32>),
        kind: u64,
        fields: &[Field<'_>],
    ) -> io::Result<u64> {
        let key = self.key(tag, measure, None)?;
        let mut cells = [Cell::Null; 2];
        if fields.len() > cells.len() {
            return Err(invalid("an event of too many fields"));
        }
        for (cell, field) in cells.iter_mut().zip(fields) {
            *cell = match *field {
                Field::Plain(cell) => cell,
                Field::Record(bytes) => Cell::Text(self.record(bytes)?),
                Field::Item(ordinal) => Cell::Unsigned(u64::from(ordinal)),
            };
        }
        let blob = self.writer.cells(cells[..fields.len()].iter().copied())?;
        self.put(key, hours, kind, blob)
    }

    fn value(
        &mut self,
        tag: u32,
        measure: u64,
        dimension: Option<u64>,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let key = self.key(tag, measure, dimension)?;
        self.carry.value(self.writer, key, hour, value, starts)?;
        Ok(())
    }

    fn named(&mut self, dimension: u64, currency: usize, category: &str) -> io::Result<()> {
        let spelling = ledger::spelling(currency, category);
        let cell = text(self.writer, spelling.as_bytes())?;
        self.writer.catalogue(CATEGORY_NAMES + dimension, cell)
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        &mut self,
        tag: u32,
        ordinal: u32,
        _: &[u8; 32],
        new: bool,
        shown: Option<&[u8]>,
        hours: (i32, Option<i32>),
        kind: u64,
    ) -> io::Result<()> {
        if !new {
            return Ok(());
        }
        let shown = shown.ok_or_else(|| invalid("an item first paid without its record"))?;
        self.event(
            tag,
            ITEMS,
            hours,
            kind,
            &[Field::Item(ordinal), Field::Record(shown)],
        )?;
        Ok(())
    }

    fn category(&mut self, _: u32, _: u64) -> io::Result<()> {
        Ok(())
    }

    fn first(&mut self, _: u32, _: u64, _: i32) -> io::Result<()> {
        Ok(())
    }

    fn unpaid(&mut self, _: u32, _: usize, _: i32, _: Option<i128>, _: bool) -> io::Result<()> {
        Ok(())
    }

    fn settled(
        &mut self,
        tag: u32,
        at: i32,
        _: i32,
        closed: Vec<(usize, i32, tally::Totals)>,
        split: tally::Split,
    ) -> io::Result<()> {
        if !split.is_empty() {
            return Err(invalid("days split apart where no lens folds them"));
        }
        self.closed(tag, at, closed)
    }
}

/// A country the lens folds, as the emit bound it.
struct Bound {
    number: u32,
    slots: Vec<u32>,
    raws: Vec<u32>,
    /// The emit's ordinal of each of the lens's items; [`UNNAMED`] for one not yet named.
    items: Vec<u32>,
    /// Its categories the lens never paid in since its segment began, by currency.
    extra: [Vec<u64>; 2],
    /// Where its own rollups settle next, and what the lens settled of the hour.
    due: Option<i32>,
    settled: Vec<(Vec<(usize, i32, tally::Totals)>, tally::Split)>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    keys: Vec<Keys>,
    record: (Vec<u8>, Blob),
    ledger: Ledger,
    content_failure: Option<String>,
    /// Whether the content's power warnings were written since the emit began.
    power_warned: bool,
    /// The first hour of the accepted stretch not yet given its evidence.
    known_from: Option<i32>,
    begun: bool,
    presence: Option<(Vec<Box<str>>, Vec<Box<str>>)>,
    settled_written: Option<i32>,
    /// The branch's accepted and census keys as interned.
    branch_keys: [Option<u64>; 2],
    /// The countries the walk's lens bound, by the number it names them by: the emit's tag where
    /// the emit bound it too, None where the emit folds it itself.
    numbers: FastMap<u32, Option<u32>>,
    bound: FastMap<u32, Bound>,
    /// The lens's category dimensions: their currency and name, and the emit's dimension.
    lens_names: FastMap<u64, (usize, Box<str>)>,
    lens_dims: FastMap<u64, u64>,
    /// The record shapes the lens named.
    packed: packed::Named,
    /// The bound tags whose own rollups, those of the hours before the lens bound them, settle
    /// at each frontier.
    prior: BTreeSet<(i32, u32)>,
}

impl Emit {
    pub fn new(content: &Result<Option<Arc<PoliticsContent>>, String>) -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            keys: Vec::new(),
            record: (Vec::new(), Blob::EMPTY),
            ledger: Ledger::new(content.as_ref().ok().cloned().flatten()),
            content_failure: content.as_ref().err().cloned(),
            power_warned: false,
            known_from: None,
            begun: false,
            presence: None,
            settled_written: None,
            branch_keys: [None; 2],
            numbers: FastMap::default(),
            bound: FastMap::default(),
            lens_names: FastMap::default(),
            lens_dims: FastMap::default(),
            packed: packed::Named::default(),
            prior: BTreeSet::new(),
        }
    }

    fn branch_key(&mut self, writer: &mut Writer, measure: u64, hour: i32) -> io::Result<u64> {
        let at = match measure {
            ACCEPTED => Some(0),
            CENSUS => Some(1),
            _ => None,
        };
        if let Some(&Some(key)) = at.and_then(|at| self.branch_keys.get(at)) {
            return Ok(key);
        }
        let actor = self.actors.actor(writer, BRANCH, hour)?;
        let key = self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )?;
        if let Some(at) = at {
            self.branch_keys[at] = Some(key);
        }
        Ok(key)
    }

    /// Gives the accepted stretch since its first hour its evidence, through `last`.
    fn evidence(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        if let Some(first) = self.known_from.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// Keeps one projected hour, or what the lens hands back after the hour kept last.
    pub fn hour(&mut self, writer: &mut Writer, mut hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit", None);
        let part = hour.lens.take();
        if hour.handover {
            if self.ledger.last != Some(hour.hour) {
                return Err(lens_failure("it hands back an hour the emit did not keep"));
            }
            if let Some(part) = part {
                self.packed.extend(part.shapes).map_err(unrefused)?;
                self.returned(part.returned).map_err(unrefused)?;
            }
            return Ok(());
        }
        // Only the walk's read of an hour refuses it while the lens folds countries: the lens
        // hands them back through the hour before. An hour the emit itself would refuse then
        // could not publish them, so it ends the family instead.
        match self.keep(writer, hour, part) {
            Err(error) if !self.bound.is_empty() => Err(unrefused(error)),
            kept => kept,
        }
    }

    fn keep(
        &mut self,
        writer: &mut Writer,
        hour: Hour,
        part: Option<Box<lens::Part>>,
    ) -> io::Result<()> {
        let at = hour.hour;
        let mut part = part.map(|part| *part).unwrap_or_default();
        if part.begins {
            if !self.bound.is_empty() {
                return Err(lens_failure("it began again holding countries"));
            }
            self.numbers.clear();
            self.lens_names.clear();
            self.lens_dims.clear();
            self.packed.clear();
        }
        self.packed
            .extend(std::mem::take(&mut part.shapes))
            .map_err(unrefused)?;
        self.returned(std::mem::take(&mut part.returned))
            .map_err(unrefused)?;
        for (dimension, currency, name) in std::mem::take(&mut part.named) {
            self.lens_names
                .insert(dimension, (usize::from(currency), name));
        }
        let mut joined = self.ledger.open(&hour);
        let adjacent = joined.adjacent;
        if let Some(last) = joined.previous.filter(|_| !adjacent) {
            if !self.bound.is_empty() {
                return Err(lens_failure("it did not hand its countries back before a gap"));
            }
            self.ledger.gap(last);
            self.evidence(writer, last)?;
            self.carry.close(writer, last)?;
        }
        if !self.begun {
            writer.catalogue(FIRST, Cell::Integer(i128::from(at)))?;
            if let Some(reason) = self.content_failure.clone() {
                // The content's refusal is the branch's once, whatever its hours.
                self.ledger.warnings.entry(reason).or_insert(1);
                self.ledger.warnings_changed = true;
            }
            self.begun = true;
        }
        self.known_from.get_or_insert(at);
        let accepted = self.branch_key(writer, ACCEPTED, at)?;
        self.carry.value(writer, accepted, at, Some(1), !adjacent)?;
        let census_complete = hour.complete & facts::bit(TableId::Countries) != 0;
        let census_key = self.branch_key(writer, CENSUS, at)?;
        self.carry.value(
            writer,
            census_key,
            at,
            census_complete.then_some(1),
            !adjacent,
        )?;
        let identities = ::replay_frame::cpu::scope("politics_ledger.emit.identities", None);
        let census_changed = {
            let mut direct = direct!(self, &mut *writer);
            self.ledger.join(&mut direct, &hour, &mut joined)?
        };
        if census_changed || hour.links.is_some() {
            self.verify_bound().map_err(unrefused)?;
        }
        if census_changed {
            self.presence(writer, at)?;
        }
        drop(identities);
        let rows = ::replay_frame::cpu::scope("politics_ledger.emit.rows", None);
        {
            let mut direct = direct!(self, &mut *writer);
            self.ledger.rows(&mut direct, &hour, &mut joined)?;
        }
        drop(rows);
        let accounts = ::replay_frame::cpu::scope("politics_ledger.emit.accounts", None);
        let payload = std::mem::take(&mut part.payload);
        let mut replays: Vec<(u32, Vec<Op>)> = std::mem::take(&mut part.ops)
            .into_iter()
            .filter_map(|(number, ops)| match self.numbers.get(&number) {
                Some(&Some(tag)) => Some((tag, ops)),
                // A country the emit folds itself: its ops are of no further use.
                _ => None,
            })
            .collect();
        replays.sort_unstable_by_key(|(tag, _)| *tag);
        let mut replays = replays.into_iter().peekable();
        // Tags in ascending order, as the accounts write them: categories are numbered so.
        for tag in std::mem::take(&mut joined.dirty) {
            while let Some(&(next, _)) = replays.peek()
                && next < tag
            {
                if let Some((next, ops)) = replays.next() {
                    self.replay(writer, next, ops, &payload).map_err(unrefused)?;
                }
            }
            if self.bound.contains_key(&tag) {
                continue;
            }
            let mut direct = direct!(self, &mut *writer);
            self.ledger.account(&mut direct, tag, at, &joined)?;
        }
        for (tag, ops) in replays {
            self.replay(writer, tag, ops, &payload).map_err(unrefused)?;
        }
        drop(accounts);
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.emit.settle", None);
        let moved = self.ledger.settled.is_some() && self.ledger.settled != self.ledger.rolled;
        {
            let mut direct = direct!(self, &mut *writer);
            self.ledger.settle(&mut direct, at)?;
        }
        if moved {
            self.settle_bound(writer, at).map_err(unrefused)?;
        }
        let binds = std::mem::take(&mut part.bound);
        if !binds.is_empty() {
            self.bind(binds, at);
        }
        Ok(())
    }

    /// The bound countries hold the slots and raw countries they held when bound: the lens
    /// hands one back before an hour that changes them.
    fn verify_bound(&self) -> io::Result<()> {
        for (&tag, bound) in &self.bound {
            let country = &self.ledger.countries[tag as usize];
            if !country.active
                || country.slots != bound.slots
                || self.ledger.raws_of(tag) != bound.raws
            {
                return Err(lens_failure("it kept a country whose census or links changed"));
            }
        }
        Ok(())
    }

    /// The branch's present and refused tags, at each change: each tag's where it changed, every
    /// tag first absent from the first hour any is listed.
    fn presence(&mut self, writer: &mut Writer, hour: i32) -> io::Result<()> {
        let ledger = &self.ledger;
        if !ledger.census_complete {
            return Ok(());
        }
        let mut states = vec![(false, false); ledger.tags.len()];
        for &tag in ledger.slot_tags.values() {
            states[tag as usize].0 = true;
        }
        for &tag in ledger.refused.keys().filter_map(|slot| ledger.known.get(slot)) {
            states[tag as usize].1 = true;
        }
        let spellings = |listed: &dyn Fn(&(bool, bool)) -> bool| -> Vec<Box<str>> {
            let mut tags: Vec<Box<str>> = states
                .iter()
                .enumerate()
                .filter(|(_, state)| listed(state))
                .map(|(tag, _)| ledger.tags[tag].clone())
                .collect();
            tags.sort_unstable();
            tags
        };
        let value = (spellings(&|state| state.0), spellings(&|state| state.1));
        if self.presence.as_ref() == Some(&value) {
            return Ok(());
        }
        if self.presence.is_none() {
            writer.catalogue(PRESENCE_FIRST, Cell::Integer(i128::from(hour)))?;
        }
        self.presence = Some(value);
        for (tag, state) in states.into_iter().enumerate() {
            if self.ledger.countries[tag].presence == state {
                continue;
            }
            self.ledger.countries[tag].presence = state;
            let bytes = codec::encode(&state)?;
            let mut direct = direct!(self, &mut *writer);
            direct.event(
                tag as u32,
                PRESENT,
                (hour, None),
                PRESENCE_KIND,
                &[Field::Record(&bytes)],
            )?;
        }
        Ok(())
    }

    /// The emit's dimension of lens category dimension `dimension`, named when new, which bound
    /// country `tag` pays in.
    fn dimension(&mut self, writer: &mut Writer, tag: u32, dimension: u64) -> io::Result<u64> {
        if dimension < CATEGORY_PARTS {
            return Ok(dimension);
        }
        let held = match self.lens_dims.get(&dimension) {
            Some(&held) => held,
            None => {
                let (currency, name) = self
                    .lens_names
                    .get(&dimension)
                    .cloned()
                    .ok_or_else(|| lens_failure("it names a category it never named"))?;
                let mut direct = direct!(self, &mut *writer);
                let held = self.ledger.category(&mut direct, currency, &name)?;
                self.lens_dims.insert(dimension, held);
                held
            }
        };
        if let Some(bound) = self.bound.get_mut(&tag)
            && let Some(&currency) = self.ledger.category_currency.get(&held)
        {
            bound.extra[currency].retain(|&extra| extra != held);
        }
        Ok(held)
    }

    /// Writes what bound country `tag` wrote in the lens at the hour.
    fn replay(
        &mut self,
        writer: &mut Writer,
        tag: u32,
        ops: Vec<Op>,
        payload: &[u8],
    ) -> io::Result<()> {
        for op in ops {
            match op {
                Op::Event {
                    measure,
                    hours,
                    kind,
                    fields,
                } => {
                    let bound = self
                        .bound
                        .get(&tag)
                        .ok_or_else(|| lens_failure("ops of a country it did not bind"))?;
                    let mut records: [Vec<u8>; 2] = Default::default();
                    if fields.len() > records.len() {
                        return Err(lens_failure("an event of too many fields"));
                    }
                    for (record, piece) in records.iter_mut().zip(&fields) {
                        if let Piece::Record(at, len) = *piece {
                            self.packed.unpack(slice(payload, (at, len))?, record)?;
                        }
                    }
                    let mut held = [Field::Item(0); 2];
                    for ((field, piece), record) in held.iter_mut().zip(&fields).zip(&records) {
                        *field = match *piece {
                            Piece::Record(..) => Field::Record(record),
                            Piece::Item(ordinal) => Field::Item(
                                bound
                                    .items
                                    .get(ordinal as usize)
                                    .copied()
                                    .filter(|&held| held != UNNAMED)
                                    .ok_or_else(|| lens_failure("an item it never named"))?,
                            ),
                        };
                    }
                    let mut direct = direct!(self, &mut *writer);
                    direct.event(tag, measure, hours, kind, &held[..fields.len()])?;
                }
                Op::Value {
                    measure,
                    dimension,
                    hour,
                    value,
                    starts,
                } => {
                    let dimension = match dimension {
                        Some(dimension) if measure == PARTS => {
                            Some(self.dimension(writer, tag, dimension)?)
                        }
                        dimension => dimension,
                    };
                    let mut direct = direct!(self, &mut *writer);
                    direct.value(tag, measure, dimension, hour, value, starts)?;
                }
                Op::Item {
                    ordinal,
                    key,
                    shown,
                    hours,
                    kind,
                } => {
                    let country = &mut self.ledger.countries[tag as usize];
                    let (held, new) = match country.item_index.get(&key) {
                        Some(&held) => (held, false),
                        None => {
                            let held = country.items.len() as u32;
                            country.items.push(key);
                            country.item_index.insert(key, held);
                            (held, true)
                        }
                    };
                    let bound = self
                        .bound
                        .get_mut(&tag)
                        .ok_or_else(|| lens_failure("ops of a country it did not bind"))?;
                    let at = ordinal as usize;
                    if bound.items.len() <= at {
                        bound.items.resize(at + 1, UNNAMED);
                    }
                    bound.items[at] = held;
                    if new {
                        let shown = shown
                            .ok_or_else(|| lens_failure("an item the emit never read, unshown"))?;
                        let mut record = Vec::new();
                        self.packed.unpack(slice(payload, shown)?, &mut record)?;
                        let mut direct = direct!(self, &mut *writer);
                        direct.event(
                            tag,
                            ITEMS,
                            hours,
                            kind,
                            &[Field::Item(held), Field::Record(&record)],
                        )?;
                    }
                }
                Op::Category(dimension) => {
                    self.dimension(writer, tag, dimension)?;
                }
                Op::First { dimension, hour } => {
                    let dimension = self.dimension(writer, tag, dimension)?;
                    let country = &mut self.ledger.countries[tag as usize];
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        country.categories.entry(dimension)
                    {
                        entry.insert(hour);
                        country.categories_changed = true;
                    }
                }
                Op::Unpaid {
                    currency,
                    hour,
                    value,
                    starts,
                } => {
                    let extra = self
                        .bound
                        .get(&tag)
                        .and_then(|bound| bound.extra.get(usize::from(currency)))
                        .ok_or_else(|| lens_failure("ops of a country it did not bind"))?
                        .clone();
                    let mut direct = direct!(self, &mut *writer);
                    for dimension in extra {
                        direct.value(tag, PARTS, Some(dimension), hour, value, starts)?;
                    }
                }
                Op::Settled {
                    settled,
                    closed,
                    split,
                } => {
                    if Some(settled) != self.ledger.settled {
                        return Err(lens_failure("it settled at another frontier"));
                    }
                    self.bound
                        .get_mut(&tag)
                        .ok_or_else(|| lens_failure("ops of a country it did not bind"))?
                        .settled
                        .push((closed, split));
                }
            }
        }
        Ok(())
    }

    /// Settles, at `at`, the rollups of the bound countries the frontier reached: those each
    /// held before the lens bound it, with what the lens settled of the hours since.
    fn settle_bound(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        let Some(settled) = self.ledger.settled else {
            return Ok(());
        };
        let mut tags = BTreeSet::new();
        while let Some(&(due, tag)) = self.prior.first() {
            if due > settled {
                break;
            }
            self.prior.pop_first();
            if let Some(bound) = self.bound.get_mut(&tag) {
                bound.due = None;
            }
            tags.insert(tag);
        }
        tags.extend(
            self.bound
                .iter()
                .filter(|(_, bound)| !bound.settled.is_empty())
                .map(|(&tag, _)| tag),
        );
        for tag in tags {
            let bound = self
                .bound
                .get_mut(&tag)
                .ok_or_else(|| lens_failure("it settled a country it did not bind"))?;
            let lensed = std::mem::take(&mut bound.settled);
            let items = &bound.items;
            let item = |ordinal: u32| -> io::Result<u32> {
                items
                    .get(ordinal as usize)
                    .copied()
                    .filter(|&held| held != UNNAMED)
                    .ok_or_else(|| lens_failure("it settled an item it never named"))
            };
            let rollups = &mut self.ledger.countries[tag as usize].rollups;
            let mut closed: BTreeMap<(usize, i32), tally::Totals> = BTreeMap::new();
            let mut later = Vec::with_capacity(lensed.len());
            for (held, split) in lensed {
                rollups.join(remap_split(split, &item)?)?;
                later.push(held);
            }
            for (level, index, totals) in rollups.settle(settled)? {
                closed.insert((level, index), totals);
            }
            for (level, index, totals) in later.into_iter().flatten() {
                let totals = totals.remap(&item)?;
                match closed.entry((level, index)) {
                    std::collections::btree_map::Entry::Occupied(held) => {
                        held.into_mut().merge(&totals)?
                    }
                    std::collections::btree_map::Entry::Vacant(held) => {
                        held.insert(totals);
                    }
                }
            }
            let due = rollups.due();
            bound.due = due;
            if let Some(due) = due {
                self.prior.insert((due, tag));
            }
            let closed = closed
                .into_iter()
                .map(|((level, index), totals)| (level, index, totals))
                .collect();
            let mut direct = direct!(self, &mut *writer);
            direct.closed(tag, at, closed)?;
        }
        Ok(())
    }

    /// Binds, at the end of hour `at`, each country the lens binds whose state is the emit's.
    fn bind(&mut self, binds: Vec<Bind>, at: i32) {
        let mut named: FastMap<u32, Vec<u32>> = FastMap::default();
        for (&(raw, _), tags) in &self.ledger.identities {
            for &(_, tag) in tags {
                named.entry(tag).or_default().push(raw);
            }
        }
        for bind in binds {
            let bound = self.bound_of(&bind, &named, at);
            self.numbers
                .insert(bind.number, bound.as_ref().map(|(tag, _)| *tag));
            let Some((tag, mut bound)) = bound else {
                continue;
            };
            let ledger = &mut self.ledger;
            ledger.countries[tag as usize].materialize(at);
            for &raw in &bound.raws {
                ledger.lensed.insert(raw, tag);
            }
            ledger.volatile.remove(tag);
            for tags in ledger.tomorrow.values_mut() {
                tags.remove(&tag);
            }
            ledger.tomorrow.retain(|_, tags| !tags.is_empty());
            ledger.unschedule(tag);
            bound.due = ledger.countries[tag as usize].rollups.due();
            if let Some(due) = bound.due {
                self.prior.insert((due, tag));
            }
            self.bound.insert(tag, bound);
        }
    }

    /// The country `bind` names, bound as the lens binds it, where the emit holds it alike at
    /// the end of hour `at`: its slots, raw countries, their rows and capture identities, its
    /// accounting and what its keys hold. `named` holds the raw countries each tag's capture
    /// identities name.
    fn bound_of(
        &self,
        bind: &Bind,
        named: &FastMap<u32, Vec<u32>>,
        at: i32,
    ) -> Option<(u32, Bound)> {
        let ledger = &self.ledger;
        if (ledger.settled, ledger.rolled) != bind.settled {
            return None;
        }
        let &tag = ledger.numbers.get(&*bind.tag)?;
        if self.bound.contains_key(&tag) {
            return None;
        }
        let real = &ledger.countries[tag as usize];
        let lens = &bind.country;
        if !real.active || real.slots.is_empty() || real.slots != lens.slots {
            return None;
        }
        let raws = ledger.raws_of(tag);
        if raws.len() != bind.raws.len() || raws.iter().zip(&bind.raws).any(|(a, (b, _))| a != b) {
            return None;
        }
        if ledger
            .refused
            .keys()
            .any(|slot| ledger.known.get(slot) == Some(&tag))
            || named
                .get(&tag)
                .is_some_and(|named| named.iter().any(|raw| raws.binary_search(raw).is_err()))
        {
            return None;
        }
        let mut identities = Vec::new();
        for &raw in &raws {
            for (&key, tags) in ledger.identities.range((raw, 0)..=(raw, u64::MAX)) {
                if tags.iter().any(|&(_, held)| held != tag) {
                    return None;
                }
                identities.push(key);
            }
        }
        if identities != bind.identities
            || lens::stale(ledger, &raws) != bind.stale
            || bind
                .witnesses
                .iter()
                .any(|(raw, witness)| ledger.witnesses.get(raw) != witness.as_ref())
            || bind
                .command
                .iter()
                .any(|&(slot, value)| ledger.command.get(&slot).copied() != value)
        {
            return None;
        }
        let empty = Raw::default();
        let mut days = 0;
        for (raw, theirs) in &bind.raws {
            let ours = ledger.raws.get(raw).unwrap_or(&empty);
            if !same_rows(
                ours,
                theirs,
                (tag, bind.number),
                (real.items.as_slice(), lens.items.as_slice()),
            ) {
                return None;
            }
            days += ours.days.len();
        }
        if ledger.day_counts.get(tag as usize).copied() != Some(days as u32) {
            return None;
        }
        // The lens's categories as the emit numbers them; the emit's others the lens never paid.
        let dimension = |held: u64| -> Option<u64> {
            if held < CATEGORY_PARTS {
                return Some(held);
            }
            let (currency, name) = self.lens_names.get(&held)?;
            ledger.categories.get(*currency)?.get(&**name).copied()
        };
        let mut known = BTreeSet::new();
        for &held in lens.categories.keys() {
            let held = dimension(held)?;
            if !real.categories.contains_key(&held) {
                return None;
            }
            known.insert(held);
        }
        let mut extra: [Vec<u64>; 2] = Default::default();
        for &held in real.categories.keys() {
            if !known.contains(&held) {
                extra[*ledger.category_currency.get(&held)?].push(held);
            }
        }
        let day = at.div_euclid(24) * 24;
        let alike = match (&real.coverage, &lens.coverage) {
            (Some(ours), Some(theirs)) => ours.alike(theirs),
            (None, None) => true,
            _ => false,
        } && match (&real.spending, &lens.spending) {
            (Some(ours), Some(theirs)) => {
                ours.political == theirs.political
                    && ours.command == theirs.command
                    && ours.reason == theirs.reason
            }
            (None, None) => true,
            _ => false,
        } && real.cover == lens.cover
            && real.identity == lens.identity
            && real.holds == lens.holds
            && real.rates == lens.rates
            && real.rated.map(|rated| rated.max(at)) == lens.rated
            && raws.iter().all(|raw| match (real.parts.get(raw), lens.parts.get(raw)) {
                (Some(ours), Some(theirs)) => ours.alike(theirs, day, &dimension),
                (None, None) => true,
                _ => false,
            })
            && ledger.volatile.contains(tag) == bind.volatile
            && lens::tomorrow(ledger, tag) == bind.tomorrow;
        if !alike {
            return None;
        }
        // What the carry holds of its keys is what the lens's does.
        let keys = self.keys.get(tag as usize)?;
        let carried = |measure: u64, dimension: Option<u64>| {
            self.actors
                .held(Place {
                    actor: keys.actor,
                    measure,
                    dimension,
                })
                .and_then(|key| self.carry.held(key))
        };
        for &((measure, held), value) in &bind.held {
            let held = match held {
                Some(held) if measure == PARTS => Some(dimension(held)?),
                held => held,
            };
            if carried(measure, held) != value {
                return None;
            }
        }
        for (currency, extra) in extra.iter().enumerate() {
            if extra
                .iter()
                .any(|&held| carried(PARTS, Some(held)) != bind.unpaid[currency])
            {
                return None;
            }
        }
        let items = lens
            .items
            .iter()
            .map(|key| real.item_index.get(key).copied().unwrap_or(UNNAMED))
            .collect();
        Some((
            tag,
            Bound {
                number: bind.number,
                slots: real.slots.clone(),
                raws,
                items,
                extra,
                due: None,
                settled: Vec::new(),
            },
        ))
    }

    /// Takes back the countries the lens returns, as the hours it folded leave them.
    fn returned(&mut self, returned: Vec<Taken>) -> io::Result<()> {
        for taken in returned {
            let entry = self
                .numbers
                .remove(&taken.number)
                .ok_or_else(|| lens_failure("it returns a country it never bound"))?;
            let Some(tag) = entry else {
                continue;
            };
            let bound = self
                .bound
                .remove(&tag)
                .ok_or_else(|| lens_failure("it returns a country the emit did not bind"))?;
            if bound.number != taken.number || !bound.settled.is_empty() {
                return Err(lens_failure("it returns a country out of turn"));
            }
            self.install(tag, bound, taken)?;
        }
        Ok(())
    }

    /// Bound country `tag`'s accounting, its raw countries' rows and its rollups, as the lens
    /// hands them back in `taken`, with what the emit held of them before.
    fn install(&mut self, tag: u32, bound: Bound, taken: Taken) -> io::Result<()> {
        let Taken {
            country: lens,
            raws,
            stale,
            volatile,
            tomorrow,
            ..
        } = taken;
        let Bound {
            raws: held_raws,
            mut items,
            due,
            ..
        } = bound;
        let lens = *lens;
        let ledger = &mut self.ledger;
        // A lens item its ops never named was named before it bound: the emit holds its key.
        if items.len() < lens.items.len() {
            items.resize(lens.items.len(), UNNAMED);
        }
        for (ordinal, key) in lens.items.iter().enumerate() {
            if items[ordinal] == UNNAMED
                && let Some(&held) = ledger.countries[tag as usize].item_index.get(key)
            {
                items[ordinal] = held;
            }
        }
        let item = |ordinal: u32| -> io::Result<u32> {
            items
                .get(ordinal as usize)
                .copied()
                .filter(|&held| held != UNNAMED)
                .ok_or_else(|| lens_failure("it hands back an item it never named"))
        };
        for raw in &held_raws {
            ledger.lensed.remove(raw);
            ledger.stale_costs.remove(raw);
        }
        ledger.stale_costs.extend(stale);
        for (raw, mut held) in raws {
            for recorded in held.recorded_costs.values_mut() {
                recorded.item = item(recorded.item)?;
            }
            for day in held.days.values_mut() {
                day.tag = tag;
            }
            let count = held.days.len() as u32;
            if let Some(old) = ledger.raws.insert(raw, held) {
                for day in old.days.values() {
                    if let Some(count) = ledger.day_counts.get_mut(day.tag as usize) {
                        *count = count.saturating_sub(1);
                    }
                }
            }
            ledger.day_counts[tag as usize] += count;
        }
        let dimensions = &self.lens_dims;
        let dimension = |held: u64| -> io::Result<u64> {
            if held < CATEGORY_PARTS {
                return Ok(held);
            }
            dimensions
                .get(&held)
                .copied()
                .ok_or_else(|| lens_failure("it hands back a category it never named"))
        };
        let real = &mut ledger.countries[tag as usize];
        real.coverage = lens.coverage;
        real.spending = lens.spending;
        real.cover = lens.cover;
        real.identity = lens.identity;
        real.holds = lens.holds;
        for raw in &held_raws {
            match lens.parts.get(raw) {
                Some(parts) => {
                    let mut parts = parts.clone();
                    let mut values = BTreeMap::new();
                    for (held, value) in std::mem::take(&mut parts.values) {
                        values.insert(dimension(held)?, value);
                    }
                    parts.values = values;
                    real.parts.insert(*raw, parts);
                }
                None => {
                    real.parts.remove(raw);
                }
            }
        }
        real.rates = lens.rates;
        real.rated = lens.rated;
        for (reason, count) in lens.warnings {
            *real.warnings.entry(reason).or_default() += count;
        }
        real.warnings_changed |= lens.warnings_changed;
        real.rollups.merge(lens.rollups.remap(&item)?)?;
        if volatile {
            ledger.volatile.insert(tag);
        }
        for at in tomorrow {
            ledger.tomorrow.entry(at).or_default().insert(tag);
        }
        ledger.reschedule(tag);
        if let Some(due) = due {
            self.prior.remove(&(due, tag));
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts next,
    /// with its evidence and every scalar that changed since the last cut.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        if !self.bound.is_empty() {
            return Err(lens_failure("it did not hand its countries back before a cut"));
        }
        // A blob of the segment cut is not referred to again.
        self.record = (Vec::new(), Blob::EMPTY);
        self.carry.cut(writer, last)?;
        self.evidence(writer, last)?;
        if self.ledger.last == Some(last) {
            self.known_from = None;
        }
        if self.ledger.settled != self.settled_written
            && let Some(settled) = self.ledger.settled
        {
            writer.catalogue(SETTLED, Cell::Integer(i128::from(settled)))?;
            self.settled_written = self.ledger.settled;
        }
        for country in &mut self.ledger.countries {
            country.materialize(last);
        }
        if self.ledger.warnings_changed {
            let cell = reasons(writer, &self.ledger.warnings)?;
            writer.catalogue(WARNINGS, cell)?;
            self.ledger.warnings_changed = false;
        }
        // The content's power diagnostics are reachable once an item demanded its power content.
        if !self.power_warned
            && let Some(content) = self.ledger.content.as_deref()
            && content.power_requested()
        {
            let mut cells = Vec::new();
            for warning in content.power_warnings() {
                cells.push(text(writer, warning.as_bytes())?);
            }
            let cell = Cell::List(writer.cells(cells)?);
            writer.catalogue(POWER_WARNINGS, cell)?;
            self.power_warned = true;
        }
        for at in 0..self.ledger.countries.len() {
            let actor = self.keys[at].actor;
            let country = &mut self.ledger.countries[at];
            if country.warnings_changed {
                let cell = reasons(writer, &country.warnings)?;
                writer.catalogue(TAG_WARNINGS + actor, cell)?;
                country.warnings_changed = false;
            }
            if country.categories_changed {
                let mut cells = Vec::new();
                for (&dimension, &first) in &country.categories {
                    let pair = writer
                        .cells([Cell::Unsigned(dimension), Cell::Integer(i128::from(first))])?;
                    cells.push(Cell::List(pair));
                }
                let cell = Cell::List(writer.cells(cells)?);
                writer.catalogue(TAG_CATEGORIES + actor, cell)?;
                country.categories_changed = false;
            }
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour, whose
    /// projection hands every table whole.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if !self.bound.is_empty() {
            return Err(lens_failure("it did not hand its countries back before a save"));
        }
        let ledger = &self.ledger;
        let state = Saved {
            last: ledger.last,
            known_from: self.known_from,
            begun: self.begun,
            census_complete: ledger.census_complete,
            tags: ledger.tags.iter().map(|tag| tag.to_string()).collect(),
            countries: &ledger.countries,
            refused: ledger.refused.clone(),
            volatile: ledger.volatile.iter().collect(),
            complete: ledger.complete,
            command: ledger
                .command
                .iter()
                .map(|(&slot, &value)| (slot, value))
                .collect(),
            presence: self.presence.as_ref().map(|(present, refused)| {
                (
                    present.iter().map(|tag| tag.to_string()).collect(),
                    refused.iter().map(|tag| tag.to_string()).collect(),
                )
            }),
            settled: ledger.settled,
            settled_written: self.settled_written,
            warnings: ledger.warnings.clone(),
            categories: ledger
                .categories
                .iter()
                .enumerate()
                .flat_map(|(currency, categories)| {
                    categories.iter().map(move |(category, &dimension)| {
                        (ledger::spelling(currency, category), dimension)
                    })
                })
                .collect(),
            tomorrow: ledger.tomorrow.clone(),
        };
        let json = serde_json::to_vec(&state)?;
        // The census, links, captures and each raw country's held rows with their records.
        let mut rows = Vec::new();
        let mut put = tally::Out(&mut rows);
        put.len(ledger.census.len());
        for row in ledger.census.values() {
            put.var(u64::from(row.id));
            put.var(u64::from(row.slot));
            put_bytes(&mut put, &row.tag);
        }
        put_pairs(&mut put, &ledger.known);
        put_pairs(&mut put, &ledger.links);
        put_words(&mut put, &ledger.extended);
        put.int(ledger.extended_through);
        put_words(&mut put, &ledger.advanced);
        put.len(ledger.identities.len());
        for (&(country, epoch), tags) in &ledger.identities {
            put.var(u64::from(country));
            put.var(epoch);
            put.len(tags.len());
            for &(hour, tag) in tags {
                put.int(hour);
                put.var(u64::from(tag));
            }
        }
        put.len(ledger.witnesses.len());
        for row in ledger.witnesses.values() {
            put_capture(&mut put, row);
        }
        let mut raws: Vec<(&u32, &Raw)> = ledger.raws.iter().collect();
        raws.sort_unstable_by_key(|(raw, _)| **raw);
        put.len(raws.len());
        for (&raw, held) in raws {
            put.var(u64::from(raw));
            put.len(held.recorded.len());
            for &(epoch, sequence) in &held.recorded {
                put.var(epoch);
                put.var(sequence);
            }
            put.flag(held.daily.is_some());
            if let Some(row) = &held.daily {
                put_daily(&mut put, row);
            }
            put.len(held.days.len());
            for day in held.days.values() {
                put_daily(&mut put, &day.row);
                put_hour(&mut put, day.reads.0);
                put_hour(&mut put, day.reads.1);
                put.var(u64::from(day.tag));
            }
            put.len(held.costs.len());
            for cost in held.costs.values() {
                put.var(cost.epoch);
                put.int(cost.day_hour);
                put.var(cost.pp_sequence);
                put.var(u64::from(cost.ordinal));
                put.zig128(i128::from(cost.cost_raw));
            }
            put.len(held.recorded_costs.len());
            for cost in held.recorded_costs.values() {
                put_cost(&mut put, &cost.row);
                put.flag(cost.covered);
                put.var(u64::from(cost.item));
            }
        }
        // Each country's rollups and items, the bulk of a long campaign's state, as varints.
        let mut binary = Vec::new();
        let mut put = tally::Out(&mut binary);
        for country in &ledger.countries {
            country.rollups.save(&mut put);
            put.len(country.items.len());
            for key in &country.items {
                put.0.extend_from_slice(key);
            }
        }
        for part in [&json, &rows, &binary] {
            out.extend_from_slice(&(part.len() as u64).to_le_bytes());
            out.extend_from_slice(part);
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        content: &Result<Option<Arc<PoliticsContent>>, String>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let mut parts = Vec::with_capacity(3);
        for _ in 0..3 {
            let length = usize::try_from(d.u64()?).map_err(|_| invalid("saved part length"))?;
            parts.push(d.bytes(length)?);
        }
        let at = d.position();
        let saved: Loaded = serde_json::from_slice(parts[0])?;
        let mut emit = Self::new(content);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(&bytes[at..])?;
        emit.known_from = saved.known_from;
        emit.begun = saved.begun;
        emit.settled_written = saved.settled_written;
        emit.presence = saved.presence.map(|(present, refused)| {
            (
                present.into_iter().map(Into::into).collect(),
                refused.into_iter().map(Into::into).collect(),
            )
        });
        for tag in &saved.tags {
            let actor = emit
                .actors
                .find(tag.as_bytes())
                .ok_or_else(|| invalid("a saved tag without its actor"))?;
            emit.keys.push(Keys::of(actor));
        }
        let ledger = &mut emit.ledger;
        ledger.last = saved.last;
        ledger.census_complete = saved.census_complete;
        ledger.tags = saved.tags.into_iter().map(Into::into).collect();
        ledger.numbers = ledger
            .tags
            .iter()
            .enumerate()
            .map(|(at, tag)| (tag.clone(), at as u32))
            .collect();
        ledger.countries = saved.countries;
        if ledger.countries.len() != ledger.tags.len() {
            return Err(invalid("saved countries and tags disagree"));
        }
        let mut binary = Decode::new(parts[2]);
        for country in &mut ledger.countries {
            country.rollups = tally::Rollups::load(&mut binary)?;
            let count = tally::len(&mut binary)?;
            country
                .items
                .try_reserve(count)
                .map_err(|_| invalid("saved items exceed memory"))?;
            for _ in 0..count {
                country.items.push(binary.array::<32>()?);
            }
            country.item_index = country
                .items
                .iter()
                .enumerate()
                .map(|(ordinal, key)| (*key, ordinal as u32))
                .collect();
        }
        binary.end()?;
        ledger.refused = saved.refused;
        let mut rows = Decode::new(parts[1]);
        for _ in 0..tally::len(&mut rows)? {
            let id = tally::word(&mut rows)?;
            let slot = tally::word(&mut rows)?;
            let tag = Arc::from(read_bytes(&mut rows)?);
            ledger.census.insert(slot, facts::CensusRow { id, slot, tag });
        }
        ledger.known = read_pairs(&mut rows)?;
        ledger.links = read_pairs(&mut rows)?;
        ledger.extended = Arc::from(read_words(&mut rows)?);
        ledger.extended_through = tally::int(&mut rows)?;
        ledger.advanced = read_words(&mut rows)?;
        for _ in 0..tally::len(&mut rows)? {
            let identity = (tally::word(&mut rows)?, rows.var()?);
            let mut tags = Vec::new();
            for _ in 0..tally::len(&mut rows)? {
                tags.push((tally::int(&mut rows)?, tally::word(&mut rows)?));
            }
            ledger.identities.insert(identity, tags);
        }
        for _ in 0..tally::len(&mut rows)? {
            let row = read_capture(&mut rows)?;
            ledger.witnesses.insert(row.country, row);
        }
        ledger.day_counts = vec![0; ledger.tags.len()];
        for _ in 0..tally::len(&mut rows)? {
            let country = tally::word(&mut rows)?;
            let mut held = Raw::default();
            for _ in 0..tally::len(&mut rows)? {
                held.recorded.insert((rows.var()?, rows.var()?));
            }
            if rows.flag()? {
                held.daily = Some(read_daily(&mut rows)?);
            }
            for _ in 0..tally::len(&mut rows)? {
                let row = read_daily(&mut rows)?;
                let reads = (read_hour(&mut rows)?, read_hour(&mut rows)?);
                let tag = tally::word(&mut rows)?;
                *ledger
                    .day_counts
                    .get_mut(tag as usize)
                    .ok_or_else(damaged)? += 1;
                held.days
                    .insert((row.epoch, row.day_hour), ledger::Day { row, reads, tag });
            }
            for _ in 0..tally::len(&mut rows)? {
                let cost = ledger::HeldCost {
                    country,
                    epoch: rows.var()?,
                    day_hour: tally::int(&mut rows)?,
                    pp_sequence: rows.var()?,
                    ordinal: tally::word(&mut rows)?,
                    cost_raw: signed(&mut rows)?,
                };
                let sum = held
                    .sums
                    .entry((cost.epoch, cost.day_hour, cost.pp_sequence))
                    .or_default();
                sum.0 = sum.0.wrapping_add(cost.cost_raw);
                sum.1 += 1;
                held.costs.insert(cost.key(), cost);
            }
            for _ in 0..tally::len(&mut rows)? {
                let row = read_cost(&mut rows)?;
                let covered = rows.flag()?;
                let item = tally::word(&mut rows)?;
                held.recorded_costs.insert(
                    (row.epoch, row.day_hour, row.ordinal),
                    ledger::Recorded { row, covered, item },
                );
            }
            ledger.raws.insert(country, held);
        }
        rows.end()?;
        for (&raw, &slot) in &ledger.links {
            ledger.raw_slots.entry(slot).or_default().push(raw);
        }
        for raws in ledger.raw_slots.values_mut() {
            raws.sort_unstable();
        }
        let slots: Vec<(u32, facts::CensusRow)> = ledger
            .census
            .iter()
            .map(|(&slot, row)| (slot, row.clone()))
            .collect();
        for (slot, row) in slots {
            if let Ok(spelling) = rows::valid_tag(row.id, row.slot, &row.tag)
                && let Some(&tag) = ledger.numbers.get(spelling)
            {
                ledger.slot_tags.insert(slot, tag);
                ledger.countries[tag as usize].slots.push(slot);
            }
        }
        ledger.volatile = saved.volatile.into_iter().collect();
        ledger.complete = saved.complete;
        ledger.command = saved.command.into_iter().collect();
        ledger.settled = saved.settled;
        ledger.warnings = saved.warnings;
        for (spelling, dimension) in saved.categories {
            let (index, category) = CURRENCIES
                .iter()
                .enumerate()
                .find_map(|(index, currency)| {
                    let category = spelling.strip_prefix(currency)?.strip_prefix(':')?;
                    Some((index, category))
                })
                .ok_or_else(|| invalid("saved category spelling"))?;
            ledger.category_currency.insert(dimension, index);
            ledger.categories[index].insert(category.into(), dimension);
        }
        ledger.tomorrow = saved.tomorrow;
        for tag in 0..ledger.countries.len() as u32 {
            ledger.reschedule(tag);
        }
        Ok(emit)
    }
}

/// Whether the emit's rows of a raw country, `ours`, are the lens's, `theirs`: the lens's days
/// recorded under its own number for the tag, its records naming its own items.
fn same_rows(
    ours: &Raw,
    theirs: &Raw,
    (tag, number): (u32, u32),
    (our_items, their_items): (&[[u8; 32]], &[[u8; 32]]),
) -> bool {
    ours.power == theirs.power
        && ours.governments == theirs.governments
        && ours.recorded == theirs.recorded
        && ours.waiting.is_empty()
        && theirs.waiting.is_empty()
        && ours.daily == theirs.daily
        && ours.costs == theirs.costs
        && ours.sums == theirs.sums
        && ours.days.len() == theirs.days.len()
        && ours.days.iter().zip(&theirs.days).all(|((key, day), (held, other))| {
            key == held
                && day.row == other.row
                && day.reads == other.reads
                && day.tag == tag
                && other.tag == number
        })
        && ours.recorded_costs.len() == theirs.recorded_costs.len()
        && ours.recorded_costs.iter().all(|(key, cost)| {
            theirs.recorded_costs.get(key).is_some_and(|other| {
                cost.row == other.row
                    && cost.covered == other.covered
                    && our_items.get(cost.item as usize).is_some()
                    && our_items.get(cost.item as usize) == their_items.get(other.item as usize)
            })
        })
}

/// The packed record at `at..at + len` of a lens part's `payload`.
fn slice(payload: &[u8], (at, len): (u32, u32)) -> io::Result<&[u8]> {
    payload
        .get(at as usize..(at as usize).saturating_add(len as usize))
        .ok_or_else(|| lens_failure("a record outside its payload"))
}

/// `split`, its fields' items named by `item`.
fn remap_split(
    split: tally::Split,
    item: &impl Fn(u32) -> io::Result<u32>,
) -> io::Result<tally::Split> {
    let mut days = Vec::with_capacity(split.days.len());
    for ((at, field), day) in split.days {
        days.push(((at, (item(field.0)?, field.1, field.2)), day));
    }
    Ok(tally::Split {
        days,
        daily: split.daily,
    })
}

/// `reasons` as a catalogue list of (reason, count) pairs.
fn reasons(writer: &mut Writer, reasons: &BTreeMap<String, u64>) -> io::Result<Cell> {
    let mut cells = Vec::with_capacity(reasons.len());
    for (reason, &count) in reasons {
        let pair = [text(writer, reason.as_bytes())?, Cell::Unsigned(count)];
        cells.push(Cell::List(writer.cells(pair)?));
    }
    Ok(Cell::List(writer.cells(cells)?))
}

#[derive(Serialize)]
struct Saved<'a> {
    last: Option<i32>,
    known_from: Option<i32>,
    begun: bool,
    census_complete: bool,
    tags: Vec<String>,
    countries: &'a [Country],
    refused: BTreeMap<u32, String>,
    volatile: Vec<u32>,
    complete: u16,
    command: Vec<(u32, i64)>,
    presence: Option<(Vec<String>, Vec<String>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    warnings: BTreeMap<String, u64>,
    categories: Vec<(String, u64)>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

#[derive(Deserialize)]
struct Loaded {
    last: Option<i32>,
    known_from: Option<i32>,
    begun: bool,
    census_complete: bool,
    tags: Vec<String>,
    countries: Vec<Country>,
    refused: BTreeMap<u32, String>,
    volatile: Vec<u32>,
    complete: u16,
    command: Vec<(u32, i64)>,
    presence: Option<(Vec<String>, Vec<String>)>,
    settled: Option<i32>,
    settled_written: Option<i32>,
    warnings: BTreeMap<String, u64>,
    categories: Vec<(String, u64)>,
    tomorrow: BTreeMap<i32, BTreeSet<u32>>,
}

/// The saved state's varints ([`tally::Out`]): a length before its bytes, a flag before a value
/// that may be missing, enums by their recorded wire values.
fn put_bytes(put: &mut tally::Out<'_>, bytes: &[u8]) {
    put.len(bytes.len());
    put.0.extend_from_slice(bytes);
}

fn put_some(put: &mut tally::Out<'_>, value: Option<u64>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.var(value);
    }
}

fn put_signed(put: &mut tally::Out<'_>, value: Option<i64>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.zig128(i128::from(value));
    }
}

fn put_hour(put: &mut tally::Out<'_>, value: Option<i32>) {
    put.flag(value.is_some());
    if let Some(value) = value {
        put.int(value);
    }
}

fn put_pairs(put: &mut tally::Out<'_>, pairs: &FastMap<u32, u32>) {
    put.len(pairs.len());
    for (&key, &value) in pairs {
        put.var(u64::from(key));
        put.var(u64::from(value));
    }
}

fn put_words(put: &mut tally::Out<'_>, words: &[u32]) {
    put.len(words.len());
    for &word in words {
        put.var(u64::from(word));
    }
}

fn put_capture(put: &mut tally::Out<'_>, row: &PoliticsCaptureCoverageRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(row.epoch);
    put.int(row.since_hour);
    put.int(row.through_hour);
    put.var(u64::from(row.streams));
}

fn put_daily(put: &mut tally::Out<'_>, row: &CountryPowerDailyRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(row.epoch);
    put.int(row.day_hour);
    put_signed(put, row.pp_gross_gain_raw);
    put_signed(put, row.pp_cost_raw);
    put_signed(put, row.pp_net_gain_raw);
    put_signed(put, row.pp_realized_delta_raw);
    put_some(put, row.pp_sequence);
    put_signed(put, row.cp_gain_raw);
    put_signed(put, row.cp_realized_delta_raw);
    put_some(put, row.cp_sequence);
}

fn put_cost(put: &mut tally::Out<'_>, row: &CountryPowerDailyCostRow) {
    put.var(u64::from(row.country));
    put.var(u64::from(row.instance));
    put.var(u64::from(row.ordinal));
    put.var(row.epoch);
    put.int(row.day_hour);
    put.var(row.pp_sequence);
    put.var(row.component as u64);
    put.zig128(i128::from(row.cost_raw));
    put.var(row.cause as u64);
    put.var(row.site as u64);
    let identity = &row.identity;
    put.var(identity.action as u64);
    put_some(put, identity.item_token.map(u64::from));
    put_bytes(put, &identity.item_key);
    put_bytes(put, &identity.slot_key);
    put_some(put, identity.character.map(u64::from));
    put.var(identity.object_kind as u64);
    put_some(put, identity.object_reference);
    put_some(put, identity.target_country.map(u64::from));
    put_some(put, identity.target_state.map(u64::from));
    put_some(put, identity.target_region.map(u64::from));
    put.var(identity.origin_kind as u64);
    put_some(put, identity.origin_token.map(u64::from));
    put_bytes(put, &identity.origin_key);
}

fn damaged() -> io::Error {
    invalid("saved state is damaged")
}

fn read_bytes(input: &mut Decode<'_>) -> io::Result<Vec<u8>> {
    let len = tally::len(input)?;
    Ok(input.bytes(len)?.to_vec())
}

fn read_some(input: &mut Decode<'_>) -> io::Result<Option<u64>> {
    Ok(if input.flag()? { Some(input.var()?) } else { None })
}

fn read_word(input: &mut Decode<'_>) -> io::Result<Option<u32>> {
    read_some(input)?
        .map(|value| u32::try_from(value).map_err(|_| damaged()))
        .transpose()
}

fn signed(input: &mut Decode<'_>) -> io::Result<i64> {
    i64::try_from(input.zig128()?).map_err(|_| damaged())
}

fn read_signed(input: &mut Decode<'_>) -> io::Result<Option<i64>> {
    Ok(if input.flag()? { Some(signed(input)?) } else { None })
}

fn read_hour(input: &mut Decode<'_>) -> io::Result<Option<i32>> {
    Ok(if input.flag()? {
        Some(tally::int(input)?)
    } else {
        None
    })
}

fn narrow<T: TryFrom<u64>>(input: &mut Decode<'_>) -> io::Result<T> {
    T::try_from(input.var()?).map_err(|_| damaged())
}

fn read_pairs(input: &mut Decode<'_>) -> io::Result<FastMap<u32, u32>> {
    let mut pairs = FastMap::default();
    for _ in 0..tally::len(input)? {
        pairs.insert(tally::word(input)?, tally::word(input)?);
    }
    Ok(pairs)
}

fn read_words(input: &mut Decode<'_>) -> io::Result<Vec<u32>> {
    let mut words = Vec::new();
    for _ in 0..tally::len(input)? {
        words.push(tally::word(input)?);
    }
    Ok(words)
}

fn read_capture(input: &mut Decode<'_>) -> io::Result<PoliticsCaptureCoverageRow> {
    Ok(PoliticsCaptureCoverageRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        epoch: input.var()?,
        since_hour: tally::int(input)?,
        through_hour: tally::int(input)?,
        streams: narrow(input)?,
    })
}

fn read_daily(input: &mut Decode<'_>) -> io::Result<CountryPowerDailyRow> {
    Ok(CountryPowerDailyRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        epoch: input.var()?,
        day_hour: tally::int(input)?,
        pp_gross_gain_raw: read_signed(input)?,
        pp_cost_raw: read_signed(input)?,
        pp_net_gain_raw: read_signed(input)?,
        pp_realized_delta_raw: read_signed(input)?,
        pp_sequence: read_some(input)?,
        cp_gain_raw: read_signed(input)?,
        cp_realized_delta_raw: read_signed(input)?,
        cp_sequence: read_some(input)?,
    })
}

fn read_cost(input: &mut Decode<'_>) -> io::Result<CountryPowerDailyCostRow> {
    Ok(CountryPowerDailyCostRow {
        country: tally::word(input)?,
        instance: tally::word(input)?,
        ordinal: tally::word(input)?,
        epoch: input.var()?,
        day_hour: tally::int(input)?,
        pp_sequence: input.var()?,
        component: PowerDailyCostKind::try_from(narrow::<u8>(input)?)?,
        cost_raw: signed(input)?,
        cause: PowerCause::try_from(narrow::<u16>(input)?)?,
        site: PowerSite::try_from(narrow::<u16>(input)?)?,
        identity: PowerIdentity {
            action: PowerAction::try_from(narrow::<u8>(input)?)?,
            item_token: read_word(input)?,
            item_key: read_bytes(input)?,
            slot_key: read_bytes(input)?,
            character: read_word(input)?,
            object_kind: PowerObjectKind::try_from(narrow::<u8>(input)?)?,
            object_reference: read_some(input)?,
            target_country: read_word(input)?,
            target_state: read_word(input)?,
            target_region: read_word(input)?,
            origin_kind: PowerOriginKind::try_from(narrow::<u8>(input)?)?,
            origin_token: read_word(input)?,
            origin_key: read_bytes(input)?,
        },
    })
}
