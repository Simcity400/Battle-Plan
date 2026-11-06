//! A segment's walk written as its strips' pages, hour by hour. An open page's bytes go to the
//! walk's [`Disk`] as they are written, in streams its strips are dealt among, so a walk holds
//! only where each open page stands and the cells its changes are taken against. Pages close
//! as they would held whole; at the segment's end each is put together from its stream, byte
//! for byte, and stored with what else the segment wrote.

use super::{Chunk, Disk, Layout, Location, Metrics, PAGE_BYTES, Pack, ROWS, Sample, Series, cell};
use crate::{
    invalid,
    leaf::{Reader, Writer, varint_len},
    manifest::{Manifest, Status},
    section::Changes,
    state::{Kind, Table},
    walk::State,
};
use engine_allocation::labels::{self, Kind as Label};
use std::io;

/// A fill of fewer of a block's cells is written cell by cell.
const FILL_MINIMUM: usize = 16;
/// Strings a dictionary chunk holds at most.
const CHUNK_STRINGS: usize = 4096;
/// The streams a walk's open pages' bytes go to disk in, its strips dealt among them in turn,
/// and the bytes a stream gathers before they go: all it holds, but for a larger record.
const STREAMS: usize = 256;
const STREAM_BYTES: usize = 32 << 10;

/// A page a segment's walk closed: its strip, its ordinals, and where its bytes lie among the
/// segment's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Closed {
    /// The table's place among the index's, its column ([`ROWS`] for its rows) and block.
    pub table: u32,
    pub column: u32,
    pub block: u32,
    pub first: u32,
    pub last: u32,
    pub location: Location,
    /// For a block's rows, the least and the most key the page names.
    pub keys: (u128, u128),
}

/// What a segment's walk wrote of its run's ordinals: its directory. The bytes its locations
/// name are the segment's, which its [`Disk`] stored.
#[derive(Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Piece {
    pub run: u32,
    pub first: u32,
    pub ordinals: u32,
    pub hours: Vec<i32>,
    pub metrics: Vec<Metrics>,
    /// Each ordinal's read bits, of every schema table, [`Self::words`] words each.
    pub read: Vec<u64>,
    pub words: usize,
    /// Per indexed table.
    pub series: Vec<Series>,
    pub pages: Vec<Closed>,
    /// The run's id of the first string it appended, the strings it appended, and the chunks
    /// holding them.
    pub strings_first: u32,
    pub strings: u32,
    pub chunks: Vec<Chunk>,
}

/// A strip's open page: its ordinals, its keys, the cells its changes are taken against, and
/// the raw bytes its cells at its first ordinal, its changes' headers and their values take.
struct Open {
    first: u32,
    last: u32,
    runs: u32,
    keys: Option<(u128, u128)>,
    held: cell::Held,
    base: usize,
    head: usize,
    values: usize,
}

impl Open {
    fn new(first: u32, base: usize) -> Self {
        Self {
            first,
            last: first,
            runs: 0,
            keys: None,
            held: cell::Held::default(),
            base,
            head: 0,
            values: 0,
        }
    }

    /// The raw bytes its page holds so far.
    fn bytes(&self) -> usize {
        self.base + self.head + self.values
    }

    /// Its page's raw length: its cells, its headers' length, the headers and their values.
    fn raw(&self) -> usize {
        self.base + varint_len(self.head as u64) + self.head + self.values
    }

    fn key(&mut self, key: u128) {
        self.keys = Some(match self.keys {
            Some((low, high)) => (low.min(key), high.max(key)),
            None => (key, key),
        });
    }
}

/// A strip in a walk: the id its bytes go to disk under, from its first page, and its page
/// open now.
#[derive(Default)]
struct Strip {
    id: Option<u32>,
    open: Option<Box<Open>>,
}

/// A stream of open pages' bytes: those it gathered, and where those gone to disk lie.
#[derive(Default)]
struct Stream {
    gathered: Vec<u8>,
    kept: Vec<(u64, u32)>,
    bytes: usize,
}

impl Stream {
    /// Sends what it gathered to `disk`, keeping no more room than it gathers.
    fn spill(&mut self, disk: &mut dyn Disk) -> io::Result<()> {
        if self.gathered.is_empty() {
            return Ok(());
        }
        let length = u32::try_from(self.gathered.len())
            .map_err(|_| invalid("A stream's record passes 4 GiB"))?;
        let at = disk.keep(&self.gathered)?;
        // Bytes that follow its last on disk are read back with them, in one read.
        let joined = self
            .kept
            .last_mut()
            .filter(|(start, held)| *start + u64::from(*held) == at)
            .and_then(|(_, held)| Some((held.checked_add(length)?, held)));
        if let Some((joined, held)) = joined {
            *held = joined;
        } else {
            engine_allocation::reserve(&mut self.kept, 1)?;
            self.kept.push((at, length));
        }
        self.gathered.clear();
        if self.gathered.capacity() > STREAM_BYTES {
            self.gathered = Vec::new();
        }
        Ok(())
    }
}

/// Buffers an hour's strips are written with.
#[derive(Default)]
struct Scratch {
    slots: Vec<u32>,
    changed: Vec<u32>,
    explicit: Vec<u32>,
    filled: Vec<u32>,
    written: Vec<u32>,
    except: Vec<u32>,
    overrides: Vec<u32>,
    group: cell::Group,
    /// A page's cells at its first ordinal, and one hour's headers and values of a strip.
    base: Writer,
    head: Writer,
    values: Writer,
}

/// One segment's pages of the indexed tables, from the walker's state after each hour.
pub struct Builder {
    layout: Layout,
    tables: Vec<usize>,
    kinds: Vec<Vec<Kind>>,
    pack: Box<dyn Pack>,
    strings: u32,
    piece: Option<Piece>,
    /// Each table's strips by column, its rows last, then by block.
    strips: Vec<Vec<Vec<Strip>>>,
    /// Each strip id's table, column and block.
    named: Vec<(u32, u32, u32)>,
    /// The pages closed, each after its strip's id, their bytes still in the streams.
    closed: Vec<(u32, Closed)>,
    streams: Vec<Stream>,
    /// The strings appended since the last chunk was stored, end to end.
    pending: Vec<u8>,
    ends: Vec<u32>,
    /// Each table's truncated view written last.
    views: Vec<Option<Vec<u32>>>,
    scratch: Scratch,
    /// A page's raw and stored bytes.
    raw: Vec<u8>,
    stored: Vec<u8>,
}

/// Keeps `value` from `ordinal` on when it differs from the last kept.
fn note<T: Copy + PartialEq>(series: &mut Vec<(u32, T)>, ordinal: u32, value: T) -> io::Result<()> {
    if series.last().is_none_or(|&(_, held)| held != value) {
        engine_allocation::reserve(series, 1)?;
        series.push((ordinal, value));
    }
    Ok(())
}

/// Packs `raw` and stores it on `disk`, where it lies.
fn store(
    disk: &mut dyn Disk,
    pack: &mut dyn Pack,
    stored: &mut Vec<u8>,
    raw: &[u8],
) -> io::Result<Location> {
    pack.pack(raw, stored)?;
    let raw = u32::try_from(raw.len()).map_err(|_| invalid("A page passes 4 GiB"))?;
    disk.put(stored, raw)
}

/// Empties `writer`, keeping its allocation.
fn reset(writer: &mut Writer) {
    *writer = Writer::reusing(std::mem::take(writer).into_bytes());
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) -> io::Result<()> {
    engine_allocation::reserve(out, varint_len(value))?;
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    Ok(())
}

/// Stores the strings appended since the last chunk as one chunk of `piece`'s.
#[allow(clippy::too_many_arguments)]
fn store_strings(
    piece: &mut Piece,
    pending: &mut Vec<u8>,
    ends: &mut Vec<u32>,
    disk: &mut dyn Disk,
    pack: &mut dyn Pack,
    raw: &mut Vec<u8>,
    stored: &mut Vec<u8>,
) -> io::Result<()> {
    if ends.is_empty() {
        return Ok(());
    }
    let count = u32::try_from(ends.len()).map_err(|_| invalid("Dictionary ids exceed 32 bits"))?;
    let mut writer = Writer::reusing(std::mem::take(raw));
    let wrote = (|| -> io::Result<()> {
        writer.varint(u64::from(count))?;
        let mut start = 0;
        for &end in ends.iter() {
            let end = end as usize;
            writer.varint((end - start) as u64)?;
            writer.raw(&pending[start..end])?;
            start = end;
        }
        Ok(())
    })();
    let location = wrote.and_then(|()| {
        let _memory = labels::kind(Label::IndexSpill);
        store(disk, pack, stored, writer.bytes())
    });
    *raw = writer.into_bytes();
    let location = location?;
    let first = piece
        .strings_first
        .checked_add(piece.strings)
        .ok_or_else(|| invalid("Dictionary ids exceed 32 bits"))?;
    {
        let _memory = labels::kind(Label::IndexPieces);
        engine_allocation::reserve(&mut piece.chunks, 1)?;
    }
    piece.chunks.push(Chunk {
        first,
        count,
        location,
    });
    piece.strings = piece
        .strings
        .checked_add(count)
        .ok_or_else(|| invalid("Dictionary ids exceed 32 bits"))?;
    pending.clear();
    ends.clear();
    Ok(())
}

/// The ascending union of ascending `left` and `right`, each slot once, into `into`.
fn union(left: &[u32], right: &[u32], into: &mut Vec<u32>) -> io::Result<()> {
    into.clear();
    engine_allocation::reserve(into, left.len() + right.len())?;
    let (mut left, mut right) = (left.iter().peekable(), right.iter().peekable());
    loop {
        let slot = match (left.peek(), right.peek()) {
            (Some(&&a), Some(&&b)) if a < b => left.next(),
            (Some(&&a), Some(&&b)) if b < a => right.next(),
            (Some(_), Some(_)) => {
                right.next();
                left.next()
            }
            (Some(_), None) => left.next(),
            (None, Some(_)) => right.next(),
            (None, None) => break,
        };
        if let Some(&slot) = slot {
            into.push(slot);
        }
    }
    Ok(())
}

/// The slots of ascending `slots` in `block` of `block_slots` slots, after those before it.
fn in_block(slots: &[u32], block: u32, block_slots: u32) -> &[u32] {
    let end = u64::from(block + 1) * u64::from(block_slots);
    &slots[..slots.partition_point(|&slot| u64::from(slot) < end)]
}

impl Builder {
    /// A segment's pages of `tables`, schema tables ascending, of each one's `kinds`, laid out
    /// as `layout` and stored as `pack` packs them.
    pub fn new(
        layout: Layout,
        tables: &[usize],
        kinds: Vec<Vec<Kind>>,
        pack: Box<dyn Pack>,
    ) -> io::Result<Self> {
        if tables.windows(2).any(|pair| pair[0] >= pair[1]) || kinds.len() != tables.len() {
            return Err(invalid("A run index's tables are not its kinds' in order"));
        }
        let _memory = labels::kind(Label::IndexCursors);
        Ok(Self {
            layout: layout.check()?,
            tables: engine_allocation::copy_vec(tables)?,
            strips: engine_allocation::collect_vec(
                kinds
                    .iter()
                    .map(|kinds| (0..=kinds.len()).map(|_| Vec::new()).collect()),
            )?,
            named: Vec::new(),
            closed: Vec::new(),
            streams: engine_allocation::default_vec(STREAMS)?,
            pending: Vec::new(),
            ends: Vec::new(),
            views: engine_allocation::collect_vec(tables.iter().map(|_| None))?,
            kinds,
            pack,
            strings: 0,
            piece: None,
            scratch: Scratch::default(),
            raw: Vec::new(),
            stored: Vec::new(),
        })
    }

    /// The walk's start: its run's dictionary holds `strings` strings before its first hour.
    pub fn start(&mut self, strings: usize) -> io::Result<()> {
        if self.piece.is_some() {
            return Err(invalid("A run index's segment starts after its first hour"));
        }
        self.strings =
            u32::try_from(strings).map_err(|_| invalid("Dictionary ids exceed 32 bits"))?;
        Ok(())
    }

    /// Ordinal `ordinal` of run `run`, once `state` applied its `manifest`, its bytes written
    /// to `disk`.
    pub fn hour(
        &mut self,
        run: u32,
        ordinal: u32,
        state: &State,
        manifest: &Manifest<'_>,
        disk: &mut dyn Disk,
    ) -> io::Result<()> {
        if self
            .piece
            .as_ref()
            .is_some_and(|piece| piece.run != run || piece.first + piece.ordinals != ordinal)
        {
            return Err(invalid("A segment's hours do not follow one another"));
        }
        if self.piece.is_none() {
            let _memory = labels::kind(Label::IndexPieces);
            self.piece = Some(Piece {
                run,
                first: ordinal,
                words: manifest.read.len(),
                series: engine_allocation::collect_vec(
                    self.tables.iter().map(|_| Series::default()),
                )?,
                strings_first: self.strings,
                ..Piece::default()
            });
        }
        let Self {
            layout,
            tables,
            kinds,
            pack,
            piece,
            strips,
            named,
            closed,
            streams,
            pending,
            ends,
            views,
            scratch,
            raw,
            stored,
            ..
        } = self;
        let Some(piece) = piece.as_mut() else {
            return Err(invalid("A segment's piece is not held"));
        };
        if manifest.read.len() != piece.words {
            return Err(invalid("A manifest's read bits disagree with the run's"));
        }
        {
            let _memory = labels::kind(Label::IndexPieces);
            engine_allocation::reserve(&mut piece.hours, 1)?;
            piece.hours.push(manifest.hour);
            engine_allocation::reserve(&mut piece.metrics, 1)?;
            piece.metrics.push(Metrics {
                total_ns: manifest.total_ns,
                pool_bytes: manifest.pool_bytes,
                dropped_frames: manifest.dropped_frames,
            });
            engine_allocation::reserve(&mut piece.read, manifest.read.len())?;
            piece.read.extend_from_slice(&manifest.read);
        }
        for append in &manifest.appends {
            {
                let _memory = labels::kind(Label::IndexSpill);
                engine_allocation::reserve(pending, append.len())?;
                pending.extend_from_slice(append);
                engine_allocation::reserve(ends, 1)?;
                ends.push(
                    u32::try_from(pending.len())
                        .map_err(|_| invalid("A segment's strings pass 4 GiB"))?,
                );
            }
            if ends.len() >= CHUNK_STRINGS {
                store_strings(piece, pending, ends, &mut *disk, &mut **pack, raw, stored)?;
            }
        }
        let mut pages = Pages {
            layout: *layout,
            disk: &mut *disk,
            named,
            closed,
            streams,
        };
        for (at, &table) in tables.iter().enumerate() {
            let missing = || invalid("The walker holds no such table");
            let series = &mut piece.series[at];
            // A withheld table holds no rows and no sample, but every series a cursor reads;
            // its readers learn why from the index's summary.
            if state.withheld(table).is_some() {
                let _memory = labels::kind(Label::IndexPieces);
                note(&mut series.statuses, ordinal, Status::NotRecorded)?;
                note(&mut series.samples, ordinal, Sample::At(None))?;
                note(&mut series.slots, ordinal, 0)?;
                continue;
            }
            if state.skips(table) {
                return Err(invalid("A walk skips a table its run index holds"));
            }
            let held = state.table(table).ok_or_else(missing)?;
            {
                let _memory = labels::kind(Label::IndexPieces);
                note(
                    &mut series.statuses,
                    ordinal,
                    state.status(table).ok_or_else(missing)?,
                )?;
                let sampled = state.sampled_at(table);
                if series
                    .samples
                    .last()
                    .map(|&(_, sample)| sample.at(manifest.hour))
                    != Some(sampled)
                {
                    let sample = match sampled {
                        Some(hour) if hour == manifest.hour => Sample::Each,
                        _ => Sample::At(sampled),
                    };
                    engine_allocation::reserve(&mut series.samples, 1)?;
                    series.samples.push((ordinal, sample));
                }
                let slots = u32::try_from(held.slots())
                    .map_err(|_| invalid("A table passes the last slot"))?;
                note(&mut series.slots, ordinal, slots)?;
            }
            if let Some(view) = state.truncated_view(table)
                && views[at].as_deref() != Some(view)
            {
                let mut writer = Writer::new();
                writer.slots(view)?;
                let location = {
                    let _memory = labels::kind(Label::IndexSpill);
                    store(&mut *pages.disk, &mut **pack, stored, writer.bytes())?
                };
                let _memory = labels::kind(Label::IndexPieces);
                engine_allocation::reserve(&mut series.views, 1)?;
                series.views.push((ordinal, location));
                views[at] = Some(engine_allocation::copy_vec(view)?);
            }
            let changes = state.changes(table).ok_or_else(missing)?;
            if changes.changed.is_empty()
                && changes.deleted.is_empty()
                && changes.inserted.is_empty()
                && changes.fill_slots.is_empty()
            {
                continue;
            }
            if changes.written.len() != changes.changed.len()
                || changes.fill_masks.len() != changes.fill_slots.len()
            {
                return Err(invalid("A table's change masks miss its changed slots"));
            }
            let _memory = labels::kind(Label::IndexCursors);
            let strips = &mut strips[at];
            let columns = kinds[at].len();
            let (values, rows) = strips.split_at_mut(columns);
            union(&changes.deleted, &changes.inserted, &mut scratch.slots)?;
            let slots = std::mem::take(&mut scratch.slots);
            let wrote = pages.rows(at as u32, held, ordinal, &slots, &mut rows[0], scratch);
            scratch.slots = slots;
            wrote?;
            for (column, strips) in values.iter_mut().enumerate() {
                let kind = kinds[at][column];
                pages.column(
                    at as u32, column, kind, held, ordinal, changes, strips, scratch,
                )?;
            }
        }
        piece.ordinals += 1;
        Ok(())
    }

    /// Closes every open page and stores every page the segment wrote, and its strings, on
    /// `disk`: what the segment wrote. The builder then starts anew.
    pub fn finish(&mut self, disk: &mut dyn Disk) -> io::Result<Piece> {
        let mut piece = self
            .piece
            .take()
            .ok_or_else(|| invalid("A segment's walk wrote no hour"))?;
        let Self {
            layout,
            pack,
            strips,
            named,
            closed,
            streams,
            pending,
            ends,
            views,
            raw,
            stored,
            ..
        } = self;
        let mut pages = Pages {
            layout: *layout,
            disk: &mut *disk,
            named: &mut *named,
            closed: &mut *closed,
            streams: &mut streams[..],
        };
        for (at, strips) in strips.iter_mut().enumerate() {
            let columns = strips.len() - 1;
            for (column, blocks) in strips.iter_mut().enumerate() {
                let column = if column == columns {
                    ROWS
                } else {
                    column as u32
                };
                for (block, strip) in blocks.iter_mut().enumerate() {
                    if let Some(open) = strip.open.take() {
                        let id = strip
                            .id
                            .ok_or_else(|| invalid("An open page's strip holds no id"))?;
                        pages.close(at as u32, column, block as u32, id, open)?;
                    }
                    strip.id = None;
                }
            }
        }
        pages.land(&mut **pack, raw, stored)?;
        store_strings(&mut piece, pending, ends, disk, &mut **pack, raw, stored)?;
        {
            let _memory = labels::kind(Label::IndexPieces);
            piece.pages = engine_allocation::collect_vec(closed.drain(..).map(|(_, page)| page))?;
        }
        named.clear();
        for view in views.iter_mut() {
            *view = None;
        }
        Ok(piece)
    }
}

/// A record a stream kept: its strip's id, and a page's cells at its first ordinal or one
/// hour's headers and values; where the next starts.
enum Kept<'b> {
    Base(&'b [u8]),
    Group(&'b [u8], &'b [u8]),
}

fn kept_at(bytes: &[u8], at: usize) -> io::Result<(u32, Kept<'_>, usize)> {
    let mut reader = Reader::new(
        bytes
            .get(at..)
            .ok_or_else(|| invalid("A stream's record lies past its bytes"))?,
    );
    let tag = reader.varint()?;
    let id = u32::try_from(tag >> 1).map_err(|_| invalid("A stream names no strip"))?;
    let kept = if tag & 1 != 0 {
        let length = reader.count()?;
        Kept::Base(reader.raw(length)?)
    } else {
        let (head, values) = (reader.count()?, reader.count()?);
        Kept::Group(reader.raw(head)?, reader.raw(values)?)
    };
    Ok((id, kept, at + reader.at()))
}

/// Where a segment's pages go.
struct Pages<'a> {
    layout: Layout,
    disk: &'a mut dyn Disk,
    named: &'a mut Vec<(u32, u32, u32)>,
    closed: &'a mut Vec<(u32, Closed)>,
    streams: &'a mut [Stream],
}

impl Pages<'_> {
    /// The id the strip of `table`'s `column` at `block` keeps its bytes under.
    fn id(&mut self, strip: &mut Strip, table: u32, column: u32, block: u32) -> io::Result<u32> {
        if let Some(id) = strip.id {
            return Ok(id);
        }
        let id = u32::try_from(self.named.len()).map_err(|_| invalid("Strips pass 2^32"))?;
        engine_allocation::reserve(self.named, 1)?;
        self.named.push((table, column, block));
        strip.id = Some(id);
        Ok(id)
    }

    /// Adds a record of strip `id`, tagged `base` for a page's cells at its first ordinal, of
    /// `parts`, each its length first but for the last of a base.
    fn keep(&mut self, id: u32, base: bool, parts: &[&[u8]]) -> io::Result<()> {
        let _memory = labels::kind(Label::IndexSpill);
        let tag = u64::from(id) << 1 | u64::from(base);
        let length = parts.iter().fold(varint_len(tag), |length, part| {
            length + varint_len(part.len() as u64) + part.len()
        });
        let stream = &mut self.streams[id as usize % STREAMS];
        if stream.gathered.len() + length > STREAM_BYTES {
            stream.spill(&mut *self.disk)?;
        }
        if stream.gathered.capacity() - stream.gathered.len() < length {
            let room = length.max(STREAM_BYTES) - stream.gathered.len();
            engine_allocation::reserve_exact(&mut stream.gathered, room)?;
        }
        put_varint(&mut stream.gathered, tag)?;
        for part in parts {
            put_varint(&mut stream.gathered, part.len() as u64)?;
        }
        for part in parts {
            stream.gathered.extend_from_slice(part);
        }
        stream.bytes += length;
        if stream.gathered.len() >= STREAM_BYTES {
            stream.spill(&mut *self.disk)?;
        }
        Ok(())
    }

    fn close(
        &mut self,
        table: u32,
        column: u32,
        block: u32,
        id: u32,
        open: Box<Open>,
    ) -> io::Result<()> {
        let raw = open.raw();
        if raw > PAGE_BYTES as usize {
            let what = match column {
                ROWS => "rows".to_owned(),
                column => format!("column {column}"),
            };
            return Err(invalid(format!(
                "Indexed table {table}'s {what} of block {block} pass a change-run page's bytes"
            )));
        }
        let _memory = labels::kind(Label::IndexPieces);
        engine_allocation::reserve(self.closed, 1)?;
        self.closed.push((
            id,
            Closed {
                table,
                column,
                block,
                first: open.first,
                last: open.last,
                location: Location {
                    offset: 0,
                    stored: 0,
                    raw: raw as u32,
                },
                keys: open.keys.unwrap_or_default(),
            },
        ));
        Ok(())
    }

    /// Puts every closed page together from its stream, stream by stream, and stores it.
    fn land(
        &mut self,
        pack: &mut dyn Pack,
        raw: &mut Vec<u8>,
        stored: &mut Vec<u8>,
    ) -> io::Result<()> {
        // Each strip's pages in order, and where each strip's begin among them.
        self.closed.sort_unstable_by_key(|&(id, page)| (id, page.first));
        let starts = {
            let _memory = labels::kind(Label::IndexPieces);
            let mut starts: Vec<u32> = engine_allocation::filled(self.named.len() + 1, 0)?;
            for &(id, _) in self.closed.iter() {
                starts[id as usize + 1] += 1;
            }
            for at in 1..starts.len() {
                starts[at] += starts[at - 1];
            }
            starts
        };
        let _memory = labels::kind(Label::IndexSpill);
        // Every stream's gathered bytes go first: what is read back is one stream's at a time.
        for stream in self.streams.iter_mut() {
            stream.spill(&mut *self.disk)?;
            stream.gathered = Vec::new();
        }
        let (mut bytes, mut records) = (Vec::new(), Vec::new());
        let mut landed = 0;
        for index in 0..self.streams.len() {
            let stream = std::mem::take(&mut self.streams[index]);
            if stream.bytes == 0 {
                continue;
            }
            bytes.clear();
            engine_allocation::reserve(&mut bytes, stream.bytes)?;
            for &(at, length) in &stream.kept {
                let start = bytes.len();
                engine_allocation::resize(&mut bytes, start + length as usize, 0)?;
                self.disk.kept(at, &mut bytes[start..])?;
            }
            bytes.extend_from_slice(&stream.gathered);
            drop(stream);
            if u32::try_from(bytes.len()).is_err() {
                return Err(invalid("A walk's stream passes 4 GiB"));
            }
            let bytes: &[u8] = &bytes;
            records.clear();
            let mut next = 0;
            while next < bytes.len() {
                let (id, kept, end) = kept_at(bytes, next)?;
                engine_allocation::reserve(&mut records, 1)?;
                records.push((id, next as u32, matches!(kept, Kept::Base(_))));
                next = end;
            }
            // By strip, each strip's in the order they were kept.
            records.sort_unstable();
            for strip in records.chunk_by(|a, b| a.0 == b.0) {
                let id = strip[0].0 as usize;
                let (from, to) = match (starts.get(id), starts.get(id + 1)) {
                    (Some(&from), Some(&to)) => (from as usize, to as usize),
                    _ => return Err(invalid("A stream names a strip the walk did not open")),
                };
                let mut page = from;
                let mut records = strip;
                while let Some((&(_, first, _), rest)) = records.split_first() {
                    let (_, Kept::Base(base), _) = kept_at(bytes, first as usize)? else {
                        return Err(invalid("A strip's changes come before its page"));
                    };
                    let length = rest
                        .iter()
                        .position(|&(_, _, base)| base)
                        .unwrap_or(rest.len());
                    let (groups, after) = rest.split_at(length);
                    records = after;
                    if page == to {
                        return Err(invalid("A strip's stream holds more pages than it closed"));
                    }
                    let closed = &mut self.closed[page].1;
                    page += 1;
                    let group = |at: u32| match kept_at(bytes, at as usize) {
                        Ok((_, Kept::Group(head, values), _)) => Ok((head, values)),
                        Ok(_) => Err(invalid("A strip's page holds a second page's cells")),
                        Err(error) => Err(error),
                    };
                    let mut head = 0;
                    for &(_, at, _) in groups {
                        head += group(at)?.0.len();
                    }
                    let mut writer = Writer::reusing(std::mem::take(raw));
                    let assembled = (|| -> io::Result<()> {
                        writer.raw(base)?;
                        writer.varint(head as u64)?;
                        for &(_, at, _) in groups {
                            writer.raw(group(at)?.0)?;
                        }
                        for &(_, at, _) in groups {
                            writer.raw(group(at)?.1)?;
                        }
                        if writer.bytes().len() != closed.location.raw as usize {
                            return Err(invalid("A page put together differs from its length"));
                        }
                        Ok(())
                    })();
                    let location = assembled
                        .and_then(|()| store(&mut *self.disk, pack, stored, writer.bytes()));
                    *raw = writer.into_bytes();
                    closed.location = location?;
                    landed += 1;
                }
                if page != to {
                    return Err(invalid("A strip's stream holds fewer pages than it closed"));
                }
            }
        }
        if landed != self.closed.len() {
            return Err(invalid("A segment's closed pages are not all in its streams"));
        }
        Ok(())
    }

    /// The first and the extent of `block`'s slots in `held`.
    fn extent(&self, held: &Table, block: u32) -> (u32, u32) {
        let start = block * self.layout.block;
        let extent = (held.slots() as u64)
            .saturating_sub(u64::from(start))
            .min(u64::from(self.layout.block)) as u32;
        (start, extent)
    }

    /// `block`'s strip in `strips`, which it grows to hold it.
    fn strip(strips: &mut Vec<Strip>, block: u32) -> io::Result<&mut Strip> {
        let block = block as usize;
        if strips.len() <= block {
            engine_allocation::resize_default(strips, block + 1)?;
        }
        Ok(&mut strips[block])
    }

    /// Closes `strip`'s page once it holds its share of changes, or of bytes.
    fn full(&mut self, table: u32, column: u32, block: u32, strip: &mut Strip) -> io::Result<()> {
        if strip.open.as_ref().is_some_and(|open| {
            open.runs >= self.layout.runs || open.bytes() >= PAGE_BYTES as usize / 4
        }) {
            let id = strip
                .id
                .ok_or_else(|| invalid("An open page's strip holds no id"))?;
            if let Some(open) = strip.open.take() {
                self.close(table, column, block, id, open)?;
            }
        }
        Ok(())
    }

    /// The rows the hour deleted or inserted, `slots` ascending, in their blocks' strips.
    fn rows(
        &mut self,
        table: u32,
        held: &Table,
        ordinal: u32,
        mut slots: &[u32],
        strips: &mut Vec<Strip>,
        scratch: &mut Scratch,
    ) -> io::Result<()> {
        let Scratch {
            group,
            base,
            head,
            values,
            ..
        } = scratch;
        while let Some(&slot) = slots.first() {
            let block = slot / self.layout.block;
            let these = in_block(slots, block, self.layout.block);
            slots = &slots[these.len()..];
            let (start, extent) = self.extent(held, block);
            let strip = Self::strip(strips, block)?;
            let id = self.id(strip, table, ROWS, block)?;
            if let Some(open) = strip.open.as_deref_mut() {
                group.start();
                reset(values);
                for &slot in these {
                    let key = held.key(slot);
                    group.row(values, slot - start, key)?;
                    if let Some(key) = key {
                        open.key(key);
                    }
                }
                reset(head);
                let written = group.close(head, open.last, ordinal)?;
                if written > 0 {
                    open.last = ordinal;
                    open.runs += written;
                    open.head += head.bytes().len();
                    open.values += values.bytes().len();
                    self.keep(id, false, &[head.bytes(), values.bytes()])?;
                }
            } else {
                reset(base);
                let keys = cell::put_rows(base, held, start, extent)?;
                self.keep(id, true, &[base.bytes()])?;
                let mut open = Open::new(ordinal, base.bytes().len());
                open.keys = keys;
                strip.open = Some(engine_allocation::boxed(open)?);
            }
            self.full(table, ROWS, block, strip)?;
        }
        Ok(())
    }

    /// The cells of `column` the hour set, in their blocks' strips.
    #[allow(clippy::too_many_arguments)]
    fn column(
        &mut self,
        table: u32,
        column: usize,
        kind: Kind,
        held: &Table,
        ordinal: u32,
        changes: &Changes,
        strips: &mut Vec<Strip>,
        scratch: &mut Scratch,
    ) -> io::Result<()> {
        // Masks hold the first 64 columns; the others count as written.
        let bit = u32::try_from(column)
            .ok()
            .and_then(|column| 1_u64.checked_shl(column));
        let wrote = |mask: u64| bit.is_none_or(|bit| mask & bit != 0);
        let Scratch {
            changed,
            explicit,
            filled,
            written,
            except,
            overrides,
            group,
            base,
            head,
            values,
            ..
        } = scratch;
        changed.clear();
        engine_allocation::reserve(changed, changes.changed.len())?;
        changed.extend(
            changes
                .changed
                .iter()
                .zip(&changes.written)
                .filter(|&(_, &mask)| wrote(mask))
                .map(|(&slot, _)| slot),
        );
        union(&changes.inserted, changed, explicit)?;
        filled.clear();
        if changes.filled.contains(&column) {
            engine_allocation::reserve(filled, changes.fill_slots.len())?;
            filled.extend(
                changes
                    .fill_slots
                    .iter()
                    .zip(&changes.fill_masks)
                    .filter(|&(_, &mask)| wrote(mask))
                    .map(|(&slot, _)| slot),
            );
        }
        let (mut explicit, mut filled) = (&explicit[..], &filled[..]);
        while let Some(&slot) = match (explicit.first(), filled.first()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        } {
            let block = slot / self.layout.block;
            let (these, fills) = (
                in_block(explicit, block, self.layout.block),
                in_block(filled, block, self.layout.block),
            );
            (explicit, filled) = (&explicit[these.len()..], &filled[fills.len()..]);
            let (start, extent) = self.extent(held, block);
            let strip = Self::strip(strips, block)?;
            let id = self.id(strip, table, column as u32, block)?;
            if strip.open.is_none() {
                reset(base);
                let mut cells = cell::Held::default();
                cell::put_base(base, held, column, kind, start, extent, &mut cells)?;
                self.keep(id, true, &[base.bytes()])?;
                let mut open = Open::new(ordinal, base.bytes().len());
                open.held = cells;
                strip.open = Some(engine_allocation::boxed(open)?);
                continue;
            }
            let Some(open) = strip.open.as_deref_mut() else {
                continue;
            };
            union(these, fills, written)?;
            let fill = if fills.len() >= FILL_MINIMUM {
                fill(
                    held,
                    column,
                    start,
                    start + extent,
                    written,
                    fills,
                    except,
                    overrides,
                )?
            } else {
                None
            };
            group.start();
            reset(values);
            let cells = match fill {
                Some(value) => {
                    group.fill(values, except, value, extent, &mut open.held)?;
                    &overrides[..]
                }
                None => &written[..],
            };
            for &slot in cells {
                group.change(values, held, column, slot, slot - start, &mut open.held)?;
            }
            reset(head);
            let count = group.close(head, open.last, ordinal)?;
            if count > 0 {
                open.last = ordinal;
                open.runs += count;
                open.head += head.bytes().len();
                open.values += values.bytes().len();
                self.keep(id, false, &[head.bytes(), values.bytes()])?;
            }
            self.full(table, column as u32, block, strip)?;
        }
        Ok(())
    }
}

/// The cell one fill can write in place of `written`'s cells of `column` in slots `start..end`,
/// when it takes less than half their changes: then `except` holds the offsets of the block's
/// live slots it must leave and `overrides` the slots whose cells differ from it.
#[allow(clippy::too_many_arguments)]
fn fill<'t>(
    held: &'t Table,
    column: usize,
    start: u32,
    end: u32,
    written: &[u32],
    fills: &[u32],
    except: &mut Vec<u32>,
    overrides: &mut Vec<u32>,
) -> io::Result<Option<crate::state::Cell<'t>>> {
    let values = held
        .columns
        .get(column)
        .ok_or_else(|| invalid("A strip names a column past its table"))?;
    let Some(cell) = fills
        .iter()
        .find(|&&slot| held.recorded(column, slot))
        .and_then(|&slot| values.cell(slot))
    else {
        return Ok(None);
    };
    except.clear();
    overrides.clear();
    let budget = written.len() / 2;
    let mut next = written.iter().copied().peekable();
    for slot in start..end {
        if next.next_if_eq(&slot).is_some() {
            if !held.recorded(column, slot) || values.cell(slot) != Some(cell) {
                engine_allocation::reserve(overrides, 1)?;
                overrides.push(slot);
            }
        } else if held.live().get(slot) {
            engine_allocation::reserve(except, 1)?;
            except.push(slot - start);
        }
        if 1 + except.len() + overrides.len() >= budget {
            return Ok(None);
        }
    }
    Ok(Some(cell))
}
