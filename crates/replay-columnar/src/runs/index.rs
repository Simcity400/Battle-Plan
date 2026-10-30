//! Pieces stitched in segment order: each run's ordinals, its tables' series and strips'
//! directories, and its dictionary's chunks, with the directory's own bytes.

use super::{
    Layout, Location, Metrics, Pack, Page, ROWS, Sample, Series, Store,
    build::{Closed, Piece},
};
use crate::{
    envelope::schema_digest,
    invalid,
    leaf::{Int, Reader, Writer},
    manifest::Status,
    state::Kind,
};
use std::io;

/// Strings a dictionary chunk holds at most.
const CHUNK_STRINGS: usize = 4096;
const MAGIC: &[u8; 8] = b"H4RUNIX1";

/// Strings `first..first + count` of a run's dictionary, stored as one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub first: u32,
    pub count: u32,
    pub location: Location,
}

/// A page of a block's rows, with the least and the most key it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowsPage {
    pub page: Page,
    pub low: u128,
    pub high: u128,
}

impl AsRef<Page> for Page {
    fn as_ref(&self) -> &Page {
        self
    }
}

impl AsRef<Page> for RowsPage {
    fn as_ref(&self) -> &Page {
        &self.page
    }
}

/// One indexed table's strips over a run: each column's pages by block, and its rows'.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strips {
    pub columns: Vec<Vec<Vec<Page>>>,
    pub rows: Vec<Vec<RowsPage>>,
}

/// One run of the recording as the index holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunIndex {
    pub branch: u64,
    pub first_hour: i32,
    /// Each ordinal's hour, metrics and read bits ([`Self::words`] words each).
    pub hours: Vec<i32>,
    pub metrics: Vec<Metrics>,
    pub read: Vec<u64>,
    pub words: usize,
    /// Per indexed table.
    pub series: Vec<Series>,
    pub strips: Vec<Strips>,
    pub chunks: Vec<Chunk>,
    /// Strings the run's pieces appended.
    pub strings: u32,
    /// Strings appended since the last chunk was stored, end to end.
    pending: Vec<u8>,
    ends: Vec<u32>,
}

impl RunIndex {
    /// Run (`branch`, `first_hour`) of no ordinals yet, of tables of `kinds`.
    fn new(branch: u64, first_hour: i32, kinds: &[Vec<Kind>]) -> io::Result<Self> {
        Ok(Self {
            branch,
            first_hour,
            series: engine_allocation::collect_vec(kinds.iter().map(|_| Series::default()))?,
            strips: engine_allocation::collect_vec(kinds.iter().map(|kinds| Strips {
                columns: kinds.iter().map(|_| Vec::new()).collect(),
                rows: Vec::new(),
            }))?,
            ..Self::default()
        })
    }

    /// The ordinal of `hour`, when the run holds it.
    pub fn ordinal(&self, hour: i32) -> Option<u32> {
        self.hours.binary_search(&hour).ok().map(|at| at as u32)
    }

    /// `ordinal`'s read bits.
    pub fn read_bits(&self, ordinal: u32) -> Option<&[u64]> {
        let start = (ordinal as usize).checked_mul(self.words)?;
        self.read.get(start..start + self.words)
    }

    /// Stores the strings appended since the last chunk as chunks, the last one short only
    /// when `all`.
    fn store_strings(
        &mut self,
        store: &mut dyn Store,
        pack: &mut dyn Pack,
        all: bool,
    ) -> io::Result<()> {
        let mut stored = Vec::new();
        let mut done = 0;
        while self.ends.len() - done >= CHUNK_STRINGS || (all && done < self.ends.len()) {
            let count = (self.ends.len() - done).min(CHUNK_STRINGS);
            let mut writer = Writer::new();
            writer.varint(count as u64)?;
            let mut start = done
                .checked_sub(1)
                .map_or(0, |previous| self.ends[previous] as usize);
            for &end in &self.ends[done..done + count] {
                let end = end as usize;
                writer.varint((end - start) as u64)?;
                writer.raw(&self.pending[start..end])?;
                start = end;
            }
            pack.pack(writer.bytes(), &mut stored)?;
            let raw = u32::try_from(writer.bytes().len())
                .map_err(|_| invalid("A dictionary chunk passes 4 GiB"))?;
            let location = store.put(&stored, raw)?;
            let first = self.strings - (self.ends.len() - done) as u32;
            engine_allocation::reserve(&mut self.chunks, 1)?;
            self.chunks.push(Chunk {
                first,
                count: count as u32,
                location,
            });
            done += count;
        }
        if done > 0 {
            let cut = self.ends[done - 1] as usize;
            self.pending.drain(..cut);
            self.ends.drain(..done);
            for end in &mut self.ends {
                *end -= cut as u32;
            }
        }
        Ok(())
    }
}

/// A recording's run index: the tables it holds, how they are laid out, and every run's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub layout: Layout,
    /// Schema tables, ascending, and each one's kinds.
    pub tables: Vec<usize>,
    pub kinds: Vec<Vec<Kind>>,
    pub runs: Vec<RunIndex>,
}

/// The piece's page bytes at `location`, stored again in `store`.
fn restore(piece: &[u8], location: Location, store: &mut dyn Store) -> io::Result<Location> {
    let start = usize::try_from(location.offset).map_err(|_| engine_allocation::oom())?;
    let bytes = piece
        .get(start..start + location.stored as usize)
        .ok_or_else(|| invalid("A piece's page lies past its bytes"))?;
    store.put(bytes, location.raw)
}

/// Takes in a piece's `series` after `held`'s: an entry equal to the one in force is dropped.
fn extend<T: Copy + PartialEq>(held: &mut Vec<(u32, T)>, series: &[(u32, T)]) -> io::Result<()> {
    engine_allocation::reserve(held, series.len())?;
    for &(ordinal, value) in series {
        if held.last().is_some_and(|&(last, _)| last >= ordinal) {
            return Err(invalid("A piece's series does not follow the run's"));
        }
        if held.last().is_none_or(|&(_, last)| last != value) {
            held.push((ordinal, value));
        }
    }
    Ok(())
}

impl Index {
    /// An index of `tables`, schema tables ascending, of each one's `kinds`, holding `runs`
    /// (branch, first hour) of no ordinals yet.
    pub fn new(
        layout: Layout,
        tables: &[usize],
        kinds: Vec<Vec<Kind>>,
        runs: &[(u64, i32)],
    ) -> io::Result<Self> {
        if tables.windows(2).any(|pair| pair[0] >= pair[1]) || kinds.len() != tables.len() {
            return Err(invalid("A run index's tables are not its kinds' in order"));
        }
        Ok(Self {
            layout: layout.check()?,
            tables: engine_allocation::copy_vec(tables)?,
            runs: runs
                .iter()
                .map(|&(branch, first_hour)| RunIndex::new(branch, first_hour, &kinds))
                .collect::<io::Result<_>>()?,
            kinds,
        })
    }

    /// `table`'s place among the indexed tables.
    pub fn table(&self, table: usize) -> Option<usize> {
        self.tables.binary_search(&table).ok()
    }

    /// The run of `branch` holding `hour`, and the hour's ordinal in it.
    pub fn find(&self, branch: u64, hour: i32) -> Option<(u32, u32)> {
        self.runs.iter().enumerate().find_map(|(run, held)| {
            (held.branch == branch)
                .then(|| held.ordinal(hour))
                .flatten()
                .map(|ordinal| (run as u32, ordinal))
        })
    }

    /// Takes in `piece`, the next of its run in segment order, its pages kept in `store`
    /// and its strings' chunks packed by `pack`.
    pub fn append(
        &mut self,
        piece: Piece,
        store: &mut dyn Store,
        pack: &mut dyn Pack,
    ) -> io::Result<()> {
        let tables = self.tables.len();
        let run = self
            .runs
            .get_mut(piece.run as usize)
            .ok_or_else(|| invalid("A piece names a run the index does not hold"))?;
        if piece.first as usize != run.hours.len()
            || piece.series.len() != tables
            || piece.hours.len() != piece.ordinals as usize
            || piece.metrics.len() != piece.ordinals as usize
            || piece.read.len() != piece.ordinals as usize * piece.words
        {
            return Err(invalid("A piece does not follow its run's ordinals"));
        }
        if run.hours.is_empty() {
            run.words = piece.words;
        } else if run.words != piece.words || run.hours.last() >= piece.hours.first() {
            return Err(invalid("A piece does not follow its run's hours"));
        }
        if piece.strings_first != run.strings {
            return Err(invalid("A piece's strings do not follow its run's"));
        }
        engine_allocation::reserve(&mut run.hours, piece.hours.len())?;
        run.hours.extend_from_slice(&piece.hours);
        engine_allocation::reserve(&mut run.metrics, piece.metrics.len())?;
        run.metrics.extend_from_slice(&piece.metrics);
        engine_allocation::reserve(&mut run.read, piece.read.len())?;
        run.read.extend_from_slice(&piece.read);
        for (held, series) in run.series.iter_mut().zip(&piece.series) {
            extend(&mut held.statuses, &series.statuses)?;
            extend(&mut held.samples, &series.samples)?;
            extend(&mut held.slots, &series.slots)?;
            engine_allocation::reserve(&mut held.views, series.views.len())?;
            for &(ordinal, location) in &series.views {
                held.views
                    .push((ordinal, restore(&piece.bytes, location, store)?));
            }
        }
        for closed in &piece.pages {
            let Closed {
                table,
                column,
                block,
                first,
                last,
                location,
                keys: (low, high),
            } = *closed;
            let page = Page {
                first,
                last,
                location: restore(&piece.bytes, location, store)?,
            };
            let strips = run
                .strips
                .get_mut(table as usize)
                .ok_or_else(|| invalid("A piece's page names a table past the index's"))?;
            let block = block as usize;
            let follows = |last: Option<&Page>| last.is_none_or(|held| held.first < first);
            if column == ROWS {
                if strips.rows.len() <= block {
                    engine_allocation::resize_default(&mut strips.rows, block + 1)?;
                }
                let pages = &mut strips.rows[block];
                if !follows(pages.last().map(|held| &held.page)) {
                    return Err(invalid("A piece's page does not follow its strip's"));
                }
                engine_allocation::reserve(pages, 1)?;
                pages.push(RowsPage { page, low, high });
            } else {
                let blocks = strips
                    .columns
                    .get_mut(column as usize)
                    .ok_or_else(|| invalid("A piece's page names a column past its table"))?;
                if blocks.len() <= block {
                    engine_allocation::resize_default(blocks, block + 1)?;
                }
                let pages = &mut blocks[block];
                if !follows(pages.last()) {
                    return Err(invalid("A piece's page does not follow its strip's"));
                }
                engine_allocation::reserve(pages, 1)?;
                pages.push(page);
            }
        }
        let base = run.pending.len() as u32;
        engine_allocation::reserve(&mut run.pending, piece.strings.len())?;
        run.pending.extend_from_slice(&piece.strings);
        engine_allocation::reserve(&mut run.ends, piece.ends.len())?;
        run.ends.extend(piece.ends.iter().map(|&end| base + end));
        run.strings = run
            .strings
            .checked_add(piece.ends.len() as u32)
            .ok_or_else(|| invalid("Dictionary ids exceed 32 bits"))?;
        run.store_strings(store, pack, false)
    }

    /// Stores every run's strings not yet in a chunk: the index is whole.
    pub fn finish(&mut self, store: &mut dyn Store, pack: &mut dyn Pack) -> io::Result<()> {
        for run in &mut self.runs {
            run.store_strings(store, pack, true)?;
        }
        Ok(())
    }

    /// The index's own bytes, its pages aside.
    pub fn write(&self) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.raw(MAGIC)?;
        writer.varint(u64::from(self.layout.block))?;
        writer.varint(u64::from(self.layout.runs))?;
        writer.varint(self.tables.len() as u64)?;
        for &table in &self.tables {
            writer.varint(table as u64)?;
        }
        writer.raw(&schema_digest(&self.kinds))?;
        writer.varint(self.runs.len() as u64)?;
        for run in &self.runs {
            if !run.ends.is_empty() {
                return Err(invalid("A run index is written before it is finished"));
            }
            writer.varint(run.branch)?;
            writer.varint(run.first_hour.wire())?;
            writer.varint(run.hours.len() as u64)?;
            let mut previous = run.first_hour;
            for &hour in &run.hours {
                writer.varint(hour.wrapping_sub(previous).wire())?;
                previous = hour;
            }
            for metrics in &run.metrics {
                writer.varint(metrics.total_ns)?;
                writer.varint(metrics.pool_bytes)?;
                writer.varint(metrics.dropped_frames)?;
            }
            writer.varint(run.words as u64)?;
            for &word in &run.read {
                writer.varint(word)?;
            }
            for series in &run.series {
                put_series(&mut writer, &series.statuses, |writer, status| {
                    status.write(writer)
                })?;
                put_series(
                    &mut writer,
                    &series.samples,
                    |writer, sample| match sample {
                        Sample::Each => writer.varint(0),
                        Sample::At(None) => writer.varint(1),
                        Sample::At(Some(hour)) => writer.varint(2 + hour.wire()),
                    },
                )?;
                put_series(&mut writer, &series.slots, |writer, slots| {
                    writer.varint(u64::from(slots))
                })?;
                put_series(&mut writer, &series.views, put_location)?;
            }
            for strips in &run.strips {
                for blocks in &strips.columns {
                    writer.varint(blocks.len() as u64)?;
                    for pages in blocks {
                        put_pages(&mut writer, pages, |_, _| Ok(()))?;
                    }
                }
                writer.varint(strips.rows.len() as u64)?;
                for pages in &strips.rows {
                    put_pages(&mut writer, pages, |writer, page| {
                        writer.varint(page.low as u64)?;
                        writer.varint((page.low >> 64) as u64)?;
                        writer.varint(page.high as u64)?;
                        writer.varint((page.high >> 64) as u64)
                    })?;
                }
            }
            writer.varint(run.chunks.len() as u64)?;
            for chunk in &run.chunks {
                writer.varint(u64::from(chunk.first))?;
                writer.varint(u64::from(chunk.count))?;
                put_location(&mut writer, chunk.location)?;
            }
            writer.varint(u64::from(run.strings))?;
        }
        Ok(writer.into_bytes())
    }

    /// The index [`Self::write`] wrote, its tables' kinds taken from `schema`, the schema of
    /// the recording it indexes.
    pub fn read(bytes: &[u8], schema: &[Vec<Kind>]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.raw(MAGIC.len())? != MAGIC {
            return Err(invalid("Not a run index"));
        }
        let layout = Layout {
            block: reader.varint_u32()?,
            runs: reader.varint_u32()?,
        }
        .check()?;
        let count = reader.count()?;
        let mut tables = engine_allocation::vec(count.min(schema.len()))?;
        let mut kinds = engine_allocation::vec(count.min(schema.len()))?;
        for _ in 0..count {
            let table = reader.count()?;
            if tables.last().is_some_and(|&last| last >= table) {
                return Err(invalid("A run index's tables are not in order"));
            }
            let held = schema
                .get(table)
                .ok_or_else(|| invalid("A run index names a table past the schema"))?;
            tables.push(table);
            kinds.push(engine_allocation::copy_vec(held)?);
        }
        if reader.raw(32)? != schema_digest(&kinds) {
            return Err(invalid("A run index's tables differ from this schema's"));
        }
        let runs = reader.count()?;
        let mut index = Self::new(layout, &tables, kinds, &[])?;
        engine_allocation::reserve(&mut index.runs, runs.min(bytes.len()))?;
        for _ in 0..runs {
            let branch = reader.varint()?;
            let first_hour = i32::unwire(reader.varint()?)?;
            let mut run = RunIndex::new(branch, first_hour, &index.kinds)?;
            let ordinals = reader.count()?;
            engine_allocation::reserve(&mut run.hours, ordinals.min(bytes.len()))?;
            let mut previous = first_hour;
            for _ in 0..ordinals {
                previous = previous.wrapping_add(i32::unwire(reader.varint()?)?);
                run.hours.push(previous);
            }
            engine_allocation::reserve(&mut run.metrics, ordinals.min(bytes.len()))?;
            for _ in 0..ordinals {
                run.metrics.push(Metrics {
                    total_ns: reader.varint()?,
                    pool_bytes: reader.varint()?,
                    dropped_frames: reader.varint()?,
                });
            }
            run.words = reader.count()?;
            let words = ordinals
                .checked_mul(run.words)
                .ok_or_else(|| invalid("A run index's read bits overflow"))?;
            engine_allocation::reserve(&mut run.read, words.min(bytes.len()))?;
            for _ in 0..words {
                run.read.push(reader.varint()?);
            }
            for series in &mut run.series {
                series.statuses = get_series(&mut reader, Status::read)?;
                series.samples = get_series(&mut reader, |reader| {
                    Ok(match reader.varint()? {
                        0 => Sample::Each,
                        1 => Sample::At(None),
                        value => Sample::At(Some(i32::unwire(value - 2)?)),
                    })
                })?;
                series.slots = get_series(&mut reader, |reader| reader.varint_u32())?;
                series.views = get_series(&mut reader, get_location)?;
            }
            for strips in &mut run.strips {
                for blocks in &mut strips.columns {
                    let count = reader.count()?;
                    engine_allocation::reserve(blocks, count.min(bytes.len()))?;
                    for _ in 0..count {
                        blocks.push(get_pages(&mut reader, |page, _| Ok(page))?);
                    }
                }
                let count = reader.count()?;
                engine_allocation::reserve(&mut strips.rows, count.min(bytes.len()))?;
                for _ in 0..count {
                    strips.rows.push(get_pages(&mut reader, |page, reader| {
                        let key = |reader: &mut Reader<'_>| -> io::Result<u128> {
                            let low = reader.varint()?;
                            Ok(u128::from(low) | u128::from(reader.varint()?) << 64)
                        };
                        Ok(RowsPage {
                            page,
                            low: key(reader)?,
                            high: key(reader)?,
                        })
                    })?);
                }
            }
            let within =
                |page: Option<&Page>| page.is_none_or(|page| (page.last as usize) < ordinals);
            for strips in &run.strips {
                let columns = strips.columns.iter().flatten().map(|pages| pages.last());
                let rows = strips
                    .rows
                    .iter()
                    .map(|pages| pages.last().map(|held| &held.page));
                if !columns.chain(rows).all(&within) {
                    return Err(invalid("A strip's page passes its run's ordinals"));
                }
            }
            for (strips, series) in run.strips.iter().zip(&run.series) {
                check_slots(layout, strips, &series.slots)?;
            }
            let chunks = reader.count()?;
            engine_allocation::reserve(&mut run.chunks, chunks.min(bytes.len()))?;
            for _ in 0..chunks {
                run.chunks.push(Chunk {
                    first: reader.varint_u32()?,
                    count: reader.varint_u32()?,
                    location: get_location(&mut reader)?,
                });
            }
            run.strings = reader.varint_u32()?;
            index.runs.push(run);
        }
        reader.finish()?;
        Ok(index)
    }
}

/// Refuses a table's slot count, `slots` from each ordinal on, past the blocks its `strips`
/// hold from that ordinal: each of its blocks has a page of its rows and of every column from
/// the ordinal its first row was inserted at, so a slot count is never taken on trust.
fn check_slots(layout: Layout, strips: &Strips, slots: &[(u32, u32)]) -> io::Result<()> {
    // Each block's ordinal from which every one of its strips has a page, and any before it.
    let mut ready: Vec<u32> = engine_allocation::vec(strips.rows.len())?;
    for (block, rows) in strips.rows.iter().enumerate() {
        let firsts = strips.columns.iter().map(|blocks| {
            blocks
                .get(block)
                .and_then(|pages| pages.first())
                .map(|page| page.first)
        });
        let from = firsts.fold(rows.first().map(|held| held.page.first), |from, first| {
            from.zip(first).map(|(from, first)| from.max(first))
        });
        let Some(from) = from else {
            break;
        };
        ready.push(ready.last().map_or(from, |&before| before.max(from)));
    }
    for &(ordinal, count) in slots {
        let blocks = layout.blocks(count as usize);
        if blocks > 0 && ready.get(blocks - 1).is_none_or(|&from| from > ordinal) {
            return Err(invalid(format!(
                "A table's {count} slots from ordinal {ordinal} pass the blocks its strips hold"
            )));
        }
    }
    Ok(())
}

fn put_location(writer: &mut Writer, location: Location) -> io::Result<()> {
    writer.varint(location.offset)?;
    writer.varint(u64::from(location.stored))?;
    writer.varint(u64::from(location.raw))
}

fn get_location(reader: &mut Reader<'_>) -> io::Result<Location> {
    Ok(Location {
        offset: reader.varint()?,
        stored: reader.varint_u32()?,
        raw: reader.varint_u32()?,
    })
}

/// A series' entries, each its ordinal's difference from the one before and its value.
fn put_series<T: Copy>(
    writer: &mut Writer,
    series: &[(u32, T)],
    mut put: impl FnMut(&mut Writer, T) -> io::Result<()>,
) -> io::Result<()> {
    writer.varint(series.len() as u64)?;
    let mut previous = 0;
    for &(ordinal, value) in series {
        writer.varint(u64::from(ordinal - previous))?;
        put(writer, value)?;
        previous = ordinal;
    }
    Ok(())
}

fn get_series<T>(
    reader: &mut Reader<'_>,
    mut get: impl FnMut(&mut Reader<'_>) -> io::Result<T>,
) -> io::Result<Vec<(u32, T)>> {
    let count = reader.count()?;
    let mut series = engine_allocation::vec(count.min(1 << 20))?;
    let mut ordinal = 0_u32;
    for at in 0..count {
        let delta = reader.varint_u32()?;
        if at > 0 && delta == 0 {
            return Err(invalid("A run index's series repeats an ordinal"));
        }
        ordinal = ordinal
            .checked_add(delta)
            .ok_or_else(|| invalid("A run index's series passes the last ordinal"))?;
        let value = get(reader)?;
        engine_allocation::reserve(&mut series, 1)?;
        series.push((ordinal, value));
    }
    Ok(series)
}

/// A strip's pages: each its first's difference from the one before, its last's from its
/// first, its location and what `put` adds.
fn put_pages<P: AsRef<Page>>(
    writer: &mut Writer,
    pages: &[P],
    mut put: impl FnMut(&mut Writer, &P) -> io::Result<()>,
) -> io::Result<()> {
    writer.varint(pages.len() as u64)?;
    let mut previous = 0;
    for held in pages {
        let page = held.as_ref();
        writer.varint(u64::from(page.first - previous))?;
        writer.varint(u64::from(page.last - page.first))?;
        put_location(writer, page.location)?;
        put(writer, held)?;
        previous = page.first;
    }
    Ok(())
}

fn get_pages<P>(
    reader: &mut Reader<'_>,
    mut get: impl FnMut(Page, &mut Reader<'_>) -> io::Result<P>,
) -> io::Result<Vec<P>> {
    let count = reader.count()?;
    let mut pages = engine_allocation::vec(count.min(1 << 20))?;
    let (mut previous, mut before) = (0_u32, None);
    for _ in 0..count {
        let first = previous
            .checked_add(reader.varint_u32()?)
            .ok_or_else(|| invalid("A strip's pages pass the last ordinal"))?;
        // A page's changes end before the next page's first ordinal.
        if before.is_some_and(|last| last >= first) {
            return Err(invalid("A strip's pages overlap"));
        }
        let last = first
            .checked_add(reader.varint_u32()?)
            .ok_or_else(|| invalid("A strip's pages pass the last ordinal"))?;
        let page = Page {
            first,
            last,
            location: get_location(reader)?,
        };
        engine_allocation::reserve(&mut pages, 1)?;
        pages.push(get(page, reader)?);
        (previous, before) = (first, Some(last));
    }
    Ok(pages)
}
