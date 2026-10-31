//! The yard views' emit: an owner's rows as day events on its family's series publication
//! ([`super::layout`]), the coverage of the hour's observation and of each source table, the
//! dating table's read schedule, and each token's spelling where the branch first numbers it.
//! Only the emit numbers tokens and dates the sources' supplying reads; every hour's rows come
//! from its segment's reader ([`super::segment`]).
use super::{
    layout::{self, FAMILY, READS, SCHEMA, kind, reason},
    project::{Key, Label, Row, Spelled},
    segment::{Read, Rows, View},
    *,
};
use dashboard_keyed::segment::Hour;
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::{self, Carry},
    codec::Decode,
    family::{Actors, Directory, Place},
    hash::FastMap,
};
use replay_data::cadence::{self, Cadence};
use std::marker::PhantomData;

#[cfg(test)]
pub(crate) mod tests;

/// The schedule's evidence: every walked hour is known, as the engine requires of a run.
const WALKED: carry::Read = carry::Read {
    holder: None,
    read: None,
    identity_first: None,
    presented: false,
};

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// A coverage from an hour on: known (with its supplying read), or unknown for a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Known(Option<i32>),
    Unknown(u64),
}

/// A coverage key's open evidence: its status and the first hour not yet appended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Coverage {
    open: Option<(Status, i32)>,
}

impl Coverage {
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
    let (known, supplying_read) = match status {
        Status::Known(read) => (Knownness::Known, read),
        Status::Unknown(reason) => (Knownness::Unknown(reason), None),
    };
    writer.evidence(
        key,
        Evidence {
            span: Span::new(first, last)?,
            known,
            supplying_read,
            holder: None,
            identity_first: None,
        },
        None,
    )
}

/// A held cell: a token as its number, a list as its cells.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Held {
    Null,
    Integer(i128),
    Unsigned(u64),
    List(Box<[Held]>),
}

/// A stream as its last event left it: its rows' width, its rows by key, and its chain: the
/// keyframe's hour and the events in it; none where the next event is a keyframe.
#[derive(Debug)]
struct Stream {
    width: usize,
    rows: BTreeMap<u64, Box<[Held]>>,
    chain: Option<(i32, u8)>,
}

/// The cells of a row of `stream`, its key's and whether it is held included.
fn cells_of(stream: u64, refits: bool) -> io::Result<usize> {
    Ok(match stream {
        layout::HOLDERS if refits => layout::line::CELLS,
        layout::HOLDERS => layout::port::CELLS,
        layout::SHIPS => layout::ship::CELLS,
        layout::TOTALS if refits => layout::refit_totals::CELLS,
        layout::TOTALS => layout::repair_totals::CELLS,
        layout::REFUSALS => layout::refusal::CELLS,
        _ => return Err(invalid("A yard row of no stream")),
    })
}

/// The 128-bit digest a token is numbered by.
fn digest(key: &str) -> u128 {
    let hash = blake3::hash(key.as_bytes());
    u128::from_le_bytes(hash.as_bytes()[..16].try_into().expect("a digest's first 16 bytes"))
}

/// A yard view's emit on one branch.
pub struct Emit<V> {
    /// The dating table's read period by its cadence: 0 where only off-slot reads read it.
    period: i32,
    actors: Actors,
    carry: Carry,
    /// OBSERVED and READS.
    family: Option<[KeyHandle; 2]>,
    observed: Coverage,
    /// Each source table's key and coverage, in the view's source order.
    sources: Vec<Option<(KeyHandle, Coverage)>>,
    /// Each token's number, by its spelling's digest.
    tokens: FastMap<u128, u64>,
    streams: FastMap<KeyHandle, Stream>,
    /// The actors of the owners an hour last named, by their number there.
    owners: Option<(Arc<[String]>, Vec<u64>)>,
    /// Each dated source's supplying read, by table index; the hour they were last dated at.
    dates: BTreeMap<usize, i32>,
    dated: Option<i32>,
    /// The last hour walked, and the first of the hours walked one after another through it.
    last: Option<i32>,
    streak: Option<i32>,
    /// Whether the schedule's evidence is open: from the first hour after a break.
    walking: bool,
    /// The schedule's next read.
    next: Option<i32>,
    cells: Vec<Cell>,
    view: PhantomData<fn() -> V>,
}

impl<V: View> Emit<V> {
    /// The emit of a branch of a recording in `format`.
    pub fn new(format: FormatVersion) -> Self {
        Self {
            period: match cadence::cadence_in(V::READ, format) {
                Cadence::Daily(_) => cadence::STAGGER_PERIOD,
                Cadence::Hourly | Cadence::Sliced => 1,
                Cadence::Static => 0,
            },
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            family: None,
            observed: Coverage::default(),
            sources: vec![None; V::SOURCES.len()],
            tokens: FastMap::default(),
            streams: FastMap::default(),
            owners: None,
            dates: BTreeMap::new(),
            dated: None,
            last: None,
            streak: None,
            walking: false,
            next: None,
            cells: Vec::new(),
            view: PhantomData,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors.key(writer, place, (RunMode::Exact, 0), None)
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 2]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([layout::OBSERVED, READS]) {
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

    /// Keeps one hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour<V>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("naval-yards.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(invalid("A yard hour does not follow the hour before"));
            }
            // An hour not walked ends every coverage and the schedule.
            Some(last) => self.close(writer, last)?,
            None => {}
        }
        if self.last.and_then(|last| last.checked_add(1)) != Some(at) {
            self.streak = Some(at);
        }
        self.last = Some(at);
        let [observed, reads] = self.family(writer)?;
        if !self.walking {
            self.carry.evidence(writer, reads, at, Some(WALKED))?;
            self.walking = true;
        }
        self.schedule(writer, reads, at, hour.dated)?;
        let Some(Read { sources, rows }) = hour.read else {
            return Ok(());
        };
        match rows {
            None => {
                // No date carries past an hour whose owners do not resolve.
                self.dates.clear();
                self.dated = None;
                self.observed
                    .set(writer, observed, at, Status::Unknown(reason::COUNTRIES))?;
                self.sources(writer, at, &sources)
            }
            Some(rows) => {
                let starts = !matches!(self.observed.open, Some((Status::Known(_), _)));
                self.date(&sources);
                self.sources(writer, at, &sources)?;
                self.observed.set(writer, observed, at, Status::Known(None))?;
                self.rows(writer, at, hour.dated.then_some(at), rows, starts)
            }
        }
    }

    /// Ends what is open after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        self.walking = false;
        self.next = None;
        let [observed, _] = self.family(writer)?;
        self.observed.cut(writer, observed, last)?;
        self.observed.open = None;
        for (key, coverage) in self.sources.iter_mut().flatten() {
            coverage.cut(writer, *key, last)?;
            coverage.open = None;
        }
        Ok(())
    }

    /// The dating table's read schedule at `at`: a read off schedule starts one, a read due and
    /// missed ends it.
    fn schedule(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        read: bool,
    ) -> io::Result<()> {
        if read {
            if self.next != Some(at) {
                self.carry
                    .value(writer, key, at, Some(i128::from(self.period)), true)?;
            }
            self.next = (self.period > 0).then(|| at + self.period);
        } else if self.next == Some(at) {
            self.carry.value(writer, key, at, None, false)?;
            self.next = None;
        }
        Ok(())
    }

    /// Each source's supplying read at the hour: dates carry only across hours walked one
    /// after another since they were last dated.
    fn date(&mut self, sources: &Sources) {
        let at = sources.hour;
        if !self
            .dated
            .zip(self.streak)
            .is_some_and(|(last, first)| first <= last && last < at)
        {
            self.dates.clear();
        }
        self.dated = Some(at);
        for source in &sources.tables {
            let table = source.table.index();
            if source.missing.is_some() {
                self.dates.remove(&table);
            } else if source.read {
                self.dates.insert(table, at);
            } else if let Some(hour) = source.sampled {
                // A same-cadence sibling can witness an equal read later than
                // the table's own carried sample stamp.
                self.dates
                    .entry(table)
                    .and_modify(|read| *read = (*read).max(hour))
                    .or_insert(hour);
            }
        }
    }

    /// Each source table's coverage from `at`.
    fn sources(&mut self, writer: &mut Writer<'_>, at: i32, sources: &Sources) -> io::Result<()> {
        for (at_source, source) in sources.tables.iter().enumerate() {
            let status = match &source.missing {
                Some(why) => {
                    let label = Label::new(Operand::Refusal(why.clone()));
                    Status::Unknown(self.token(writer, at, &label)?)
                }
                None => Status::Known(
                    source
                        .dated
                        .then(|| self.dates.get(&source.table.index()).copied())
                        .flatten(),
                ),
            };
            let held = self.sources[at_source].map(|(key, _)| key);
            let key = match held {
                Some(key) => key,
                None => {
                    let key = self.key(
                        writer,
                        Place {
                            actor: FAMILY,
                            measure: layout::SOURCE,
                            dimension: Some(source.table.index() as u64),
                        },
                    )?;
                    self.sources[at_source] = Some((key, Coverage::default()));
                    key
                }
            };
            if let Some((_, coverage)) = &mut self.sources[at_source] {
                coverage.set(writer, key, at, status)?;
            }
        }
        Ok(())
    }

    /// The number of the token `label` spells, numbered and spelled at `at` when it is new.
    fn token(&mut self, writer: &mut Writer<'_>, at: i32, label: &Label) -> io::Result<u64> {
        let digest = digest(&label.key);
        if let Some(&number) = self.tokens.get(&digest) {
            return Ok(number);
        }
        let number = self.tokens.len() as u64;
        let key = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: layout::TOKEN,
                dimension: Some(number),
            },
        )?;
        let text = writer.text(&label.key)?;
        let fields = writer.cells([Cell::Text(text)])?;
        event(writer, key, at, None, kind::TOKEN, fields)?;
        engine_allocation::reserve_map(&mut self.tokens, 1)?;
        self.tokens.insert(digest, number);
        Ok(number)
    }

    /// The actors of `owners`, by their number there.
    fn actors(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        owners: &Arc<[String]>,
    ) -> io::Result<Vec<u64>> {
        if let Some((held, actors)) = &self.owners
            && Arc::ptr_eq(held, owners)
        {
            return Ok(actors.clone());
        }
        let mut actors = Vec::with_capacity(owners.len());
        for owner in owners.iter() {
            actors.push(self.actors.actor(writer, owner.as_bytes(), at)?);
        }
        self.owners = Some((owners.clone(), actors.clone()));
        Ok(actors)
    }

    fn held(&mut self, writer: &mut Writer<'_>, at: i32, cell: &Spelled) -> io::Result<Held> {
        Ok(match cell {
            Spelled::Null => Held::Null,
            Spelled::Integer(value) => Held::Integer(*value),
            Spelled::Unsigned(value) => Held::Unsigned(*value),
            Spelled::Token(label) => Held::Unsigned(self.token(writer, at, label)?),
            Spelled::List(cells) => Held::List(
                cells
                    .iter()
                    .map(|cell| self.held(writer, at, cell))
                    .collect::<io::Result<_>>()?,
            ),
        })
    }

    /// The hour's rows, each stream they change written as one event; where they are every
    /// row, every row held and not among them is removed; where the observation's coverage
    /// `starts`, every stream starts a chain, and one holding rows writes its keyframe.
    fn rows(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        read: Option<i32>,
        rows: Rows,
        starts: bool,
    ) -> io::Result<()> {
        if starts {
            for stream in self.streams.values_mut() {
                stream.chain = None;
            }
        }
        let actors = self.actors(writer, at, &rows.owners)?;
        let mut touched = Vec::new();
        let (mut changed, mut given) = (Vec::new(), Vec::new());
        let whole = rows.whole;
        let mut rows = rows.rows.into_iter().peekable();
        while let Some(first) = rows.peek() {
            let (owner, stream) = (first.owner, first.stream);
            let actor = *actors
                .get(owner as usize)
                .ok_or_else(|| invalid("A yard row names an owner the hour does not"))?;
            let key = self.key(
                writer,
                Place {
                    actor,
                    measure: stream,
                    dimension: None,
                },
            )?;
            let width = cells_of(stream, V::REFITS)?;
            self.streams.entry(key).or_insert_with(|| Stream {
                width,
                rows: BTreeMap::new(),
                chain: None,
            });
            changed.clear();
            given.clear();
            while let Some(row) = rows.next_if(|row| row.owner == owner && row.stream == stream) {
                let Row {
                    key: row_key,
                    cells,
                    ..
                } = row;
                let number = match &row_key {
                    Key::Number(number) => *number,
                    Key::Token(label) => self.token(writer, at, label)?,
                };
                if whole {
                    given.push(number);
                }
                let cells = match cells {
                    None => None,
                    Some(cells) if cells.len() + 2 == width => Some(
                        cells
                            .iter()
                            .map(|cell| self.held(writer, at, cell))
                            .collect::<io::Result<Box<[Held]>>>()?,
                    ),
                    Some(_) => return Err(invalid("A yard row's cells are not its stream's")),
                };
                let stream = self
                    .streams
                    .get_mut(&key)
                    .ok_or_else(|| invalid("A yard row of no stream"))?;
                let before = match &cells {
                    Some(cells) => stream.rows.insert(number, cells.clone()),
                    None => stream.rows.remove(&number),
                };
                if before != cells {
                    changed.push(number);
                }
            }
            if whole {
                given.sort_unstable();
                let stream = self
                    .streams
                    .get_mut(&key)
                    .ok_or_else(|| invalid("A yard row of no stream"))?;
                stream.rows.retain(|number, _| {
                    let kept = given.binary_search(number).is_ok();
                    if !kept {
                        changed.push(*number);
                    }
                    kept
                });
            }
            // A delta lists its rows by key, however the hour came to them.
            changed.sort_unstable();
            touched.push(key);
            self.write(writer, key, at, read, &changed, starts)?;
        }
        if whole || starts {
            touched.sort_unstable();
            let mut held: Vec<KeyHandle> = self
                .streams
                .iter()
                .filter(|(key, stream)| {
                    !stream.rows.is_empty() && touched.binary_search(key).is_err()
                })
                .map(|(&key, _)| key)
                .collect();
            held.sort_unstable();
            for key in held {
                let removed: Vec<u64> = match self.streams.get_mut(&key) {
                    Some(stream) if whole => std::mem::take(&mut stream.rows).into_keys().collect(),
                    Some(_) => Vec::new(),
                    None => continue,
                };
                self.write(writer, key, at, read, &removed, starts)?;
            }
        }
        Ok(())
    }

    /// Writes the event of an hour that changed the rows `changed` of the stream `key`: a delta
    /// of them, or a keyframe where the chain needs one or where coverage `starts` on rows held.
    fn write(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        read: Option<i32>,
        changed: &[u64],
        starts: bool,
    ) -> io::Result<()> {
        let stream = self
            .streams
            .get_mut(&key)
            .ok_or_else(|| invalid("A yard event of no stream"))?;
        if changed.is_empty() && !(starts && !stream.rows.is_empty()) {
            return Ok(());
        }
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        let kind = match stream.chain {
            Some((first, events)) if events < layout::CHAIN => {
                cells.push(Cell::Integer(i128::from(first)));
                for &number in changed {
                    push_row(writer, &mut cells, number, stream.rows.get(&number), stream.width)?;
                }
                stream.chain = Some((first, events + 1));
                kind::DELTA
            }
            _ => {
                for (&number, row) in &stream.rows {
                    push_row(writer, &mut cells, number, Some(row), stream.width)?;
                }
                stream.chain = Some((at, 1));
                kind::KEYFRAME
            }
        };
        let fields = writer.cells(cells.drain(..))?;
        self.cells = cells;
        event(writer, key, at, read, kind, fields)
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some([observed, _]) = self.family {
            self.observed.cut(writer, observed, last)?;
        }
        for (key, coverage) in self.sources.iter_mut().flatten() {
            coverage.cut(writer, *key, last)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut: its walk, dates, coverage, tokens and every
    /// stream's rows and chain; never what it published.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut o = Out(out);
        o.hour(self.last);
        o.hour(self.streak);
        o.hour(self.dated);
        o.u8(u8::from(self.walking));
        o.hour(self.next);
        o.var(self.dates.len() as u64);
        for (&table, &read) in &self.dates {
            o.var(table as u64);
            o.zig(i64::from(read));
        }
        o.coverage(self.observed);
        for source in &self.sources {
            match source {
                Some((key, coverage)) => {
                    o.u8(1);
                    o.var(*key);
                    o.coverage(*coverage);
                }
                None => o.u8(0),
            }
        }
        let mut tokens: Vec<(u64, u128)> = self
            .tokens
            .iter()
            .map(|(&digest, &number)| (number, digest))
            .collect();
        tokens.sort_unstable();
        o.var(tokens.len() as u64);
        for (_, digest) in tokens {
            o.0.extend_from_slice(&digest.to_le_bytes());
        }
        let mut streams: Vec<(&KeyHandle, &Stream)> = self.streams.iter().collect();
        streams.sort_unstable_by_key(|(key, _)| **key);
        o.var(streams.len() as u64);
        for (&key, stream) in streams {
            o.var(key);
            o.var(stream.width as u64);
            o.hour(stream.chain.map(|(first, _)| first));
            if let Some((_, events)) = stream.chain {
                o.u8(events);
            }
            o.var(stream.rows.len() as u64);
            for (&number, row) in &stream.rows {
                o.var(number);
                for held in row.iter() {
                    o.held(held);
                }
            }
        }
        self.carry.save(o.0)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(format: FormatVersion, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new(format);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.family = directory
            .key(FAMILY, layout::OBSERVED, None)
            .zip(directory.key(FAMILY, READS, None))
            .map(|(observed, reads)| [observed, reads]);
        let mut d = Decode::new(bytes);
        emit.last = hour(&mut d)?;
        emit.streak = hour(&mut d)?;
        emit.dated = hour(&mut d)?;
        emit.walking = d.flag()?;
        emit.next = hour(&mut d)?;
        for _ in 0..count(&mut d)? {
            let table = count(&mut d)?;
            emit.dates.insert(table, hour_of(&mut d)?);
        }
        emit.observed = coverage(&mut d)?;
        for source in &mut emit.sources {
            *source = match d.u8()? {
                0 => None,
                1 => Some((d.var()?, coverage(&mut d)?)),
                _ => return Err(damaged()),
            };
        }
        for number in 0..count(&mut d)? as u64 {
            emit.tokens.insert(u128::from_le_bytes(d.array()?), number);
        }
        for _ in 0..count(&mut d)? {
            let key = d.var()?;
            let width = count(&mut d)?;
            let chain = match hour(&mut d)? {
                None => None,
                Some(first) => {
                    let events = d.u8()?;
                    if events == 0 || events > layout::CHAIN {
                        return Err(damaged());
                    }
                    Some((first, events))
                }
            };
            let mut rows = BTreeMap::new();
            for _ in 0..count(&mut d)? {
                let number = d.var()?;
                let row = (2..width)
                    .map(|_| held(&mut d))
                    .collect::<io::Result<Box<[Held]>>>()?;
                rows.insert(number, row);
            }
            emit.streams.insert(key, Stream { width, rows, chain });
        }
        emit.carry = Carry::load(&bytes[d.position()..])?;
        Ok(emit)
    }
}

/// Appends the row `number` holds, or its removal, to an event's cells.
fn push_row(
    writer: &mut Writer<'_>,
    cells: &mut Vec<Cell>,
    number: u64,
    row: Option<&Box<[Held]>>,
    width: usize,
) -> io::Result<()> {
    engine_allocation::reserve(cells, width)?;
    cells.push(Cell::Unsigned(number));
    cells.push(Cell::Boolean(row.is_some()));
    match row {
        Some(row) => {
            for held in row.iter() {
                let cell = cell(writer, held)?;
                cells.push(cell);
            }
        }
        None => cells.extend(std::iter::repeat_n(Cell::Null, width - 2)),
    }
    Ok(())
}

fn cell(writer: &mut Writer<'_>, held: &Held) -> io::Result<Cell> {
    Ok(match held {
        Held::Null => Cell::Null,
        Held::Integer(value) => Cell::Integer(*value),
        Held::Unsigned(value) => Cell::Unsigned(*value),
        Held::List(list) => {
            let cells = list
                .iter()
                .map(|held| cell(writer, held))
                .collect::<io::Result<Vec<Cell>>>()?;
            Cell::List(writer.cells(cells)?)
        }
    })
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

/// The saved emit's encoding: varints, zigzag hours and values, flags.
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

    fn var128(&mut self, mut value: u128) {
        while value >= 0x80 {
            self.0.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.0.push(value as u8);
    }

    fn zig(&mut self, value: i64) {
        self.var(((value << 1) ^ (value >> 63)) as u64);
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

    fn coverage(&mut self, coverage: Coverage) {
        match coverage.open {
            None => self.u8(0),
            Some((Status::Known(read), from)) => {
                self.u8(1);
                self.hour(read);
                self.zig(i64::from(from));
            }
            Some((Status::Unknown(reason), from)) => {
                self.u8(2);
                self.var(reason);
                self.zig(i64::from(from));
            }
        }
    }

    fn held(&mut self, held: &Held) {
        match held {
            Held::Null => self.u8(0),
            Held::Integer(value) => {
                self.u8(1);
                self.var128(((value << 1) ^ (value >> 127)) as u128);
            }
            Held::Unsigned(value) => {
                self.u8(2);
                self.var(*value);
            }
            Held::List(list) => {
                self.u8(3);
                self.var(list.len() as u64);
                for held in list.iter() {
                    self.held(held);
                }
            }
        }
    }
}

fn damaged() -> io::Error {
    invalid("A saved yard emit is damaged")
}

fn count(d: &mut Decode<'_>) -> io::Result<usize> {
    usize::try_from(d.var()?).map_err(|_| damaged())
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

fn coverage(d: &mut Decode<'_>) -> io::Result<Coverage> {
    let open = match d.u8()? {
        0 => None,
        1 => {
            let read = hour(d)?;
            Some((Status::Known(read), hour_of(d)?))
        }
        2 => {
            let reason = d.var()?;
            Some((Status::Unknown(reason), hour_of(d)?))
        }
        _ => return Err(damaged()),
    };
    Ok(Coverage { open })
}

fn held(d: &mut Decode<'_>) -> io::Result<Held> {
    Ok(match d.u8()? {
        0 => Held::Null,
        1 => Held::Integer(d.zig128()?),
        2 => Held::Unsigned(d.var()?),
        3 => Held::List(
            (0..count(d)?)
                .map(|_| held(d))
                .collect::<io::Result<Box<[Held]>>>()?,
        ),
        _ => return Err(damaged()),
    })
}
