//! The map's copy of its layers' columns, one per table on the frame side: the union of the drawn
//! layers' columns by slot, typed, with live bits, keys when a layer asks, the strings of its text
//! columns, and the table's status. Made at a reset on the playhead's fetchers from the sought lane
//! ([`rebuild`]), then kept by applying each step once, before any layer runs. It always holds the
//! state at the last step applied; only the host writes it. A log's mirror holds the rows it lists:
//! read by ordinal at a reset, each step's appends put at free slots and its forgets taken out.

use crate::layers::{List, Num, Number, Text};
use playhead::{
    Rebuild, Rows, State, Step, Wanted,
    changes::{Changes, Raw},
    lane::Lane,
    pages::Pages,
};
use replay_columnar::{manifest::Status, state::Kind, store::Value};
use replay_data::{Hour, rows::TableId, scope::CaptureMode};

/// A reason the recording's strings do not name (the strings after a seek, before the recorder's
/// step 4 fixed them).
fn unnamed(reason: u32) -> Box<str> {
    format!("reason {reason}, whose text the recording did not name here").into()
}
use std::{collections::HashMap, io, sync::Arc};

const ABSENT: u16 = u16::MAX;
/// What a table's missing reason reads when its recorded text was not read with it.
pub const UNREAD_REASON: &str = "its recorded reason was not read";

/// The tests' count of this thread's row reads: what a layer's `apply` must not do.
#[cfg(any(test, feature = "test-support"))]
pub mod probe {
    use std::cell::Cell;

    thread_local! {
        static READS: Cell<u64> = const { Cell::new(0) };
    }

    pub fn read() {
        READS.with(|reads| reads.set(reads.get() + 1));
    }

    /// Row reads on this thread so far.
    pub fn reads() -> u64 {
        READS.with(Cell::get)
    }
}

/// The key index's hasher: a fixed-seed folded multiply, several times faster than SipHash on a
/// 128-bit key; no key is chosen by an adversary.
pub type Keys = foldhash::fast::FixedState;

/// A cell as a layer reads it: missing and not recorded are never a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell<T> {
    Value(T),
    /// An optional number without a value.
    Missing,
    Unrecorded,
}

impl<T> Cell<T> {
    pub fn value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
}

/// Slots, one bit each, grown on demand.
#[derive(Clone, Debug, Default)]
struct Bits(Vec<u64>);

impl Bits {
    #[inline]
    fn get(&self, slot: usize) -> bool {
        self.0.get(slot / 64).is_some_and(|word| word & (1 << (slot % 64)) != 0)
    }

    #[inline]
    fn set(&mut self, slot: usize, on: bool) {
        let word = slot / 64;
        if word >= self.0.len() {
            if !on {
                return;
            }
            self.0.resize(word + 1, 0);
        }
        if on {
            self.0[word] |= 1 << (slot % 64);
        } else {
            self.0[word] &= !(1 << (slot % 64));
        }
    }

    /// Each slot `new` from `first` on takes slot `old[new]`'s bit (`old` ascending, so never a
    /// bit already moved), and the slots past `old` are cleared: a renumber's packing.
    fn renumber(&mut self, old: &[u32], first: usize) {
        for (new, &from) in old.iter().enumerate().skip(first) {
            let on = self.get(from as usize);
            self.set(new, on);
        }
        self.0.truncate(old.len().div_ceil(64));
        if let Some(last) = self.0.last_mut().filter(|_| old.len() % 64 != 0) {
            *last &= (1 << (old.len() % 64)) - 1;
        }
    }

    fn count(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    /// The lowest slot not set.
    fn first_free(&self) -> usize {
        self.0.iter().position(|&word| word != u64::MAX).map_or(self.0.len() * 64, |at| at * 64 + self.0[at].trailing_ones() as usize)
    }

    fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(at, &word)| {
            let mut left = word;
            std::iter::from_fn(move || {
                (left != 0).then(|| {
                    let bit = left.trailing_zeros();
                    left &= left - 1;
                    at as u32 * 64 + bit
                })
            })
        })
    }

    fn bytes(&self) -> usize {
        self.0.capacity() * 8
    }
}

/// One column's cells by slot, at the width its kind holds.
#[derive(Clone, Debug)]
enum Cells {
    W1(Vec<u8>),
    W2(Vec<u16>),
    W4(Vec<u32>),
    W8(Vec<u64>),
    /// Each cell's bytes; a cell keeps its buffer when written again.
    Lists(Vec<Vec<u8>>),
}

impl Cells {
    fn new(kind: Kind) -> Self {
        match kind {
            Kind::U8 | Kind::I8 | Kind::Bool => Self::W1(Vec::new()),
            Kind::U16 | Kind::I16 => Self::W2(Vec::new()),
            Kind::U32 | Kind::I32 | Kind::OptionalU32 | Kind::OptionalI32 | Kind::Bytes => Self::W4(Vec::new()),
            Kind::U64 | Kind::I64 | Kind::OptionalU64 | Kind::OptionalI64 => Self::W8(Vec::new()),
            Kind::U32List | Kind::U64List | Kind::Opaque => Self::Lists(Vec::new()),
        }
    }

    fn grow(&mut self, slots: usize) {
        match self {
            Self::W1(cells) if cells.len() < slots => cells.resize(slots, 0),
            Self::W2(cells) if cells.len() < slots => cells.resize(slots, 0),
            Self::W4(cells) if cells.len() < slots => cells.resize(slots, 0),
            Self::W8(cells) if cells.len() < slots => cells.resize(slots, 0),
            Self::Lists(cells) if cells.len() < slots => cells.resize_with(slots, Vec::new),
            _ => {}
        }
    }

    #[inline]
    fn put(&mut self, slot: usize, bits: u64) {
        match self {
            Self::W1(cells) => cells[slot] = bits as u8,
            Self::W2(cells) => cells[slot] = bits as u16,
            Self::W4(cells) => cells[slot] = bits as u32,
            Self::W8(cells) => cells[slot] = bits,
            Self::Lists(_) => {}
        }
    }

    #[inline]
    fn raw(&self, slot: usize) -> u64 {
        match self {
            Self::W1(cells) => u64::from(cells[slot]),
            Self::W2(cells) => u64::from(cells[slot]),
            Self::W4(cells) => u64::from(cells[slot]),
            Self::W8(cells) => cells[slot],
            Self::Lists(_) => 0,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::W1(cells) => cells.len(),
            Self::W2(cells) => cells.len(),
            Self::W4(cells) => cells.len(),
            Self::W8(cells) => cells.len(),
            Self::Lists(cells) => cells.len(),
        }
    }

    /// Slots `first` on take their old slots' cells; the rows before `first` kept theirs. `lists`,
    /// the bytes its list cells hold, loses the cells the packing drops.
    fn renumber(&mut self, old: &[u32], first: usize, lists: &mut usize) {
        fn moved<T: Copy>(cells: &mut Vec<T>, old: &[u32], first: usize) {
            for (new, &from) in old.iter().enumerate().skip(first) {
                cells[new] = cells[from as usize];
            }
            cells.truncate(old.len());
        }
        match self {
            Self::W1(cells) => moved(cells, old, first),
            Self::W2(cells) => moved(cells, old, first),
            Self::W4(cells) => moved(cells, old, first),
            Self::W8(cells) => moved(cells, old, first),
            Self::Lists(cells) => {
                for (new, &from) in old.iter().enumerate().skip(first) {
                    cells.swap(new, from as usize);
                }
                if let Some(dropped) = cells.get(old.len()..) {
                    *lists -= dropped.iter().map(Vec::capacity).sum::<usize>();
                }
                cells.truncate(old.len());
            }
        }
    }

    /// The bytes the cells themselves hold; a list cell's own bytes are counted as they are
    /// written ([`Column::lists`]).
    fn bytes(&self) -> usize {
        match self {
            Self::W1(cells) => cells.capacity(),
            Self::W2(cells) => cells.capacity() * 2,
            Self::W4(cells) => cells.capacity() * 4,
            Self::W8(cells) => cells.capacity() * 8,
            Self::Lists(cells) => cells.capacity() * 24,
        }
    }
}

/// One column of a mirror looked up once ([`Mirror::resolve`]); none when the mirror does not hold
/// it, which reads as not recorded.
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'m>(Option<&'m Column>);

impl<'m> Resolved<'m> {
    /// The cell's bits as held: a number at its width, a text id, missing or not recorded.
    #[inline]
    pub fn bits(&self, slot: u32) -> Cell<u64> {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        let Some(held) = self.0 else { return Cell::Unrecorded };
        let slot = slot as usize;
        if slot >= held.cells.len() {
            return Cell::Unrecorded;
        }
        match held.standing(slot) {
            Some(Cell::Missing) => Cell::Missing,
            Some(_) => Cell::Unrecorded,
            None => Cell::Value(held.cells.raw(slot)),
        }
    }

    /// A list or opaque cell's bytes.
    #[inline]
    pub fn list(&self, slot: u32) -> Cell<&'m [u8]> {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        let Some(held) = self.0 else { return Cell::Unrecorded };
        if let Some(standing) = held.standing(slot as usize) {
            return match standing {
                Cell::Missing => Cell::Missing,
                _ => Cell::Unrecorded,
            };
        }
        match &held.cells {
            Cells::Lists(cells) => Cell::Value(cells.get(slot as usize).map_or(&[][..], |cell| &cell[..])),
            _ => Cell::Unrecorded,
        }
    }
}

#[derive(Clone, Debug)]
struct Column {
    index: usize,
    cells: Cells,
    missing: Option<Bits>,
    unrecorded: Option<Bits>,
    /// The bytes its list cells hold, kept as they are written: a mirror's bytes are asked for
    /// every wake of a worker, and walking every cell for them is the table's own size.
    lists: usize,
}

impl Column {
    #[inline]
    fn set(&mut self, slot: usize, value: Value<'_>) {
        let (missing, unrecorded) = match value {
            Value::Number(bits) => {
                self.cells.put(slot, bits);
                (false, false)
            }
            Value::Text(id) => {
                self.cells.put(slot, u64::from(id));
                (false, false)
            }
            Value::List(bytes) => {
                if let Cells::Lists(cells) = &mut self.cells {
                    self.lists -= cells[slot].capacity();
                    cells[slot].clear();
                    cells[slot].extend_from_slice(bytes);
                    self.lists += cells[slot].capacity();
                }
                (false, false)
            }
            Value::Missing => (true, false),
            Value::Unrecorded => (false, true),
        };
        mark(&mut self.missing, slot, missing);
        mark(&mut self.unrecorded, slot, unrecorded);
    }

    /// A change set's cells of this column from its raw bytes, one loop for the stored width and
    /// the held one; then the missing and unrecorded bits, touched only when held or written.
    fn write(&mut self, written: &playhead::changes::Written) {
        let slots = written.slots();
        let Self { cells, missing, unrecorded, lists, .. } = self;
        match (cells, written.raw()) {
            (Cells::W1(cells), Raw::Numbers { width, signed, bytes }) => scatter(cells, slots, width, signed, bytes, |bits| bits as u8),
            (Cells::W2(cells), Raw::Numbers { width, signed, bytes }) => scatter(cells, slots, width, signed, bytes, |bits| bits as u16),
            (Cells::W4(cells), Raw::Numbers { width, signed, bytes }) => scatter(cells, slots, width, signed, bytes, |bits| bits as u32),
            (Cells::W4(cells), Raw::Texts { width, bytes }) => scatter(cells, slots, width, false, bytes, |bits| bits as u32),
            (Cells::W8(cells), Raw::Numbers { width, signed, bytes }) => scatter(cells, slots, width, signed, bytes, |bits| bits),
            (Cells::Lists(cells), Raw::Lists { ends, bytes }) => {
                let mut start = 0;
                for (&slot, &end) in slots.iter().zip(ends) {
                    let cell = &mut cells[slot as usize];
                    *lists -= cell.capacity();
                    cell.clear();
                    cell.extend_from_slice(&bytes[start..end as usize]);
                    *lists += cell.capacity();
                    start = end as usize;
                }
            }
            (_, _) => {}
        }
        let (gone, lost) = (written.missing(), written.unrecorded());
        if missing.is_some() || unrecorded.is_some() || !gone.is_empty() || !lost.is_empty() {
            if missing.is_some() || unrecorded.is_some() {
                for &slot in slots {
                    clear(missing, slot as usize);
                    clear(unrecorded, slot as usize);
                }
            }
            for &at in gone {
                mark(missing, slots[at as usize] as usize, true);
            }
            for &at in lost {
                mark(unrecorded, slots[at as usize] as usize, true);
            }
        }
    }

    #[inline]
    fn standing(&self, slot: usize) -> Option<Cell<()>> {
        if self.unrecorded.as_ref().is_some_and(|bits| bits.get(slot)) {
            Some(Cell::Unrecorded)
        } else if self.missing.as_ref().is_some_and(|bits| bits.get(slot)) {
            Some(Cell::Missing)
        } else {
            None
        }
    }

    fn renumber(&mut self, old: &[u32], first: usize) {
        self.cells.renumber(old, first, &mut self.lists);
        for bits in [&mut self.missing, &mut self.unrecorded].into_iter().flatten() {
            bits.renumber(old, first);
        }
    }

    fn bytes(&self) -> usize {
        self.cells.bytes() + self.lists + self.missing.as_ref().map_or(0, Bits::bytes) + self.unrecorded.as_ref().map_or(0, Bits::bytes)
    }
}

/// Each slot's cell read from `bytes` at `width` (sign-extended when `signed`) into `cells`.
#[inline]
fn scatter<T>(cells: &mut [T], slots: &[u32], width: u8, signed: bool, bytes: &[u8], cast: impl Fn(u64) -> T) {
    match (width, signed) {
        (0, _) => slots.iter().for_each(|&slot| cells[slot as usize] = cast(0)),
        (1, false) => slots.iter().zip(bytes).for_each(|(&slot, &byte)| cells[slot as usize] = cast(u64::from(byte))),
        (1, true) => slots.iter().zip(bytes).for_each(|(&slot, &byte)| cells[slot as usize] = cast(byte as i8 as i64 as u64)),
        (2, signed) => slots.iter().zip(bytes.chunks_exact(2)).for_each(|(&slot, held)| {
            let bits = u16::from_le_bytes([held[0], held[1]]);
            cells[slot as usize] = cast(if signed { bits as i16 as i64 as u64 } else { u64::from(bits) });
        }),
        (4, signed) => slots.iter().zip(bytes.chunks_exact(4)).for_each(|(&slot, held)| {
            let bits = u32::from_le_bytes([held[0], held[1], held[2], held[3]]);
            cells[slot as usize] = cast(if signed { bits as i32 as i64 as u64 } else { u64::from(bits) });
        }),
        _ => slots.iter().zip(bytes.chunks_exact(8)).for_each(|(&slot, held)| cells[slot as usize] = cast(u64::from_le_bytes(held.try_into().expect("eight bytes")))),
    }
}

/// Clears `slot`'s bit when the bits are held; they cover every slot of the column.
#[inline]
fn clear(bits: &mut Option<Bits>, slot: usize) {
    if let Some(bits) = bits
        && let Some(word) = bits.0.get_mut(slot / 64)
    {
        *word &= !(1 << (slot % 64));
    }
}

#[inline]
fn mark(bits: &mut Option<Bits>, slot: usize, on: bool) {
    match bits {
        Some(bits) => bits.set(slot, on),
        None if on => bits.get_or_insert_with(Bits::default).set(slot, true),
        None => {}
    }
}

/// One table's mirror.
#[derive(Clone, Debug)]
pub struct Mirror {
    table: usize,
    columns: Vec<Column>,
    /// Each table column's place in `columns`, or `ABSENT`.
    position: Vec<u16>,
    live: Bits,
    keys: Option<Vec<u128>>,
    /// Each live row's slot by key, when keyed.
    index: Option<HashMap<u128, u32, Keys>>,
    /// Whether it holds a text column, so it keeps the table's strings.
    text: bool,
    texts: HashMap<u32, Box<[u8]>>,
    /// The bytes `texts` holds, kept as strings are named ([`Column::lists`] for why).
    texts_bytes: usize,
    status: (Status, Option<Hour>),
    /// An unavailable status's reason: its string id and text.
    reason: Option<(u32, Box<str>)>,
    /// The recording's capture mode, which says why a table is not recorded.
    capture: Option<CaptureMode>,
    /// The row format its cells were written in: an opaque cell decodes at it.
    format: u16,
    head: Option<Hour>,
    gap: bool,
    /// Change sets naming a column it does not hold.
    refused: u64,
    /// Rows its renumbers moved to another slot.
    #[cfg(any(test, feature = "test-support"))]
    moved: u64,
    /// A log's listed rows' slots by ordinal; none for a table holding state.
    listed: Option<HashMap<u64, u32>>,
}

impl Mirror {
    /// The cells of `columns` (ascending) of a sought lane, with its keys when it is keyed.
    pub fn rebuild(lane: &Lane, columns: &[usize]) -> Self {
        let cells = lane.cells();
        let kinds: Vec<Kind> = cells.columns().iter().map(|column| column.kind()).collect();
        let mut position = vec![ABSENT; kinds.len()];
        let mut held = Vec::with_capacity(columns.len());
        for (at, &index) in columns.iter().enumerate() {
            position[index] = at as u16;
            held.push(Column { index, cells: Cells::new(kinds[index]), missing: None, unrecorded: None, lists: 0 });
        }
        let slots = cells.slots() as usize;
        let live: Vec<u32> = cells.rows().collect();
        let mut keys = lane.keyed().then(|| vec![0; slots]);
        let mut index = lane.keyed().then(|| HashMap::with_capacity_and_hasher(live.len(), Keys::default()));
        let mut bits = Bits(vec![0; slots.div_ceil(64)]);
        for &slot in &live {
            bits.set(slot as usize, true);
            if let (Some(keys), Some(index)) = (&mut keys, &mut index) {
                let key = cells.key(slot).expect("a live row has a key");
                keys[slot as usize] = key;
                index.insert(key, slot);
            }
        }
        for column in &mut held {
            column.cells.grow(slots);
            let source = cells.column(column.index).expect("a selected column");
            for &slot in &live {
                column.set(slot as usize, source.value(slot));
            }
        }
        let text = columns.iter().any(|&index| kinds[index] == Kind::Bytes);
        let texts: HashMap<u32, Box<[u8]>> = if text { lane.texts().iter().map(|(id, text)| (id, text.into())).collect() } else { HashMap::new() };
        let texts_bytes = texts.values().map(|text| text.len() + 40).sum();
        let reason = match lane.status().0 {
            Status::Unavailable { reason } => Some((reason, lane.texts().iter().find(|&(id, _)| id == reason).map_or_else(|| unnamed(reason), |(_, text)| String::from_utf8_lossy(text).into()))),
            _ => None,
        };
        let listed = lane.log().map(|_| HashMap::new());
        Self {
            table: lane.table(),
            columns: held,
            position,
            live: bits,
            keys,
            index,
            text,
            texts,
            texts_bytes,
            status: lane.status(),
            reason,
            capture: None,
            format: lane.format(),
            head: lane.head(),
            gap: false,
            refused: 0,
            #[cfg(any(test, feature = "test-support"))]
            moved: 0,
            listed,
        }
    }

    /// Names string `id` to the mirror, keeping the bytes its strings hold.
    fn name(&mut self, id: u32, text: &[u8]) {
        self.texts_bytes += text.len() + 40;
        if let Some(held) = self.texts.insert(id, text.into()) {
            self.texts_bytes -= held.len() + 40;
        }
    }

    /// A log's mirror given the rows it lists, read by ordinal ([`playhead::lane::log::rows`]).
    pub fn put_listed(&mut self, rows: &Rows) {
        let Some(listed) = &mut self.listed else { return };
        for (place, &ordinal) in rows.ordinals().iter().enumerate() {
            listed.insert(ordinal, place as u32);
            self.live.set(place, true);
            if let Some(keys) = &mut self.keys {
                if keys.len() <= place {
                    keys.resize(place + 1, 0);
                }
                keys[place] = rows.keys()[place];
                if let Some(index) = &mut self.index {
                    index.insert(keys[place], place as u32);
                }
            }
        }
        for written in rows.columns() {
            let Some(&at) = self.position.get(written.column()).filter(|&&at| at != ABSENT) else { continue };
            let column = &mut self.columns[at as usize];
            column.cells.grow(rows.len());
            for (place, value) in written.values() {
                column.set(place as usize, value);
            }
        }
        if self.text {
            for (id, text) in rows.texts().iter() {
                self.name(id, text);
            }
        }
    }

    /// A step's appends and forgets of a log: each appended row at the lowest free slot with its
    /// key and cells, then each forgotten row taken out.
    fn apply_log(&mut self, changes: &Changes) {
        let (Some(log), Some(listed)) = (changes.log(), &mut self.listed) else { return };
        let mut slots = Vec::with_capacity(log.len());
        for (place, ordinal, _) in log.rows() {
            let slot = self.live.first_free();
            self.live.set(slot, true);
            listed.insert(ordinal, slot as u32);
            slots.push(slot);
            if let (Some(keys), Some(held)) = (&mut self.keys, log.keys()) {
                if keys.len() <= slot {
                    keys.resize(slot + 1, 0);
                }
                keys[slot] = held[place as usize];
                if let Some(index) = &mut self.index {
                    index.insert(keys[slot], slot as u32);
                }
            }
        }
        for written in log.columns() {
            let Some(&at) = self.position.get(written.column()).filter(|&&at| at != ABSENT) else { continue };
            let column = &mut self.columns[at as usize];
            if let Some(&most) = slots.iter().max() {
                column.cells.grow(most + 1);
            }
            for (place, value) in written.values() {
                column.set(slots[place as usize], value);
            }
        }
        for run in log.forgotten() {
            for ordinal in run.from..run.to {
                let Some(slot) = listed.remove(&ordinal) else { continue };
                self.live.set(slot as usize, false);
                if let (Some(keys), Some(index)) = (&self.keys, &mut self.index)
                    && let Some(&key) = keys.get(slot as usize)
                    && index.get(&key) == Some(&slot)
                {
                    index.remove(&key);
                }
            }
        }
    }

    /// A step's change set of this table, in its order: deletes, inserts, cells, renumber.
    pub fn apply(&mut self, changes: &Changes) {
        self.gap = false;
        self.apply_log(changes);
        let deleted = changes.deleted();
        for &slot in deleted.slots() {
            self.live.set(slot as usize, false);
            if let (Some(keys), Some(index)) = (&self.keys, &mut self.index)
                && let Some(&key) = keys.get(slot as usize)
                && index.get(&key) == Some(&slot)
            {
                index.remove(&key);
            }
        }
        let inserted = changes.inserted();
        for (at, &slot) in inserted.slots().iter().enumerate() {
            self.live.set(slot as usize, true);
            if let Some(keys) = &mut self.keys {
                if keys.len() <= slot as usize {
                    keys.resize(slot as usize + 1, 0);
                }
                let key = inserted.keys().map_or(0, |held| held[at]);
                keys[slot as usize] = key;
                if let Some(index) = &mut self.index {
                    index.insert(key, slot);
                }
            }
        }
        for written in changes.columns() {
            let Some(&at) = self.position.get(written.column()).filter(|&&at| at != ABSENT) else {
                if !written.is_empty() {
                    self.refused += 1;
                }
                continue;
            };
            let column = &mut self.columns[at as usize];
            if let Some(&most) = written.slots().iter().max() {
                column.cells.grow(most as usize + 1);
            }
            column.write(written);
        }
        if let Some(old) = changes.renumber() {
            self.renumber(old);
        }
        if self.text {
            for (id, text) in changes.texts().iter() {
                self.name(id, text);
            }
        }
        if let Some(status) = changes.status() {
            self.status = status;
            // Read in the dictionary of the run the change set is of: a new run numbers its strings
            // anew, so an equal id may spell another reason. A change set naming no text for it (a
            // seek's reset of a table holding no text) keeps the reason its rebuild read.
            self.reason = match status.0 {
                Status::Unavailable { reason } => {
                    let named = changes.texts().iter().any(|(id, _)| id == reason) || self.texts.contains_key(&reason);
                    match self.reason.take() {
                        Some(held) if held.0 == reason && !named => Some(held),
                        _ => Some((reason, reason_words(reason, changes, self.texts.get(&reason).map(|text| &text[..])))),
                    }
                }
                _ => None,
            };
        }
        self.advance_head(changes);
    }

    pub(crate) fn advance_head(&mut self, changes: &Changes) {
        self.head = changes.head().or(self.head);
    }

    /// The live rows packed to the slots `old` (ascending) lists them at: the rows before the
    /// first that moves keep their slots and are not touched.
    fn renumber(&mut self, old: &[u32]) {
        debug_assert!(old.windows(2).all(|pair| pair[0] < pair[1]), "a renumber lists the old slots ascending");
        let first = old.iter().enumerate().position(|(new, &from)| from as usize != new).unwrap_or(old.len());
        #[cfg(any(test, feature = "test-support"))]
        {
            self.moved += (old.len() - first) as u64;
        }
        self.live.renumber(old, first);
        if let Some(keys) = &mut self.keys {
            if keys.len() < old.len() {
                keys.resize(old.len(), 0);
            }
            for (new, &from) in old.iter().enumerate().skip(first) {
                keys[new] = keys.get(from as usize).copied().unwrap_or(0);
            }
            keys.truncate(old.len());
            if let Some(index) = &mut self.index {
                for slot in (first..old.len()).filter(|&slot| self.live.get(slot)) {
                    index.insert(keys[slot], slot as u32);
                }
                // Each live row's key once, as the index holds them: else it is made whole again.
                if index.len() != self.live.count() {
                    index.clear();
                    index.extend(self.live.ones().map(|slot| (keys[slot as usize], slot)));
                }
            }
        }
        self.columns.iter_mut().for_each(|column| column.renumber(old, first));
    }

    /// Rows its renumbers moved to another slot since it was made.
    #[cfg(any(test, feature = "test-support"))]
    pub fn moved(&self) -> u64 {
        self.moved
    }

    pub fn table(&self) -> TableId {
        TableId::ALL[self.table]
    }

    /// One past the highest slot it holds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn slots(&self) -> usize {
        self.live.0.len() * 64
    }

    #[inline]
    pub fn live(&self, slot: u32) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        self.live.get(slot as usize)
    }

    /// Its live rows' slots, ascending.
    pub fn rows(&self) -> impl Iterator<Item = u32> + '_ {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        self.live.ones()
    }

    /// The row's key, when the table is read keyed; a slot's key stays until the slot is reused.
    #[inline]
    pub fn key(&self, slot: u32) -> Option<u128> {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        self.keys.as_ref().and_then(|keys| keys.get(slot as usize).copied())
    }

    /// The live row of `key`, when the table is read keyed.
    #[inline]
    pub fn slot(&self, key: u128) -> Option<u32> {
        #[cfg(any(test, feature = "test-support"))]
        probe::read();
        self.index.as_ref().and_then(|index| index.get(&key).copied())
    }

    #[inline]
    pub fn number<T: Number>(&self, column: Num<T>, slot: u32) -> Cell<T> {
        debug_assert_eq!(column.table.index(), self.table, "a column of another table");
        match self.raw(column.column, slot) {
            Cell::Value(raw) => Cell::Value(T::from_raw(raw)),
            Cell::Missing => Cell::Missing,
            Cell::Unrecorded => Cell::Unrecorded,
        }
    }

    /// A text cell's string id; [`Self::string`] gives its bytes.
    #[inline]
    pub fn text(&self, column: Text, slot: u32) -> Cell<u32> {
        debug_assert_eq!(column.table.index(), self.table, "a column of another table");
        match self.raw(column.column, slot) {
            Cell::Value(raw) => Cell::Value(raw as u32),
            Cell::Missing => Cell::Missing,
            Cell::Unrecorded => Cell::Unrecorded,
        }
    }

    /// The bytes of string `id` of this table, once it has been named to the mirror.
    pub fn string(&self, id: u32) -> Option<&[u8]> {
        self.texts.get(&id).map(|text| &text[..])
    }

    pub fn list(&self, column: List, slot: u32) -> Cell<&[u8]> {
        debug_assert_eq!(column.table.index(), self.table, "a column of another table");
        self.resolve(column.column).list(slot)
    }

    /// `column` resolved once, for a run of reads over many rows: every row's cell of one column
    /// of a view costs the same lookup, which a maker of rows does once instead.
    #[inline]
    pub fn resolve(&self, column: usize) -> Resolved<'_> {
        Resolved(self.column(column))
    }

    #[inline]
    fn raw(&self, column: usize, slot: u32) -> Cell<u64> {
        self.resolve(column).bits(slot)
    }

    #[inline]
    fn column(&self, column: usize) -> Option<&Column> {
        self.position.get(column).filter(|&&at| at != ABSENT).map(|&at| &self.columns[at as usize])
    }

    /// A cell as its held bytes: a number at its width, a text id in four, a list's bytes.
    #[cfg(any(test, feature = "test-support"))]
    pub fn owned(&self, column: usize, slot: u32) -> Cell<Vec<u8>> {
        let Some(held) = self.column(column) else { return Cell::Unrecorded };
        match (held.standing(slot as usize), &held.cells) {
            (Some(Cell::Missing), _) => Cell::Missing,
            (Some(_), _) => Cell::Unrecorded,
            (None, Cells::Lists(cells)) => Cell::Value(cells[slot as usize].to_vec()),
            (None, Cells::W1(cells)) => Cell::Value(vec![cells[slot as usize]]),
            (None, Cells::W2(cells)) => Cell::Value(cells[slot as usize].to_le_bytes().to_vec()),
            (None, Cells::W4(cells)) => Cell::Value(cells[slot as usize].to_le_bytes().to_vec()),
            (None, Cells::W8(cells)) => Cell::Value(cells[slot as usize].to_le_bytes().to_vec()),
        }
    }

    /// A number or text cell's bits as held, when it holds a value.
    pub fn raw_cell(&self, column: usize, slot: u32) -> Option<u64> {
        self.raw(column, slot).value()
    }

    /// A number or text cell as held, missing and not recorded included: what a layer copies to
    /// a worker of its own.
    pub fn cell(&self, column: usize, slot: u32) -> Cell<u64> {
        self.raw(column, slot)
    }

    /// A list cell's bytes by column index, for the same copies.
    pub fn list_cell(&self, column: usize, slot: u32) -> Cell<&[u8]> {
        self.resolve(column).list(slot)
    }

    /// The hour its table was last sampled at.
    pub fn sampled(&self) -> Option<Hour> {
        self.status.1
    }

    /// Whether it holds `column`.
    pub fn holds(&self, column: usize) -> bool {
        self.column(column).is_some()
    }

    /// The columns its table has in the recording, which a lane older than the schema has fewer
    /// of: a column past them is recorded nowhere, and every read of it says so.
    pub fn width(&self) -> usize {
        self.position.len()
    }

    /// Where a layer could tell this mirror from `other`, in words; none where it could not. The
    /// judge of a piece's crossing (`BATTLE_PLAN_VIEWER_CHECK_PIECE`) reads the mirror the crossing's
    /// change sets left against the one a rebuild of the new piece's lane made: the status, the
    /// head, the live rows and their keys, and every held column's cell of every live row, a text
    /// cell by the string each mirror names its id with.
    pub fn differs(&self, other: &Self, kinds: &[Kind]) -> Option<String> {
        if self.status != other.status {
            return Some(format!("its status reads {:?} against {:?}", self.status, other.status));
        }
        if self.head != other.head {
            return Some(format!("its head stands at {:?} against {:?}", self.head, other.head));
        }
        let (mine, theirs): (Vec<u32>, Vec<u32>) = (self.rows().collect(), other.rows().collect());
        if mine != theirs {
            let differing = mine.iter().zip(&theirs).position(|(mine, theirs)| mine != theirs);
            return Some(format!("it holds {} live rows against {}, differing at place {differing:?}", mine.len(), theirs.len()));
        }
        for &slot in &mine {
            if self.key(slot) != other.key(slot) {
                return Some(format!("slot {slot}'s key reads {:?} against {:?}", self.key(slot), other.key(slot)));
            }
            for held in &other.columns {
                let column = held.index;
                if !self.holds(column) {
                    return Some(format!("it holds no column {column}"));
                }
                if self.cell(column, slot) != other.cell(column, slot) {
                    return Some(format!("slot {slot}'s column {column} reads {:?} against {:?}", self.cell(column, slot), other.cell(column, slot)));
                }
                if self.list_cell(column, slot) != other.list_cell(column, slot) {
                    return Some(format!("slot {slot}'s list column {column} differs"));
                }
                if kinds.get(column) == Some(&Kind::Bytes)
                    && let Cell::Value(id) = self.cell(column, slot)
                    && self.string(id as u32) != other.string(id as u32)
                {
                    return Some(format!("slot {slot}'s column {column} names string {id} {:?} against {:?}", self.string(id as u32).map(String::from_utf8_lossy), other.string(id as u32).map(String::from_utf8_lossy)));
                }
            }
        }
        if self.columns.len() != other.columns.len() {
            return Some(format!("it holds {} columns against {}", self.columns.len(), other.columns.len()));
        }
        None
    }

    /// The table's status and sample hour; during a gap nothing is recorded, whatever it held.
    pub fn status(&self) -> Option<Status> {
        (!self.gap).then_some(self.status.0)
    }

    /// The whole table is recorded at the head: complete and not in a gap.
    pub fn complete(&self) -> bool {
        self.status() == Some(Status::Complete)
    }

    /// Why the table is not wholly recorded at the head, by name; none when it is.
    pub fn not_recorded(&self) -> Option<String> {
        not_recorded(self.table(), self.status(), self.capture, self.reason())
    }

    /// Its status and sample hour whatever a gap says, whether a gap step left it unrecorded, and
    /// an unavailable status's reason: what a worker's own copy of the table starts from.
    pub fn standing(&self) -> ((Status, Option<Hour>), bool, Option<(u32, &str)>) {
        (self.status, self.gap, self.reason.as_ref().map(|(id, text)| (*id, &**text)))
    }

    /// The table's strings, when it keeps them (it holds a text column).
    pub fn strings(&self) -> Option<impl Iterator<Item = (u32, &[u8])> + '_> {
        self.text.then(|| self.texts.iter().map(|(&id, text)| (id, &text[..])))
    }
}

/// An unavailable status's reason in words: as the step names it, as named before (`held`), or
/// unnamed.
pub fn reason_words(reason: u32, changes: &Changes, held: Option<&[u8]>) -> Box<str> {
    let named = changes.texts().iter().find(|&(id, _)| id == reason).map(|(_, text)| String::from_utf8_lossy(text).into());
    named.or_else(|| held.map(|text| String::from_utf8_lossy(text).into())).unwrap_or_else(|| unnamed(reason))
}

/// Why `table` is not wholly recorded at a status, by name; none when it is.
pub fn not_recorded(table: TableId, status: Option<Status>, capture: Option<CaptureMode>, reason: Option<&str>) -> Option<String> {
    match status {
        Some(Status::Complete) => None,
        None => Some(format!("{table:?} is not recorded here: a gap in the recording")),
        Some(Status::NotRecorded) => Some(match capture {
            Some(mode) => format!("{table:?} is not recorded in a {mode} recording"),
            None => format!("{table:?} is not recorded at this hour"),
        }),
        Some(Status::Truncated { missing }) => Some(format!("{table:?} is truncated ({missing} missing rows)")),
        Some(Status::Unavailable { .. }) => Some(format!("{table:?} is unavailable: {}", reason.unwrap_or_default())),
    }
}

impl Mirror {
    /// Why the table is unavailable, as the recording words it, while it is.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_ref().map(|(_, text)| &**text)
    }

    /// The row format its cells were written in: an opaque cell decodes at it.
    pub fn format(&self) -> u16 {
        self.format
    }

    /// The recording's capture mode, when the opener knows it.
    pub fn capture(&self) -> Option<CaptureMode> {
        self.capture
    }

    pub fn set_capture(&mut self, capture: Option<CaptureMode>) {
        self.capture = capture;
    }

    /// The last game hour its cells are at.
    /// Whether the last step left it in a gap: no hour of it stands.
    pub fn in_gap(&self) -> bool {
        self.gap
    }

    pub fn head(&self) -> Option<Hour> {
        self.head
    }

    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// [`Self::bytes`] walked cell by cell instead of kept as the cells are written: what the
    /// kept sums must equal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn bytes_walked(&self) -> usize {
        let columns: usize = self
            .columns
            .iter()
            .map(|column| {
                let cells = match &column.cells {
                    Cells::Lists(cells) => cells.capacity() * 24 + cells.iter().map(Vec::capacity).sum::<usize>(),
                    held => held.bytes(),
                };
                cells + column.missing.as_ref().map_or(0, Bits::bytes) + column.unrecorded.as_ref().map_or(0, Bits::bytes)
            })
            .sum();
        self.live.bytes()
            + self.keys.as_ref().map_or(0, |keys| keys.capacity() * 16)
            + self.index.as_ref().map_or(0, |index| index.capacity() * 24)
            + columns
            + self.position.capacity() * 2
            + self.texts.values().map(|text| text.len() + 40).sum::<usize>()
            + self.listed.as_ref().map_or(0, |listed| listed.capacity() * 16)
    }

    pub fn bytes(&self) -> usize {
        self.live.bytes()
            + self.keys.as_ref().map_or(0, |keys| keys.capacity() * 16)
            + self.index.as_ref().map_or(0, |index| index.capacity() * 24)
            + self.columns.iter().map(Column::bytes).sum::<usize>()
            + self.position.capacity() * 2
            + self.texts_bytes
            + self.listed.as_ref().map_or(0, |listed| listed.capacity() * 16)
    }
}

/// Told what each of a step's change sets did to a mirror, so a store of views made from these
/// mirrors knows which of its rows to make again ([`super::views::Noting`]).
pub trait Notes {
    /// A gap step: every table holds nothing until the next step that names it.
    fn gap(&mut self);
    /// `changes` was applied to `table`'s mirror, `moved` when its status moved with it.
    fn applied(&mut self, table: usize, changes: &Changes, moved: bool);
    /// Whether a drawn layer or a maker reads `table`: a mirror no one reads sleeps.
    fn awake(&self, table: usize) -> bool;
}

/// Nothing is noted: a mirror set no view store reads, and none of it sleeps.
impl Notes for () {
    fn gap(&mut self) {}
    fn applied(&mut self, _table: usize, _changes: &Changes, _moved: bool) {}
    fn awake(&self, _table: usize) -> bool {
        true
    }
}

/// What a step's apply cost, split by whether a drawn layer or a maker reads the table.
#[derive(Debug, Default, Clone, Copy)]
pub struct Split {
    pub awake_ns: u64,
    pub awake_sets: u64,
    pub asleep_ns: u64,
    pub asleep_sets: u64,
    pub asleep_cells: u64,
    pub asleep_rows: u64,
}

/// The mirrors of every table the map reads, by table.
#[derive(Debug, Default)]
pub struct Mirrors {
    tables: Vec<Option<Box<Mirror>>>,
    /// The seek of the last step applied ([`Step::epoch`]).
    epoch: u64,
}

impl Mirrors {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn get(&self, table: TableId) -> Option<&Mirror> {
        self.tables.get(table.index()).and_then(Option::as_deref)
    }

    /// Puts `mirror` in its table's place; the one it replaces.
    pub fn insert(&mut self, mirror: Box<Mirror>) -> Option<Box<Mirror>> {
        let at = mirror.table;
        if self.tables.len() <= at {
            self.tables.resize_with(at + 1, || None);
        }
        self.tables[at].replace(mirror)
    }

    /// Takes `table`'s mirror out: no drawn layer declares the table any more.
    pub fn remove(&mut self, table: usize) -> Option<Box<Mirror>> {
        self.tables.get_mut(table).and_then(Option::take)
    }

    /// The step's change sets, each to its table's mirror; a gap step marks every table as not
    /// recorded until the next step that names it.
    pub fn apply(&mut self, step: &Step) {
        self.applying(step, None, &mut ());
    }

    /// [`Self::apply`], each table's wall nanoseconds and the thread's own cycles in it added to
    /// `spent` by table index: a wall time far over the cycles is the thread waiting, not working.
    /// What each change set did is told to `notes`, for a store of views made from these mirrors.
    pub fn apply_timed(&mut self, step: &Step, spent: &mut Vec<(u64, u64)>, split: &mut Split, notes: &mut dyn Notes) {
        self.applying(step, Some((spent, split)), notes);
    }

    fn applying(&mut self, step: &Step, mut spent: Option<(&mut Vec<(u64, u64)>, &mut Split)>, notes: &mut dyn Notes) {
        self.epoch = step.epoch();
        if step.gap() {
            self.tables.iter_mut().flatten().for_each(|mirror| mirror.gap = true);
            notes.gap();
            return;
        }
        // Read once between tables: each runs from the read before.
        let mut mark = replay_frame::cpu::thread_cycles();
        for changes in step.tables() {
            if let Some(Some(mirror)) = self.tables.get_mut(changes.table()) {
                let started = std::time::Instant::now();
                let before = (mirror.status(), mirror.gap, changes.status().and_then(|_| mirror.reason.clone()));
                let awake = notes.awake(changes.table());
                mirror.apply(changes);
                // A gap left behind moves the status as a status written does, and so does a
                // reason spelled anew: each makes every row of a view of this table again.
                let moved = before != (mirror.status(), false, changes.status().and_then(|_| mirror.reason.clone()));
                notes.applied(changes.table(), changes, moved);
                if let Some((spent, split)) = spent.as_mut() {
                    if spent.len() <= changes.table() {
                        spent.resize(changes.table() + 1, (0, 0));
                    }
                    let now = replay_frame::cpu::thread_cycles();
                    let took = started.elapsed().as_nanos() as u64;
                    let held = &mut spent[changes.table()];
                    held.0 += took;
                    held.1 += now.saturating_sub(std::mem::replace(&mut mark, now));
                    if awake {
                        (split.awake_ns, split.awake_sets) = (split.awake_ns + took, split.awake_sets + 1);
                    } else {
                        (split.asleep_ns, split.asleep_sets) = (split.asleep_ns + took, split.asleep_sets + 1);
                        split.asleep_cells += changes.columns().iter().map(|written| written.slots().len() as u64).sum::<u64>();
                        split.asleep_rows += (changes.inserted().slots().len() + changes.deleted().slots().len()) as u64;
                    }
                }
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Mirror> + '_ {
        self.tables.iter().flatten().map(|mirror| &**mirror)
    }

}

/// The map's rebuild: a table's mirror of the map's columns, made from its sought lane on the
/// playhead's fetchers; a log's listed rows read there by ordinal from `pages`.
pub fn rebuild(pages: Arc<dyn Pages>) -> Rebuild {
    Arc::new(move |lane: &Lane, columns: &[usize]| -> io::Result<State> {
        let mut mirror = Mirror::rebuild(lane, columns);
        if let Some(log) = lane.log() {
            let wanted = Wanted { run: lane.run(), table: lane.table(), ordinals: log.live().ones().collect(), columns: columns.to_vec() };
            mirror.put_listed(&playhead::lane::log::rows(&*pages, &wanted)?);
        }
        let bytes = mirror.bytes();
        Ok(State::new(mirror, bytes))
    })
}
