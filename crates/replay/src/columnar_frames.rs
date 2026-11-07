//! A columnar recording's tables as the row decoder's frames.
//!
//! [`State`] holds each table's rows as typed columns at the dense slots the writer assigned,
//! and names the slots each hour changed, deleted and inserted. [`Views`] keeps the selected
//! tables' views as [`crate::frame::FrameDecoder`] reconstructs them from the row recording:
//! a table the hour leaves alone keeps its allocation; a complete table the hour changes gets
//! a new view of the hour before's rows: its inserted rows built from their columns
//! ([`crate::rows::RowCodec::from_wire`]) and its deleted keys removed, then each changed or
//! filled row given the columns the hour wrote ([`crate::rows::RowCodec::patch_from_wire`]);
//! any other gets a view built from the rows the walker holds. A complete view is sampled at
//! the walker's sample hour. Hours can be noted as the walker applies them and the views built
//! only for the hours asked ([`Views::note`], [`Views::build`]): a view built after several
//! hours holds what the hours did since it was built last.
//!
//! The manifest keeps which tables an hour read, not how long each took, so a read table's
//! [`Metrics::table_ns`] is [`READ_TIME_NOT_RECORDED`], as for recordings that keep no timings.

use crate::{
    campaign::Parallel,
    format::FormatVersion,
    frame::{
        DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, RowChange, SharedView, Stamp,
        TableUpdate, TableView, not_selected,
    },
    rows::{DeltaRow, RowChanges, TABLE_COUNT, TableId, TableRows, full_mask},
    scope::CaptureMode,
};
#[cfg(test)]
use replay_columnar::envelope::{Extent, Kind, Root};
#[cfg(any(test, feature = "test-support"))]
use replay_columnar::envelope::{self, IndexPage};
use replay_columnar::{
    manifest::{Manifest, Status},
    section::Changes,
    state::{Kind as Column, Table},
    walk::{Dictionary, State},
};
use std::{
    io,
    mem::size_of,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

mod input;
mod reader;
pub(crate) use reader::{Reads, Spread, Steps};
pub use reader::{Damage, DamageScope, Recording, RootFile, Run, Tally, Timings, Walker};

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The tables whose last complete read every restart holds, its read hour with it, so a walk
/// starting at a restart knows them without the hours before.
pub const LAST_COMPLETE: [TableId; 3] = [
    TableId::Divisions,
    TableId::DivisionEquipmentDeletions,
    TableId::SunkShips,
];

/// [`LAST_COMPLETE`] by table index.
pub fn keeps_last() -> [bool; TABLE_COUNT] {
    let mut keeps = [false; TABLE_COUNT];
    for table in LAST_COMPLETE {
        keeps[table.index()] = true;
    }
    keeps
}

/// Every schema table's column kinds, in the recording's table order.
pub fn schema() -> io::Result<Vec<Vec<Column>>> {
    let mut schema = engine_allocation::vec(TableId::ALL.len())?;
    for table in TableId::ALL {
        schema.push(crate::native_columns::columns(table.schema())?);
    }
    Ok(schema)
}

pub(crate) use crate::frame::wire;

/// Each schema table's columns a recording of row format `format` holds: the fields that
/// format wrote, schema fields being columns one for one, a table whose layout changed since
/// as its earlier layout framed it.
pub fn columns_at(format: u16) -> io::Result<Vec<usize>> {
    engine_allocation::collect_vec(
        TableId::ALL
            .into_iter()
            .map(|table| table.last_change().framed_fields(table, format)),
    )
}

/// The tables of a recording of row format `format` this build reads otherwise, each with why
/// by name: their layout or meaning changed since. Retired tables keep their framing and no
/// reader reads them.
pub fn withheld_at(format: u16) -> io::Result<Vec<(usize, String)>> {
    let mut withheld = Vec::new();
    for table in TableId::ALL {
        let change = table.last_change();
        if change.retired.is_some() {
            continue;
        }
        if let Some(gap) = change.gap(table, format) {
            engine_allocation::reserve(&mut withheld, 1)?;
            withheld.push((table.index(), gap.to_string()));
        }
    }
    Ok(withheld)
}

/// `table`'s rows at `slots`, sorted by key, built from their columns.
fn rows_at(
    id: TableId,
    table: &Table,
    dictionary: &Dictionary,
    slots: &mut Vec<(u128, u32)>,
    keys: &mut Vec<u128>,
    format: u16,
) -> io::Result<TableRows> {
    slots.sort_unstable();
    slots.dedup();
    keys.clear();
    engine_allocation::reserve(keys, slots.len())?;
    keys.extend(slots.iter().map(|&(key, _)| key));
    let slots = &*slots;
    id.rows_from_wire(keys, &mut |row, column| {
        wire(table, dictionary, slots[row].1, column, format)
    })
}

/// An hour's metrics as its manifest keeps them: a read table's time is
/// [`READ_TIME_NOT_RECORDED`].
pub fn metrics(manifest: &Manifest<'_>) -> Metrics {
    let mut table_ns = [0; TABLE_COUNT];
    for (index, time) in table_ns.iter_mut().enumerate() {
        if manifest
            .read
            .get(index / 64)
            .is_some_and(|word| word >> (index % 64) & 1 != 0)
        {
            *time = READ_TIME_NOT_RECORDED;
        }
    }
    Metrics {
        total_ns: manifest.total_ns,
        table_ns,
        pool_bytes: manifest.pool_bytes,
        dropped_frames: manifest.dropped_frames,
    }
}

/// The bytes `view`'s rows, or its reason, hold: what an item keeping a walk's view alive holds
/// once the walk's next build of it copies it.
pub fn view_heap_bytes(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.heap_bytes(),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
    }
}

/// The bytes `frame`'s views hold, each counted though another frame shares it.
pub fn frame_heap_bytes(frame: &DecodedFrame) -> usize {
    frame.tables.iter().map(|view| view_heap_bytes(view)).sum()
}

/// One table's noting of the hours a walk applies to it apart ([`Views::notes`]).
pub struct Note<'v> {
    pending: &'v mut Pending,
    by_slot: bool,
    previous: Option<&'v TableView>,
}

impl Note<'_> {
    /// Notes an hour of `status`, which sectioned the table when `present`, once applied:
    /// `changes` it made to `table`.
    pub fn hour(
        &mut self,
        status: Status,
        present: bool,
        changes: &Changes,
        table: &Table,
    ) -> io::Result<()> {
        if note_status(self.pending, self.by_slot, status, present) {
            note_edits(changes, table, self.pending, self.by_slot, self.previous)?;
        }
        Ok(())
    }
}

/// A complete view of an earlier build, and its edits since: one per key, in key order, to the
/// view built after it ([`Views::spares`]).
struct Spare {
    view: SharedView,
    since: Vec<Noted>,
}

/// Threads of their own that a reader's work runs on in place of rayon's, for work that must
/// not compete with the rest at its priority: each started once, set by `start`.
pub struct Pool(rayon::ThreadPool);

impl Pool {
    /// `threads` threads named `name`, each running `start` first.
    pub fn new(threads: usize, name: &'static str, start: fn()) -> io::Result<Self> {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .thread_name(move |at| format!("{name} {at}"))
            .start_handler(move |_| start())
            .build()
            .map(Self)
            .map_err(io::Error::other)
    }
}

/// `work`, its rayon work on `pool`'s threads when given, else on rayon's.
pub(crate) fn on_pool<R: Send>(pool: Option<&Pool>, work: impl FnOnce() -> R + Send) -> R {
    match pool {
        Some(pool) => pool.0.install(work),
        None => work(),
    }
}

/// One table's hours noted since its view was built ([`Views::note`]).
#[derive(Default)]
struct Pending {
    hours: u32,
    /// The first noted hour's status, and whether a later noted hour had another.
    status: Option<Status>,
    varied: bool,
    /// Some noted hour had a section of the table.
    sectioned: bool,
    /// The view is built anew, whatever the hours noted.
    anew: bool,
    /// Each noted section's edits, in hour order, while the table stays complete.
    edits: Vec<Noted>,
    /// How many edits the last compaction left.
    compacted: usize,
    /// The edits by slot instead, for views noted by slot ([`Views::by_slot`]).
    marks: Marks,
}

/// A table's edits since its view was built, kept by slot: what each slot's row got and the
/// keys deleted, so an hour costs a bit and a mask per changed row and nothing is sorted
/// until the view is built ([`Views::by_slot`]).
#[derive(Default)]
struct Marks {
    /// The slots written or inserted since, a bit each.
    touched: Vec<u64>,
    /// The slots inserted since, a bit each.
    inserted: Vec<u64>,
    /// The columns written to each slot since, by slot.
    masks: Vec<u64>,
    /// The keys deleted since, some perhaps more than once.
    deleted: Vec<u128>,
    /// How many keys `deleted` held when last deduplicated.
    distinct: usize,
    /// Some slot or key is marked.
    any: bool,
}

/// Sets `slot`'s bit in `bits`.
fn mark(bits: &mut Vec<u64>, slot: u32) -> io::Result<()> {
    let word = slot as usize / 64;
    if word >= bits.len() {
        grow(bits, word + 1)?;
    }
    bits[word] |= 1_u64 << (slot % 64);
    Ok(())
}

/// `values` grown with zeros to `len`.
fn grow(values: &mut Vec<u64>, len: usize) -> io::Result<()> {
    engine_allocation::reserve(values, len.saturating_sub(values.len()))?;
    values.resize(len.max(values.len()), 0);
    Ok(())
}

impl Marks {
    /// Marks what `changes`, one hour's section of `table`, did.
    fn note(&mut self, changes: &Changes, table: &Table) -> io::Result<()> {
        engine_allocation::reserve(&mut self.deleted, changes.deleted_keys.len())?;
        self.deleted.extend_from_slice(&changes.deleted_keys);
        self.any |= !changes.deleted_keys.is_empty();
        for &slot in &changes.inserted {
            mark(&mut self.touched, slot)?;
            mark(&mut self.inserted, slot)?;
            self.any = true;
        }
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
        for (&slot, &mask) in written.filter(|(_, mask)| **mask != 0) {
            mark(&mut self.touched, slot)?;
            let at = slot as usize;
            if at >= self.masks.len() {
                grow(&mut self.masks, table.slots().max(at + 1))?;
            }
            self.masks[at] |= mask;
            self.any = true;
        }
        // Deleted keys stay within the table's slots and twice what the last deduplication left.
        if self.deleted.len() > table.slots().max(2 * self.distinct) {
            self.deleted.sort_unstable();
            self.deleted.dedup();
            self.distinct = self.deleted.len();
        }
        Ok(())
    }

    /// Each marked row's edit, of `table` as it is now, after `edits`, unsorted; the marks
    /// cleared. A live marked slot was inserted (its row set whole) or written; a deleted key
    /// live again was inserted since, so only those gone are deleted.
    fn edits(&mut self, table: &Table, edits: &mut Vec<Noted>) -> io::Result<()> {
        if !self.any {
            return Ok(());
        }
        // Room for every edit before any mark is taken: a refusal leaves the marks as they were.
        let most = self
            .touched
            .iter()
            .map(|bits| bits.count_ones() as usize)
            .sum::<usize>()
            + self.deleted.len();
        if edits.len() + most >= PLACES as usize {
            return Err(invalid("A table's edits pass the places they can hold"));
        }
        engine_allocation::reserve(edits, most)?;
        self.any = false;
        let place = |edits: &Vec<Noted>| u32::try_from(edits.len()).unwrap_or(PLACES);
        for (word, bits) in self.touched.iter_mut().enumerate() {
            let mut bits = std::mem::take(bits);
            let inserted = self.inserted.get_mut(word).map_or(0, std::mem::take);
            while bits != 0 {
                let bit = bits.trailing_zeros();
                bits &= bits - 1;
                let slot = u32::try_from(word * 64 + bit as usize)
                    .map_err(|_| invalid("A marked slot passes the slots a table holds"))?;
                let mask = self.masks.get_mut(slot as usize).map_or(0, std::mem::take);
                let Some(key) = table.key(slot) else {
                    continue;
                };
                let edit = if inserted >> bit & 1 != 0 {
                    Edit::Inserted(slot)
                } else {
                    Edit::Written(mask, slot)
                };
                let at = place(edits);
                edits.push(Noted::new(key, at, edit)?);
            }
        }
        self.deleted.sort_unstable();
        self.deleted.dedup();
        for &key in &self.deleted {
            if table.slot(key).is_none() {
                let at = place(edits);
                edits.push(Noted::new(key, at, Edit::Deleted)?);
            }
        }
        self.deleted.clear();
        self.distinct = 0;
        Ok(())
    }

    /// The marks cleared, their buffers kept.
    fn emptied(mut self) -> Self {
        if std::mem::take(&mut self.any) {
            for (word, bits) in self.touched.iter_mut().enumerate() {
                let mut bits = std::mem::take(bits);
                while bits != 0 {
                    let slot = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if let Some(mask) = self.masks.get_mut(slot) {
                        *mask = 0;
                    }
                }
            }
            self.inserted.fill(0);
            self.deleted.clear();
            self.distinct = 0;
        }
        self
    }

    fn heap_bytes(&self) -> usize {
        (self.touched.capacity() + self.inserted.capacity() + self.masks.capacity())
            * size_of::<u64>()
            + self.deleted.capacity() * size_of::<u128>()
    }
}

/// What a section did to a row, with the slot the row is at for an edit that leaves one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Deleted,
    /// Inserted, every column with it.
    Inserted(u32),
    /// These columns written.
    Written(u64, u32),
}

/// A noted edit of the row keyed `key`, and its place among its table's edits, by which a
/// sort keeps each key's in hour order without a scratch buffer.
#[derive(Clone, Copy, Debug)]
struct Noted {
    key: u128,
    /// The columns written; 0 for a deletion or an insertion.
    mask: u64,
    slot: u32,
    /// Its place, below [`PLACES`], with its kind in the bits above.
    place: u32,
}

const _: () = assert!(size_of::<Noted>() == 32);

/// The places a table's edits can hold.
const PLACES: u32 = 1 << 30;
const DELETED: u32 = 1 << 30;
const INSERTED: u32 = 2 << 30;
const WRITTEN: u32 = 3 << 30;

impl Noted {
    fn new(key: u128, place: u32, edit: Edit) -> io::Result<Self> {
        if place >= PLACES {
            return Err(invalid("A table's edits pass the places they can hold"));
        }
        let (mask, slot, kind) = match edit {
            Edit::Deleted => (0, 0, DELETED),
            Edit::Inserted(slot) => (0, slot, INSERTED),
            Edit::Written(mask, slot) => (mask, slot, WRITTEN),
        };
        Ok(Self {
            key,
            mask,
            slot,
            place: place | kind,
        })
    }

    fn edit(self) -> Edit {
        match self.place & !(PLACES - 1) {
            DELETED => Edit::Deleted,
            INSERTED => Edit::Inserted(self.slot),
            _ => Edit::Written(self.mask, self.slot),
        }
    }

    /// Its order: by key, then by place.
    fn order(&self) -> (u128, u32) {
        (self.key, self.place & (PLACES - 1))
    }
}

/// The slot `key` is at in `table`: the slot its last edit named, else found again.
fn held_slot(table: &Table, key: u128, slot: u32) -> Option<u32> {
    if table.key(slot) == Some(key) {
        Some(slot)
    } else {
        table.slot(key)
    }
}

/// `edits` sorted by key, each key's in hour order, without a scratch buffer.
fn sort(edits: &mut [Noted]) {
    edits.sort_unstable_by_key(Noted::order);
}

/// `edits`, sorted by key and in hour order within one ([`sort`]), as what they leave each
/// key: set whole (inserted since its last deletion), removed, or with the columns written
/// since.
fn fold(edits: &[Noted], mut each: impl FnMut(u128, Edit) -> io::Result<()>) -> io::Result<()> {
    let mut at = 0;
    while let Some(&Noted { key, .. }) = edits.get(at) {
        let (mut deleted, mut inserted, mut mask, mut slot) = (false, false, 0, 0);
        while let Some(noted) = edits.get(at).filter(|noted| noted.key == key) {
            match noted.edit() {
                Edit::Deleted => {
                    deleted = true;
                    inserted = false;
                    mask = 0;
                }
                Edit::Inserted(held) => {
                    inserted = true;
                    mask = 0;
                    slot = held;
                }
                Edit::Written(columns, held) => {
                    mask |= columns;
                    slot = held;
                }
            }
            at += 1;
        }
        if inserted {
            each(key, Edit::Inserted(slot))?;
        } else if deleted {
            each(key, Edit::Deleted)?;
        } else if mask != 0 {
            each(key, Edit::Written(mask, slot))?;
        }
    }
    Ok(())
}

/// `edits` as [`fold`] leaves them, one per key, in key order, less deletions of keys
/// `previous` does not hold: they fold the same with any edits after them. In place: each
/// key's fold is written over its first edits.
fn compact(edits: &mut Vec<Noted>, previous: &TableRows) -> io::Result<()> {
    sort(edits);
    let mut kept = 0;
    let mut read = 0;
    while read < edits.len() {
        let key = edits[read].key;
        let end = read + edits[read..].partition_point(|noted| noted.key == key);
        let mut folded = None;
        fold(&edits[read..end], |key, edit| {
            folded = Some((key, edit));
            Ok(())
        })?;
        if let Some((key, edit)) = folded
            && (edit != Edit::Deleted || previous.contains_key(key))
        {
            // `kept` stays below `read`, an index of a placed edit.
            edits[kept] = Noted::new(key, kept as u32, edit)?;
            kept += 1;
        }
        read = end;
    }
    edits.truncate(kept);
    Ok(())
}

/// What [`Views::build`] took for each kind of view, when its builds are timed.
#[derive(Clone, Debug, Default)]
pub struct BuildTimes {
    /// Tables whose views were carried, patched, built anew and given their status, and the
    /// time they took; tables built anew share one parallel pass.
    pub carried: (u32, Duration),
    pub patched: (u32, Duration),
    pub anew: (u32, Duration),
    pub status: (u32, Duration),
    /// Each selected table's time, in selection order, the parallel pass aside.
    pub tables: Vec<(TableId, Duration)>,
}

/// What [`Views::build`] makes of a selected table.
#[derive(Clone, Copy)]
enum Plan {
    /// Its view's allocation, sampled anew.
    Carry,
    /// Its complete view with its folded edits: `changed` rows and `deleted` keys.
    Patch { changed: usize, deleted: usize },
    /// A complete view of every row the walker holds.
    Anew,
    /// A view of its status other than complete.
    Status,
    /// Unavailable, as the walk withholds it ([`State::withheld`]).
    Withheld,
}

/// `mask`'s fields of `id` and the fields a format of `recorded` fields leaves unrecorded, as
/// a decoder's change facts count them.
fn change_fields(id: TableId, mask: u64, recorded: usize) -> u64 {
    let missing = if recorded >= u64::BITS as usize {
        0
    } else {
        u64::MAX << recorded
    };
    mask | (id.full_mask() & missing)
}

/// The selected tables' views, carried from hour to hour as the walker advances. Hours are
/// noted as the walker applies them ([`Self::note`]) and views built only when a frame is
/// asked for ([`Self::build`]), from every hour noted since the views were built last.
pub struct Views {
    selected: Vec<TableId>,
    views: Vec<Option<SharedView>>,
    /// By table, a view of an earlier build a frame may still hold, to patch in place once
    /// none does instead of copying the view the frame of the build after holds.
    spares: Vec<Option<Spare>>,
    /// The threads builds work on, rayon's when none ([`Self::set_pool`]).
    pool: Option<Arc<Pool>>,
    /// The format opaque cells were encoded at.
    format: u16,
    mode: CaptureMode,
    slots: Vec<(u128, u32)>,
    keys: Vec<u128>,
    deleted: Vec<u128>,
    /// Each patched row's key, slot and the columns written to it.
    patches: Vec<(u128, u32, u64)>,
    masks: Vec<(u128, u64)>,
    union: Vec<Noted>,
    /// Whether [`Self::advance_recorded`] asked for the hour's change facts, and each selected
    /// table's, in selection order.
    record: bool,
    facts: Vec<TableUpdate>,
    /// Each selected table's hours noted since its view was built, in selection order.
    pending: Vec<Pending>,
    /// The hour the views were built for last.
    built: Option<i32>,
    /// Whether complete views carry change facts and field stamps.
    changes: bool,
    /// The last build's times, when builds are timed.
    timed: bool,
    times: BuildTimes,
    /// Hours are noted by slot ([`Self::by_slot`]).
    by_slot: bool,
    /// Each hour's tables are noted apart, on rayon's threads ([`Self::apart`]).
    apart: bool,
    /// The positions of the tables an hour edits, kept between hours.
    edited: Vec<usize>,
}

impl Views {
    /// The bytes its views and buffers hold, the edits of the hours noted among them.
    pub fn heap_bytes(&self) -> usize {
        self.views
            .iter()
            .flatten()
            .map(|view| view_heap_bytes(view))
            .sum::<usize>()
            + self.slots.capacity() * size_of::<(u128, u32)>()
            + (self.keys.capacity() + self.deleted.capacity()) * size_of::<u128>()
            + self.patches.capacity() * size_of::<(u128, u32, u64)>()
            + self.masks.capacity() * size_of::<(u128, u64)>()
            + self.facts.capacity() * size_of::<TableUpdate>()
            + self.pending.capacity() * size_of::<Pending>()
            + self
                .pending
                .iter()
                .map(|pending| {
                    pending.edits.capacity() * size_of::<Noted>() + pending.marks.heap_bytes()
                })
                .sum::<usize>()
            + self.times.tables.capacity() * size_of::<(TableId, Duration)>()
    }

    /// Views of `tables` and Campaign, as a decoder of them reconstructs them.
    pub fn new(tables: &[TableId], format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for table in tables {
            chosen[table.index()] = true;
        }
        let selected = engine_allocation::collect_vec(
            TableId::ALL
                .into_iter()
                .filter(|table| chosen[table.index()]),
        )?;
        Ok(Self {
            pending: engine_allocation::collect_vec(selected.iter().map(|_| Pending::default()))?,
            selected,
            views: engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?,
            spares: engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?,
            pool: None,
            format: format.version(),
            mode,
            slots: Vec::new(),
            keys: Vec::new(),
            deleted: Vec::new(),
            patches: Vec::new(),
            masks: Vec::new(),
            union: Vec::new(),
            record: false,
            facts: Vec::new(),
            built: None,
            changes: false,
            timed: false,
            times: BuildTimes::default(),
            by_slot: false,
            apart: false,
            edited: Vec::new(),
        })
    }

    /// Views whose hours are noted by slot: a bit and the columns written per changed row, and
    /// the keys deleted, sorted only when a view is built. For views built rarely of tables
    /// changing often, whose edits would otherwise be sorted again and again as they pile up.
    pub fn by_slot(mut self) -> Self {
        self.by_slot = true;
        self
    }

    /// Views noting each hour's tables apart, on rayon's threads, and building their patches
    /// apart: for a walker that has the threads to itself, as a playback does.
    pub fn apart(mut self) -> Self {
        self.apart = true;
        self
    }

    /// Complete views carrying change facts and field stamps, as a decoder with change facts
    /// makes them.
    pub fn with_change_facts(mut self) -> Self {
        self.changes = true;
        self
    }

    /// Builds work on `pool`'s threads from now on, or on rayon's.
    pub fn set_pool(&mut self, pool: Option<Arc<Pool>>) {
        self.pool = pool;
    }

    /// The tables viewed, Campaign among them, in table order.
    pub fn selected(&self) -> &[TableId] {
        &self.selected
    }

    /// Times each build from now on, or stops.
    pub fn time_builds(&mut self, timed: bool) {
        self.timed = timed;
    }

    /// What the last build took, when builds are timed.
    pub fn times(&self) -> &BuildTimes {
        &self.times
    }

    /// The hour the views were built for last; none before the first build or after a
    /// [`Self::reset`].
    pub fn built(&self) -> Option<i32> {
        self.built
    }

    /// Forgets the views and the hours noted: the next views are built anew, as at a restart.
    pub fn reset(&mut self) {
        self.spares.fill_with(|| None);
        for pending in &mut self.pending {
            pending.edits.clear();
            *pending = Pending {
                anew: true,
                edits: std::mem::take(&mut pending.edits),
                marks: std::mem::take(&mut pending.marks).emptied(),
                ..Pending::default()
            };
        }
        self.built = None;
    }

    /// [`Self::advance`], with the hour's change facts in the row recording's shape: a table
    /// left alone is Unchanged, one the hour changed a Delta whose rows carry their keys and
    /// the columns the hour wrote (an inserted row every column) but no values, which readers
    /// take from the frame; a view built anew is a Snapshot of its rows. Tables not selected
    /// are unavailable, as in the row recording's selected reads.
    pub fn advance_recorded(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<(DecodedFrame, RecordedFrame)> {
        self.record = true;
        self.facts.clear();
        engine_allocation::reserve(&mut self.facts, self.selected.len())?;
        let frame = self.advance(state, manifest, restart);
        self.record = false;
        let frame = frame?;
        let mut slots = [None; TABLE_COUNT];
        for (at, table) in self.selected.iter().enumerate() {
            slots[table.index()] = Some(at);
        }
        let updates = engine_allocation::collect_vec(self.facts.drain(..).map(Arc::new))?;
        let recorded = RecordedFrame::from_updates(
            frame.branch,
            frame.hour,
            restart,
            frame.metrics.clone(),
            Arc::new(slots),
            updates,
            Arc::new(TableUpdate::Unavailable {
                reason: String::new(),
            }),
        )?;
        Ok((frame, recorded))
    }

    /// Keeps `update` as the hour's fact for the table being viewed, when facts are asked.
    fn fact(&mut self, update: impl FnOnce() -> io::Result<TableUpdate>) -> io::Result<()> {
        if self.record {
            engine_allocation::reserve(&mut self.facts, 1)?;
            self.facts.push(update()?);
        }
        Ok(())
    }

    /// The Delta fact of a changed complete table: its inserted rows (every column), then
    /// its patched rows, in key order, and its deleted keys.
    fn delta(&self, id: TableId, baseline: i32) -> io::Result<TableUpdate> {
        // The fields the recording's format wrote, as its own deltas count them.
        let fields = id.last_change().fields(id, self.format);
        let row = |key: u128, mask: u64| DeltaRow {
            format: self.format,
            key,
            mask,
            recorded_fields: fields,
            values: Vec::new(),
        };
        let mut changed = engine_allocation::vec(self.keys.len() + self.patches.len())?;
        changed.extend(self.keys.iter().map(|&key| row(key, full_mask(fields))));
        changed.extend(self.patches.iter().map(|&(key, _, mask)| row(key, mask)));
        changed.sort_unstable_by_key(|row: &DeltaRow| row.key);
        Ok(TableUpdate::Delta {
            baseline,
            changed,
            deleted: engine_allocation::copy_vec(&self.deleted)?,
        })
    }

    /// The frame of `manifest`'s hour, once `state` applied it. A `restart` (a unit the walker
    /// started at, or a keyframe) has no hour before: every view is built anew.
    pub fn advance(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<DecodedFrame> {
        self.note(state, manifest, restart)?;
        self.build(
            state,
            (manifest.branch, manifest.hour),
            metrics(manifest),
            None,
        )
    }

    /// The views as built for the hour they were built for last, dated `branch`: the hour
    /// before's while the walker's next hour is not yet applied to them; none before a build.
    /// Once the frame is dropped the views are unshared again, so the next build changes them
    /// in place.
    pub fn frame(&self, branch: u64) -> Option<DecodedFrame> {
        let hour = self.built?;
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
        for table in &self.selected {
            tables[table.index()] = self.views[table.index()].clone()?;
        }
        Some(DecodedFrame {
            branch,
            hour,
            tables,
            metrics: Metrics::default(),
        })
    }

    /// Takes up the selected tables' views of `frame`, a frame [`Self::frame`] gave, as built
    /// for its hour, in place of those held: the next build goes on from them as it would
    /// have from the views that frame was taken of.
    pub fn load(&mut self, frame: &DecodedFrame) {
        self.spares.fill_with(|| None);
        for pending in &mut self.pending {
            pending.edits.clear();
            *pending = Pending {
                edits: std::mem::take(&mut pending.edits),
                marks: std::mem::take(&mut pending.marks).emptied(),
                ..Pending::default()
            };
        }
        for table in &self.selected {
            self.views[table.index()] = Some(frame.tables[table.index()].clone());
        }
        self.built = Some(frame.hour);
    }

    /// The frame of `state` as a restart restored it, before the walk's first hour, dated
    /// `hour` of `branch`: every view is built anew, and no table reads as read.
    pub fn restored(&mut self, state: &State, branch: u64, hour: i32) -> io::Result<DecodedFrame> {
        self.reset();
        self.build(state, (branch, hour), Metrics::default(), None)
    }

    /// Takes in `manifest`'s hour, once `state` applied it, building no view: the next views
    /// built follow from every hour noted since the last. A `restart` (a unit the walker
    /// started at, or a keyframe) has no hour before: every view is built anew.
    pub fn note(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<()> {
        self.note_tables(state, manifest, restart, &|_| true)
    }

    /// [`Self::note`] for the tables `noted` names alone: the others see no hour, and are built
    /// from the hours they saw, as views noted only at those hours would be.
    pub fn note_tables(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
        noted: &dyn Fn(TableId) -> bool,
    ) -> io::Result<()> {
        let mut present = [false; TABLE_COUNT];
        for section in &manifest.sections {
            *present
                .get_mut(section.table)
                .ok_or_else(|| invalid("A section names a table past the schema"))? = true;
        }
        if restart {
            self.reset();
        }
        // The tables whose edits the hour adds to, found first; their edits made after.
        let mut edited = std::mem::take(&mut self.edited);
        edited.clear();
        for (at, pending) in self.pending.iter_mut().enumerate() {
            let id = self.selected[at];
            if !noted(id) {
                continue;
            }
            let index = id.index();
            if state.withheld(index).is_some() {
                continue;
            }
            if state.skips(index) {
                return Err(invalid(format!("{id:?} is not read by this walk")));
            }
            let status = state
                .status(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            if note_status(pending, self.by_slot, status, present[index]) {
                engine_allocation::reserve(&mut edited, 1)?;
                edited.push(at);
            }
        }
        let (by_slot, selected, views) = (self.by_slot, &self.selected, &self.views);
        let edit = |at: usize, pending: &mut Pending| {
            let index = selected[at].index();
            let changes = state
                .changes(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            let table = state
                .table(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            note_edits(changes, table, pending, by_slot, views[index].as_deref())
        };
        let noted = if self.apart && edited.len() > 1 {
            use rayon::prelude::*;
            self.pending
                .par_iter_mut()
                .enumerate()
                .filter(|(at, _)| edited.binary_search(at).is_ok())
                .try_for_each(|(at, pending)| edit(at, pending))
        } else {
            edited
                .iter()
                .try_for_each(|&at| edit(at, &mut self.pending[at]))
        };
        self.edited = edited;
        noted
    }

    /// Each selected table's noter, by table index, for a walk noting its hours table by table
    /// ([`Self::note`] of each hour, as `state` stands before them); none for a table withheld.
    pub fn notes(&mut self, state: &State) -> io::Result<Vec<Option<Note<'_>>>> {
        let mut notes = engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        let (by_slot, views) = (self.by_slot, &self.views);
        for (pending, id) in self.pending.iter_mut().zip(&self.selected) {
            let index = id.index();
            if state.withheld(index).is_some() {
                continue;
            }
            if state.skips(index) {
                return Err(invalid(format!("{id:?} is not read by this walk")));
            }
            notes[index] = Some(Note {
                pending,
                by_slot,
                previous: views[index].as_deref(),
            });
        }
        Ok(notes)
    }

    /// The frame of `hour` of `branch` from every hour noted since the views were built last,
    /// as [`Self::update`] builds the views.
    pub fn build(
        &mut self,
        state: &State,
        (branch, hour): (u64, i32),
        metrics: Metrics,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<DecodedFrame> {
        self.update(state, hour, parallel)?;
        let mut frame = self
            .frame(branch)
            .ok_or_else(|| invalid("A built table has no view"))?;
        frame.metrics = metrics;
        Ok(frame)
    }

    /// `table`'s view as built last; None before a build, or when it is not selected.
    pub fn view(&self, table: TableId) -> Option<&SharedView> {
        self.views[table.index()].as_ref()
    }

    /// Builds the views of `hour` from every hour noted since they were built last, read with
    /// [`Self::view`]: a table no noted hour touched keeps its allocation; a complete table
    /// that stayed complete gets the noted sections' edits, its inserted rows built from their
    /// columns, its deleted keys removed and each written row given the columns written since;
    /// any other gets a view built anew from the rows the walker holds, complete ones on
    /// `parallel`'s workers when given.
    pub fn update(
        &mut self,
        state: &State,
        hour: i32,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<()> {
        let every = engine_allocation::collect_vec(0..self.selected.len())?;
        self.update_at(state, hour, &every, parallel)
    }

    /// [`Self::update`] of `tables` alone, each selected: the others keep their views and the
    /// hours noted since, for a later build. Views that record or carry change facts build every
    /// table at once.
    pub fn update_tables(
        &mut self,
        state: &State,
        hour: i32,
        tables: &[TableId],
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<()> {
        if self.record || self.changes {
            return Err(invalid("Views with change facts build every table at once"));
        }
        let mut at = engine_allocation::vec(tables.len())?;
        for table in tables {
            at.push(
                self.selected
                    .binary_search(table)
                    .map_err(|_| invalid("A table built is not selected"))?,
            );
        }
        at.sort_unstable();
        at.dedup();
        self.update_at(state, hour, &at, parallel)
    }

    /// [`Self::update`] of the selected tables at `positions`, ascending.
    fn update_at(
        &mut self,
        state: &State,
        hour: i32,
        positions: &[usize],
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<()> {
        let mut plans = engine_allocation::vec(positions.len())?;
        let by_slot = self.by_slot;
        for &at in positions {
            let pending = &mut self.pending[at];
            let id = self.selected[at];
            let index = id.index();
            if state.withheld(index).is_some() {
                plans.push(Plan::Withheld);
                continue;
            }
            if state.skips(index) {
                return Err(invalid(format!("{id:?} is not read by this walk")));
            }
            let status = state
                .status(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            let constant =
                pending.hours == 0 || (!pending.varied && pending.status == Some(status));
            let previous = self.views[index]
                .as_ref()
                .filter(|_| !pending.anew && constant);
            let plan = match (status, previous.map(|view| &**view)) {
                (Status::Complete, Some(TableView::Complete(rows))) => {
                    if pending.sectioned {
                        if by_slot {
                            let table = state
                                .table(index)
                                .ok_or_else(|| invalid("The walker holds no such table"))?;
                            pending.marks.edits(table, &mut pending.edits)?;
                        }
                        sort(&mut pending.edits);
                        let (mut changed, mut deleted) = (0, 0);
                        fold(&pending.edits, |key, edit| {
                            match edit {
                                Edit::Deleted => deleted += usize::from(rows.contains_key(key)),
                                Edit::Inserted(_) | Edit::Written(..) => changed += 1,
                            }
                            Ok(())
                        })?;
                        // Marks name every row changed since, so none changed keeps its view.
                        if by_slot && changed == 0 && deleted == 0 {
                            Plan::Carry
                        } else {
                            Plan::Patch { changed, deleted }
                        }
                    } else {
                        Plan::Carry
                    }
                }
                (Status::Complete, _) => Plan::Anew,
                _ => Plan::Status,
            };
            plans.push(plan);
        }
        if self.changes {
            // Facts a replaced view no longer needs go before the new ones are made.
            for (plan, &at) in plans.iter().zip(positions) {
                if !matches!(plan, Plan::Carry)
                    && let Some(view) = self.views[self.selected[at].index()].as_mut()
                {
                    view.reclaim_changes();
                }
            }
        }
        let anew = engine_allocation::collect_vec(
            plans
                .iter()
                .zip(positions)
                .filter(|(plan, _)| matches!(plan, Plan::Anew))
                .map(|(_, &at)| self.selected[at]),
        )?;
        if self.timed {
            self.times.tables.clear();
            engine_allocation::reserve(&mut self.times.tables, positions.len())?;
            let tables = std::mem::take(&mut self.times.tables);
            self.times = BuildTimes {
                tables,
                ..BuildTimes::default()
            };
        }
        let started = self.timed.then(Instant::now);
        let mut anew = self.rows_anew(state, &anew, parallel)?.into_iter();
        if let Some(started) = started {
            self.times.anew.1 = started.elapsed();
        }
        let mut made = self.patch_apart(state, &plans, positions, hour, parallel)?;
        // A continuing walk's deltas apply to the hour the views were built for.
        let baseline = self.built.unwrap_or(hour.saturating_sub(1));
        for (place, (plan, &at)) in plans.into_iter().zip(positions).enumerate() {
            let id = self.selected[at];
            let index = id.index();
            let started = self.timed.then(Instant::now);
            let mut pending = std::mem::take(&mut self.pending[at]);
            let view = match made[place].take() {
                Some(view) => view,
                None => {
                    let previous = self.views[index].take().filter(|_| !pending.anew);
                    self.make(
                        state,
                        id,
                        plan,
                        &pending,
                        previous,
                        &mut anew,
                        (baseline, hour),
                    )
                }
            };
            pending.edits.clear();
            self.pending[at] = Pending {
                edits: pending.edits,
                marks: pending.marks.emptied(),
                ..Pending::default()
            };
            self.views[index] = Some(view?);
            if let Some(started) = started {
                let took = started.elapsed();
                let tally = match plan {
                    Plan::Carry => &mut self.times.carried,
                    Plan::Patch { .. } => &mut self.times.patched,
                    Plan::Anew => &mut self.times.anew,
                    Plan::Status | Plan::Withheld => &mut self.times.status,
                };
                tally.0 += 1;
                tally.1 += took;
                self.times.tables.push((id, took));
            }
        }
        self.built = Some(hour);
        Ok(())
    }

    /// The views of the tables `plans` patch, at `positions`, made on rayon's threads, at most
    /// `parallel`'s workers at once, the most edited first, each on one; none for the others, or when facts are recorded,
    /// whose order the serial build keeps, or when fewer than two tables are patched.
    fn patch_apart(
        &mut self,
        state: &State,
        plans: &[Plan],
        positions: &[usize],
        hour: i32,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<Option<io::Result<SharedView>>>> {
        let mut made = engine_allocation::collect_vec(plans.iter().map(|_| None))?;
        let jobs = engine_allocation::collect_vec(
            (0..plans.len()).filter(|&place| matches!(plans[place], Plan::Patch { .. })),
        )?;
        let workers = parallel.map_or(1, |parallel| parallel.workers.min(jobs.len()));
        if workers < 2 || self.record {
            return Ok(made);
        }
        let mut previous = engine_allocation::vec(jobs.len())?;
        for &place in &jobs {
            let index = self.selected[positions[place]].index();
            previous.push(Mutex::new((self.views[index].take(), self.spares[index].take())));
        }
        let mut order = engine_allocation::collect_vec(0..jobs.len())?;
        order.sort_by_key(|&job| {
            std::cmp::Reverse(match plans[jobs[job]] {
                Plan::Patch { changed, deleted } => changed + deleted,
                _ => 0,
            })
        });
        let results = engine_allocation::collect_vec(
            (0..jobs.len()).map(|_| Mutex::new(None::<io::Result<(SharedView, Option<Spare>)>>)),
        )?;
        let next = AtomicUsize::new(0);
        let (format, changes) = (self.format, self.changes);
        let (selected, pending, pool) = (&self.selected, &self.pending, &self.pool);
        let (label, context) = (engine_allocation::labels::current(), job_priority::context());
        // The pool's threads, which the walk works on too: no thread is started per build.
        on_pool(pool.as_deref(), || rayon::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|_| job_priority::with_context(context, || {
                    let _memory = engine_allocation::labels::resume(label);
                    let mut scratch = PatchScratch::default();
                    while let Some(&job) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let (place, at) = (jobs[job], positions[jobs[job]]);
                        let id = selected[at];
                        let made = (|| {
                            let Plan::Patch { changed, deleted } = plans[place] else {
                                return Err(invalid("A table patched apart is not patched"));
                            };
                            let (previous, spare) = std::mem::take(
                                &mut *previous[job].lock().unwrap_or_else(PoisonError::into_inner),
                            );
                            let previous =
                                previous.ok_or_else(|| invalid("A patched table has no view"))?;
                            let table = state
                                .table(id.index())
                                .ok_or_else(|| invalid("The walker holds no such table"))?;
                            patch_table(
                                (id, table, state.dictionary()),
                                previous,
                                spare,
                                true,
                                &pending[at].edits,
                                (changed, deleted),
                                &mut scratch,
                                (format, changes),
                                (hour, state.sampled_at(id.index())),
                            )
                        })();
                        *results[job].lock().unwrap_or_else(PoisonError::into_inner) = Some(made);
                    }
                }));
            }
        }));
        for (job, result) in results.into_iter().enumerate() {
            let result = result
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or_else(|| Err(invalid("A worker left a table unpatched")));
            made[jobs[job]] = Some(result.map(|(view, spare)| {
                self.spares[self.selected[positions[jobs[job]]].index()] = spare;
                view
            }));
        }
        Ok(made)
    }

    /// Every row the walker holds of each of `tables`, built from its columns: on rayon's
    /// threads, at most `parallel`'s workers at once, when given, the largest tables first,
    /// each table on one.
    fn rows_anew(
        &mut self,
        state: &State,
        tables: &[TableId],
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<TableRows>> {
        let format = self.format;
        let build = |id: TableId,
                     slots: &mut Vec<(u128, u32)>,
                     keys: &mut Vec<u128>|
         -> io::Result<TableRows> {
            let table = state
                .table(id.index())
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            slots.clear();
            engine_allocation::reserve(slots, table.live().count())?;
            for slot in table.live().ones() {
                let key = table
                    .key(slot)
                    .ok_or_else(|| invalid("A live slot holds no key"))?;
                slots.push((key, slot));
            }
            rows_at(id, table, state.dictionary(), slots, keys, format)
        };
        let workers = parallel.map_or(1, |parallel| parallel.workers.min(tables.len()));
        if workers < 2 {
            let (slots, keys) = (&mut self.slots, &mut self.keys);
            return tables.iter().map(|&id| build(id, slots, keys)).collect();
        }
        let mut order = engine_allocation::collect_vec(0..tables.len())?;
        order.sort_by_key(|&at| {
            std::cmp::Reverse(
                state
                    .table(tables[at].index())
                    .map_or(0, |table| table.live().count()),
            )
        });
        let next = AtomicUsize::new(0);
        let pool = &self.pool;
        let built = engine_allocation::collect_vec(
            (0..tables.len()).map(|_| Mutex::new(None::<io::Result<TableRows>>)),
        )?;
        let (label, context) = (engine_allocation::labels::current(), job_priority::context());
        // The pool's threads, which the walk works on too: no thread is started per build.
        on_pool(pool.as_deref(), || rayon::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|_| job_priority::with_context(context, || {
                    let _memory = engine_allocation::labels::resume(label);
                    let (mut slots, mut keys) = (Vec::new(), Vec::new());
                    while let Some(&at) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let rows = build(tables[at], &mut slots, &mut keys);
                        *built[at].lock().unwrap_or_else(PoisonError::into_inner) = Some(rows);
                    }
                }));
            }
        }));
        built
            .into_iter()
            .map(|cell| {
                cell.into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .unwrap_or_else(|| Err(invalid("A worker left a table unbuilt")))
            })
            .collect()
    }

    /// Table `id`'s view as `plan` makes it from `previous`, its view as built last unless it
    /// is built anew, and `pending`, its hours noted since; `anew` holds the rows of the
    /// tables built anew, in selection order.
    #[allow(clippy::too_many_arguments)]
    fn make(
        &mut self,
        state: &State,
        id: TableId,
        plan: Plan,
        pending: &Pending,
        previous: Option<SharedView>,
        anew: &mut impl Iterator<Item = TableRows>,
        (baseline, hour): (i32, i32),
    ) -> io::Result<SharedView> {
        let index = id.index();
        let status = state
            .status(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let sampled = state.sampled_at(index);
        let table = state
            .table(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let dictionary = state.dictionary();
        if !matches!(plan, Plan::Carry | Plan::Patch { .. }) {
            self.spares[index] = None;
        }
        match plan {
            Plan::Withheld => {
                let reason = state
                    .withheld(index)
                    .ok_or_else(|| invalid("A withheld table has no reason"))?;
                self.fact(|| {
                    Ok(TableUpdate::Unavailable {
                        reason: engine_allocation::string(reason)?,
                    })
                })?;
                Ok(match previous {
                    Some(view) if matches!(&*view, TableView::Unavailable { reason: held } if held == reason) => {
                        view
                    }
                    _ => TableView::Unavailable {
                        reason: engine_allocation::string(reason)?,
                    }
                    .into(),
                })
            }
            Plan::Carry => {
                self.fact(|| Ok(TableUpdate::Unchanged { baseline }))?;
                let previous = previous.ok_or_else(|| invalid("A carried table has no view"))?;
                Ok(previous.with_sampled_at(sampled))
            }
            Plan::Anew => {
                let rows = anew
                    .next()
                    .ok_or_else(|| invalid("A table built anew has no rows"))?;
                self.fact(|| Ok(TableUpdate::Snapshot(rows.try_clone_validated()?)))?;
                let view = SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled);
                Ok(if self.changes {
                    view.with_stamp(Stamp::new(hour))
                } else {
                    view
                })
            }
            Plan::Status => {
                // An hour since the views were built under another status forbids a carry.
                let previous = previous.filter(|_| !pending.varied);
                match status {
                    Status::NotRecorded => {
                        let mode = self.mode;
                        self.fact(|| Ok(TableUpdate::NotRecorded { mode }))?;
                        Ok(match previous {
                            Some(view) if matches!(*view, TableView::NotRecorded { .. }) => view,
                            _ => TableView::NotRecorded { mode }.into(),
                        })
                    }
                    Status::Unavailable { reason } => {
                        let reason = dictionary
                            .get(reason)
                            .ok_or_else(|| invalid("A reason names a missing dictionary entry"))?;
                        let reason = std::str::from_utf8(reason)
                            .map_err(|_| invalid("A table's reason is not text"))?;
                        self.fact(|| {
                            Ok(TableUpdate::Unavailable {
                                reason: engine_allocation::string(reason)?,
                            })
                        })?;
                        Ok(match previous {
                            Some(view) if matches!(&*view, TableView::Unavailable { reason: held } if held == reason) => {
                                view
                            }
                            _ => TableView::Unavailable {
                                reason: engine_allocation::string(reason)?,
                            }
                            .into(),
                        })
                    }
                    Status::Truncated { missing } => {
                        let view = state
                            .truncated_view(index)
                            .ok_or_else(|| invalid("A truncated table names no rows it read"))?;
                        self.slots.clear();
                        engine_allocation::reserve(&mut self.slots, view.len())?;
                        for &slot in view {
                            let key = table
                                .key(slot)
                                .ok_or_else(|| invalid("A truncated view names an empty slot"))?;
                            self.slots.push((key, slot));
                        }
                        let rows = rows_at(
                            id,
                            table,
                            dictionary,
                            &mut self.slots,
                            &mut self.keys,
                            self.format,
                        )?;
                        self.fact(|| {
                            Ok(TableUpdate::Truncated {
                                rows: rows.try_clone_validated()?,
                                missing,
                            })
                        })?;
                        Ok(TableView::Truncated { rows, missing }.into())
                    }
                    Status::Complete => Err(invalid("A complete table has no status view")),
                }
            }
            Plan::Patch { changed, deleted } => {
                let previous = previous.ok_or_else(|| invalid("A patched table has no view"))?;
                let mut scratch = PatchScratch {
                    slots: std::mem::take(&mut self.slots),
                    keys: std::mem::take(&mut self.keys),
                    deleted: std::mem::take(&mut self.deleted),
                    patches: std::mem::take(&mut self.patches),
                    masks: std::mem::take(&mut self.masks),
                    union: std::mem::take(&mut self.union),
                };
                let spare = self.spares[index].take();
                let made = patch_table(
                    (id, table, dictionary),
                    previous,
                    spare,
                    !self.record,
                    &pending.edits,
                    (changed, deleted),
                    &mut scratch,
                    (self.format, self.changes),
                    (hour, sampled),
                );
                let PatchScratch {
                    slots,
                    keys,
                    deleted,
                    patches,
                    masks,
                    union,
                } = scratch;
                (self.slots, self.keys, self.deleted, self.patches, self.masks, self.union) =
                    (slots, keys, deleted, patches, masks, union);
                let (view, spare) = made?;
                self.spares[index] = spare;
                if self.record {
                    let delta = self.delta(id, baseline)?;
                    self.fact(|| Ok(delta))?;
                }
                Ok(view)
            }
        }
    }
}


/// Notes an hour of `status`, which sectioned the table when `present`, in its `pending`:
/// whether the hour's edits are to be noted too ([`note_edits`]).
fn note_status(pending: &mut Pending, by_slot: bool, status: Status, present: bool) -> bool {
    pending.hours += 1;
    match pending.status {
        None => pending.status = Some(status),
        Some(first) => pending.varied |= first != status,
    }
    if !present {
        return false;
    }
    pending.sectioned = true;
    if pending.anew || pending.varied || status != Status::Complete {
        pending.edits.clear();
        if by_slot {
            pending.marks = std::mem::take(&mut pending.marks).emptied();
        }
        return false;
    }
    true
}

/// Adds `changes`, what the last hour did to complete `table`, to `pending`, its hours noted
/// since its view, `previous`, was built: by slot, or as keyed edits compacted against it.
fn note_edits(
    changes: &Changes,
    table: &Table,
    pending: &mut Pending,
    by_slot: bool,
    previous: Option<&TableView>,
) -> io::Result<()> {
    if changes.masks.len() != changes.changed.len()
        || changes.fill_masks.len() != changes.fill_slots.len()
    {
        return Err(invalid("A table's change masks miss its changed slots"));
    }
    if by_slot {
        return pending.marks.note(changes, table);
    }
    engine_allocation::reserve(
        &mut pending.edits,
        changes.deleted_keys.len()
            + changes.inserted.len()
            + changes.changed.len()
            + changes.fill_slots.len(),
    )?;
    // Each edit's place is its index, so a sort by key keeps each key's in hour order.
    let place = |edits: &Vec<Noted>| u32::try_from(edits.len()).unwrap_or(PLACES);
    // A key deleted and inserted again in one hour is set, not removed.
    for &key in &changes.deleted_keys {
        let at = place(&pending.edits);
        pending.edits.push(Noted::new(key, at, Edit::Deleted)?);
    }
    for &slot in &changes.inserted {
        let key = table
            .key(slot)
            .ok_or_else(|| invalid("An inserted slot holds no row"))?;
        let at = place(&pending.edits);
        pending
            .edits
            .push(Noted::new(key, at, Edit::Inserted(slot))?);
    }
    let written = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
    for (&slot, &mask) in written.filter(|(_, mask)| **mask != 0) {
        let key = table
            .key(slot)
            .ok_or_else(|| invalid("A changed slot holds no row"))?;
        let at = place(&pending.edits);
        pending
            .edits
            .push(Noted::new(key, at, Edit::Written(mask, slot))?);
    }
    // Edits stay within twice the table's slots and what the last compaction left.
    if pending.edits.len() > table.slots().max(2 * pending.compacted) {
        match previous {
            Some(TableView::Complete(previous)) => {
                compact(&mut pending.edits, previous)?;
                pending.compacted = pending.edits.len();
            }
            _ => pending.edits.clear(),
        }
    }
    Ok(())
}

/// A [`Views::update`] scratch for one patched table at a time.
#[derive(Default)]
struct PatchScratch {
    slots: Vec<(u128, u32)>,
    keys: Vec<u128>,
    deleted: Vec<u128>,
    patches: Vec<(u128, u32, u64)>,
    masks: Vec<(u128, u64)>,
    /// A spare's edits since, then the hours noted since the view after it.
    union: Vec<Noted>,
}

/// [`patched`] of `previous`, keeping it as the next spare while a frame holds it, and patching
/// `spare` in its place when no frame holds that: the view, and the spare to keep. Without
/// `spares`, every view is patched from `previous`.
#[allow(clippy::too_many_arguments)]
fn patch_table(
    held: (TableId, &Table, &Dictionary),
    previous: SharedView,
    spare: Option<Spare>,
    spares: bool,
    edits: &[Noted],
    counts: (usize, usize),
    scratch: &mut PatchScratch,
    stamps: (u16, bool),
    hours: (i32, Option<i32>),
) -> io::Result<(SharedView, Option<Spare>)> {
    if !spares || !previous.is_shared() {
        return patched(held, previous, None, edits, counts, scratch, stamps, hours).map(|view| (view, None));
    }
    let TableView::Complete(rows) = &*previous else {
        return Err(invalid("A patched table's view is not complete"));
    };
    let mut since = engine_allocation::copy_vec(edits)?;
    compact(&mut since, rows)?;
    let kept = Spare {
        view: previous.clone(),
        since,
    };
    let spare = spare.filter(|spare| !spare.view.is_shared());
    let view = match spare {
        Some(Spare { view, since }) => patched(
            held,
            previous,
            Some((view, &since)),
            edits,
            counts,
            scratch,
            stamps,
            hours,
        )?,
        None => patched(held, previous, None, edits, counts, scratch, stamps, hours)?,
    };
    Ok((view, Some(kept)))
}

/// Table `id`'s complete view `previous` given `edits`, the folded edits of the hours noted
/// since, which change `changed` rows and delete `deleted` keys: its inserted rows built from
/// `table`'s columns and the others given the columns written, with change facts and stamps
/// when `changes`. `scratch` keeps the inserted keys, patches and deleted keys made.
#[allow(clippy::too_many_arguments)]
fn patched(
    (id, table, dictionary): (TableId, &Table, &Dictionary),
    previous: SharedView,
    spare: Option<(SharedView, &[Noted])>,
    edits: &[Noted],
    (changed, deleted): (usize, usize),
    scratch: &mut PatchScratch,
    (format, changes): (u16, bool),
    (hour, sampled): (i32, Option<i32>),
) -> io::Result<SharedView> {
    let TableView::Complete(rows) = &*previous else {
        return Err(invalid("A patched table's view is not complete"));
    };
    // From a spare, its edits since and the hours noted since, both in key order, merged: each
    // key's since first, so each key's stay in hour order.
    let union = &mut scratch.union;
    union.clear();
    if let Some((_, since)) = spare {
        engine_allocation::reserve(union, since.len() + edits.len())?;
        let (mut before, mut after) = (since.iter().peekable(), edits.iter().peekable());
        loop {
            let noted = match (before.peek(), after.peek()) {
                (Some(early), Some(late)) if late.key < early.key => after.next(),
                (Some(_), _) => before.next(),
                (None, _) => after.next(),
            };
            let Some(noted) = noted else { break };
            let at = union.len() as u32;
            union.push(Noted::new(noted.key, at, noted.edit())?);
        }
    }
    let base = match &spare {
        Some((view, _)) => match &**view {
            TableView::Complete(rows) => rows,
            _ => return Err(invalid("A spare view is not complete")),
        },
        None => rows,
    };
    let folded: &[Noted] = if spare.is_some() { union } else { edits };
    scratch.slots.clear();
    scratch.deleted.clear();
    scratch.patches.clear();
    {
        let (slots, removed, patches) =
            (&mut scratch.slots, &mut scratch.deleted, &mut scratch.patches);
        fold(folded, |key, edit| {
            match edit {
                Edit::Inserted(slot) => {
                    let slot = held_slot(table, key, slot)
                        .ok_or_else(|| invalid("An inserted key holds no slot"))?;
                    engine_allocation::reserve(slots, 1)?;
                    slots.push((key, slot));
                }
                Edit::Deleted => {
                    if base.contains_key(key) {
                        engine_allocation::reserve(removed, 1)?;
                        removed.push(key);
                    }
                }
                Edit::Written(mask, slot) => {
                    let slot = held_slot(table, key, slot)
                        .ok_or_else(|| invalid("A changed key holds no slot"))?;
                    engine_allocation::reserve(patches, 1)?;
                    patches.push((key, slot, mask));
                }
            }
            Ok(())
        })?;
    }
    // Inserted rows are built whole; a key deleted and inserted again is set.
    let inserted = rows_at(
        id,
        table,
        dictionary,
        &mut scratch.slots,
        &mut scratch.keys,
        format,
    )?;
    if spare.is_none()
        && (scratch.keys.len() + scratch.patches.len() != changed
            || scratch.deleted.len() != deleted)
    {
        return Err(invalid("A table's edits differ from their plan"));
    }
    // Change facts and stamps follow `previous` by the hours noted since it, whatever the rows
    // are patched from.
    let stamp = if changes {
        let recorded = id.last_change().fields(id, format);
        let mut rows_changed = engine_allocation::vec(changed)?;
        let mut removed = Vec::new();
        fold(edits, |key, edit| {
            match edit {
                Edit::Inserted(_) => {
                    engine_allocation::reserve(&mut rows_changed, 1)?;
                    rows_changed.push(RowChange {
                        key,
                        fields: change_fields(id, full_mask(recorded), recorded),
                        inserted: !rows.contains_key(key),
                    });
                }
                Edit::Written(mask, _) => {
                    engine_allocation::reserve(&mut rows_changed, 1)?;
                    rows_changed.push(RowChange {
                        key,
                        fields: change_fields(id, mask, recorded),
                        inserted: false,
                    });
                }
                Edit::Deleted => {
                    if rows.contains_key(key) {
                        engine_allocation::reserve(&mut removed, 1)?;
                        removed.push(key);
                    }
                }
            }
            Ok(())
        })?;
        Some(previous.edited(rows_changed, removed, hour))
    } else {
        None
    };
    let source = match spare {
        Some((view, _)) => {
            drop(previous);
            view
        }
        None => previous,
    };
    let mut held = match source.try_unwrap() {
        Ok(TableView::Complete(rows)) => rows,
        Ok(_) => unreachable!("only complete views are patched"),
        Err(shared) => match &*shared {
            TableView::Complete(rows) => rows.try_clone_validated()?,
            _ => unreachable!("only complete views are patched"),
        },
    };
    if !scratch.keys.is_empty() || !scratch.deleted.is_empty() {
        let deleted = engine_allocation::copy_vec(&scratch.deleted)?;
        held = held.apply_changes(RowChanges::new(inserted, deleted))?;
    }
    // Every other changed or filled row gets the columns written to it.
    if !scratch.patches.is_empty() {
        scratch.masks.clear();
        engine_allocation::reserve(&mut scratch.masks, scratch.patches.len())?;
        scratch.masks
            .extend(scratch.patches.iter().map(|&(key, _, mask)| (key, mask)));
        let (patches, format) = (&scratch.patches, format);
        held.patch_from_wire(&scratch.masks, &mut |at, column| {
            wire(table, dictionary, patches[at].1, column, format)
        })?;
    }
    let view = SharedView::from(TableView::Complete(held)).with_sampled_at(sampled);
    Ok(match stamp {
        Some(stamp) => view.with_stamp(stamp),
        None => view,
    })
}

/// Hours a walk can take on its own: from its run's start or from a restart, through the
/// hour before the run's next restart, or fewer ([`Recording::with_segments`]). Ordinals of the
/// run, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Segment {
    /// The branch its hours are walked as: its run's, or the timeline a view walks it for.
    pub branch: u64,
    pub run: u32,
    /// The page whose restart the walk starts from; None from the run's start.
    pub restart: Option<usize>,
    pub first: u32,
    pub last: u32,
}
