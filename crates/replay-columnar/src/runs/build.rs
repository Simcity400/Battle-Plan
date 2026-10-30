//! A segment's walk written as its strips' pages, hour by hour.

use super::{Layout, Location, Metrics, PAGE_BYTES, Pack, ROWS, Sample, Series, cell};
use crate::{
    invalid,
    leaf::Writer,
    manifest::Manifest,
    section::Changes,
    state::{Kind, Table},
    walk::State,
};
use std::io;

/// A fill of fewer of a block's cells is written cell by cell.
const FILL_MINIMUM: usize = 16;

/// A page a segment's walk closed: its strip, its ordinals, and where its bytes lie in its
/// piece's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// What a segment's walk wrote of its run's ordinals.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Piece {
    pub run: u32,
    pub first: u32,
    pub ordinals: u32,
    pub hours: Vec<i32>,
    pub metrics: Vec<Metrics>,
    /// Each ordinal's read bits, of every schema table, [`Self::words`] words each.
    pub read: Vec<u64>,
    pub words: usize,
    /// Per indexed table; a view's location is in [`Self::bytes`].
    pub series: Vec<Series>,
    pub pages: Vec<Closed>,
    pub bytes: Vec<u8>,
    /// The run's id of the first string it appended, and the strings, end to end.
    pub strings_first: u32,
    pub strings: Vec<u8>,
    pub ends: Vec<u32>,
}

impl Piece {
    /// The bytes it holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.hours.capacity() * size_of::<i32>()
            + self.metrics.capacity() * size_of::<Metrics>()
            + self.read.capacity() * size_of::<u64>()
            + self.series.capacity() * size_of::<Series>()
            + self
                .series
                .iter()
                .map(|series| {
                    series.statuses.capacity() * size_of::<(u32, crate::manifest::Status)>()
                        + series.samples.capacity() * size_of::<(u32, Sample)>()
                        + series.slots.capacity() * size_of::<(u32, u32)>()
                        + series.views.capacity() * size_of::<(u32, Location)>()
                })
                .sum::<usize>()
            + self.pages.capacity() * size_of::<Closed>()
            + self.bytes.capacity()
            + self.strings.capacity()
            + self.ends.capacity() * size_of::<u32>()
    }
}

/// A strip's page being written.
struct Open {
    first: u32,
    last: u32,
    runs: u32,
    keys: Option<(u128, u128)>,
    /// A column page's cells as its changes are taken against.
    held: cell::Held,
    /// Its cells at its first ordinal, its changes' headers and their values.
    base: Writer,
    head: Writer,
    values: Writer,
}

impl Open {
    fn new(first: u32) -> Self {
        Self {
            first,
            last: first,
            runs: 0,
            keys: None,
            held: cell::Held::default(),
            base: Writer::new(),
            head: Writer::new(),
            values: Writer::new(),
        }
    }

    /// The raw bytes its page holds so far.
    fn bytes(&self) -> usize {
        self.base.bytes().len() + self.head.bytes().len() + self.values.bytes().len()
    }

    fn key(&mut self, key: u128) {
        self.keys = Some(match self.keys {
            Some((low, high)) => (low.min(key), high.max(key)),
            None => (key, key),
        });
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
}

/// One segment's pages of the indexed tables, from the walker's state after each hour.
pub struct Builder {
    layout: Layout,
    tables: Vec<usize>,
    kinds: Vec<Vec<Kind>>,
    pack: Box<dyn Pack>,
    strings: u32,
    piece: Option<Piece>,
    /// Each table's open pages by column, its rows last, then by block.
    open: Vec<Vec<Vec<Option<Open>>>>,
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

/// Packs `raw` into `bytes`, where it lies.
fn stash(
    bytes: &mut Vec<u8>,
    pack: &mut dyn Pack,
    stored: &mut Vec<u8>,
    raw: &[u8],
) -> io::Result<Location> {
    pack.pack(raw, stored)?;
    let offset = bytes.len() as u64;
    engine_allocation::reserve(bytes, stored.len())?;
    bytes.extend_from_slice(stored);
    let width = |length: usize| u32::try_from(length).map_err(|_| invalid("A page passes 4 GiB"));
    Ok(Location {
        offset,
        stored: width(stored.len())?,
        raw: width(raw.len())?,
    })
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
        Ok(Self {
            layout: layout.check()?,
            tables: engine_allocation::copy_vec(tables)?,
            open: engine_allocation::collect_vec(
                kinds
                    .iter()
                    .map(|kinds| (0..=kinds.len()).map(|_| Vec::new()).collect()),
            )?,
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

    /// Ordinal `ordinal` of run `run`, once `state` applied its `manifest`.
    pub fn hour(
        &mut self,
        run: u32,
        ordinal: u32,
        state: &State,
        manifest: &Manifest<'_>,
    ) -> io::Result<()> {
        if self
            .piece
            .as_ref()
            .is_some_and(|piece| piece.run != run || piece.first + piece.ordinals != ordinal)
        {
            return Err(invalid("A segment's hours do not follow one another"));
        }
        if self.piece.is_none() {
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
            open,
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
        for append in &manifest.appends {
            engine_allocation::reserve(&mut piece.strings, append.len())?;
            piece.strings.extend_from_slice(append);
            engine_allocation::reserve(&mut piece.ends, 1)?;
            piece.ends.push(
                u32::try_from(piece.strings.len())
                    .map_err(|_| invalid("A segment's strings pass 4 GiB"))?,
            );
        }
        let mut pages = Pages {
            layout: *layout,
            pack: &mut **pack,
            closed: &mut piece.pages,
            bytes: &mut piece.bytes,
            raw,
            stored,
        };
        for (at, &table) in tables.iter().enumerate() {
            if state.skips(table) {
                return Err(invalid("A walk skips a table its run index holds"));
            }
            let missing = || invalid("The walker holds no such table");
            let held = state.table(table).ok_or_else(missing)?;
            let series = &mut piece.series[at];
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
            let slots =
                u32::try_from(held.slots()).map_err(|_| invalid("A table passes the last slot"))?;
            note(&mut series.slots, ordinal, slots)?;
            if let Some(view) = state.truncated_view(table)
                && views[at].as_deref() != Some(view)
            {
                let mut writer = Writer::new();
                writer.slots(view)?;
                let location = stash(pages.bytes, pages.pack, pages.stored, writer.bytes())?;
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
            let strips = &mut open[at];
            let columns = kinds[at].len();
            let (values, rows) = strips.split_at_mut(columns);
            union(&changes.deleted, &changes.inserted, &mut scratch.slots)?;
            pages.rows(
                at as u32,
                held,
                ordinal,
                &scratch.slots,
                &mut rows[0],
                &mut scratch.group,
            )?;
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

    /// Closes every open page: what the segment wrote. The builder then starts anew.
    pub fn finish(&mut self) -> io::Result<Piece> {
        let mut piece = self
            .piece
            .take()
            .ok_or_else(|| invalid("A segment's walk wrote no hour"))?;
        let mut pages = Pages {
            layout: self.layout,
            pack: &mut *self.pack,
            closed: &mut piece.pages,
            bytes: &mut piece.bytes,
            raw: &mut self.raw,
            stored: &mut self.stored,
        };
        for (at, strips) in self.open.iter_mut().enumerate() {
            let columns = strips.len() - 1;
            for (column, blocks) in strips.iter_mut().enumerate() {
                let column = if column == columns {
                    ROWS
                } else {
                    column as u32
                };
                for (block, open) in blocks.iter_mut().enumerate() {
                    if let Some(open) = open.take() {
                        pages.close(at as u32, column, block as u32, open)?;
                    }
                }
            }
        }
        for view in &mut self.views {
            *view = None;
        }
        Ok(piece)
    }
}

/// Where a segment's pages go.
struct Pages<'a> {
    layout: Layout,
    pack: &'a mut dyn Pack,
    closed: &'a mut Vec<Closed>,
    bytes: &'a mut Vec<u8>,
    raw: &'a mut Vec<u8>,
    stored: &'a mut Vec<u8>,
}

impl Pages<'_> {
    fn close(&mut self, table: u32, column: u32, block: u32, open: Open) -> io::Result<()> {
        let page = cell::assemble(
            &open.base,
            &open.head,
            &open.values,
            std::mem::take(self.raw),
        )?;
        if page.bytes().len() > PAGE_BYTES as usize {
            let what = match column {
                ROWS => "rows".to_owned(),
                column => format!("column {column}"),
            };
            return Err(invalid(format!(
                "Indexed table {table}'s {what} of block {block} pass a change-run page's bytes"
            )));
        }
        let location = stash(self.bytes, self.pack, self.stored, page.bytes());
        *self.raw = page.into_bytes();
        let location = location?;
        engine_allocation::reserve(self.closed, 1)?;
        self.closed.push(Closed {
            table,
            column,
            block,
            first: open.first,
            last: open.last,
            location,
            keys: open.keys.unwrap_or_default(),
        });
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
    fn strip(strips: &mut Vec<Option<Open>>, block: u32) -> io::Result<&mut Option<Open>> {
        let block = block as usize;
        if strips.len() <= block {
            engine_allocation::resize_default(strips, block + 1)?;
        }
        Ok(&mut strips[block])
    }

    /// Closes `strip`'s page once it holds its share of changes, or of bytes.
    fn full(
        &mut self,
        table: u32,
        column: u32,
        block: u32,
        strip: &mut Option<Open>,
    ) -> io::Result<()> {
        if strip.as_ref().is_some_and(|open| {
            open.runs >= self.layout.runs || open.bytes() >= PAGE_BYTES as usize / 4
        }) && let Some(open) = strip.take()
        {
            self.close(table, column, block, open)?;
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
        strips: &mut Vec<Option<Open>>,
        group: &mut cell::Group,
    ) -> io::Result<()> {
        while let Some(&slot) = slots.first() {
            let block = slot / self.layout.block;
            let these = in_block(slots, block, self.layout.block);
            slots = &slots[these.len()..];
            let (start, extent) = self.extent(held, block);
            let strip = Self::strip(strips, block)?;
            if let Some(open) = strip.as_mut() {
                group.start();
                for &slot in these {
                    let key = held.key(slot);
                    group.row(&mut open.values, slot - start, key)?;
                    if let Some(key) = key {
                        open.key(key);
                    }
                }
                let written = group.close(&mut open.head, open.last, ordinal)?;
                if written > 0 {
                    open.last = ordinal;
                    open.runs += written;
                }
            } else {
                let mut open = Open::new(ordinal);
                open.keys = cell::put_rows(&mut open.base, held, start, extent)?;
                *strip = Some(open);
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
        strips: &mut Vec<Option<Open>>,
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
            if strip.is_none() {
                let mut open = Open::new(ordinal);
                cell::put_base(
                    &mut open.base,
                    held,
                    column,
                    kind,
                    start,
                    extent,
                    &mut open.held,
                )?;
                *strip = Some(open);
                continue;
            }
            let Some(open) = strip.as_mut() else {
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
            let cells = match fill {
                Some(value) => {
                    group.fill(&mut open.values, except, value, extent, &mut open.held)?;
                    &overrides[..]
                }
                None => &written[..],
            };
            for &slot in cells {
                group.change(
                    &mut open.values,
                    held,
                    column,
                    slot,
                    slot - start,
                    &mut open.held,
                )?;
            }
            let count = group.close(&mut open.head, open.last, ordinal)?;
            if count > 0 {
                open.last = ordinal;
                open.runs += count;
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
