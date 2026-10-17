//! Per-table baselines. A failed table cannot lend its rows to a later delta.

use crate::{
    BranchId, Hour,
    codec::{Decoder, invalid_static as invalid},
    rows::{DeltaRow, TableId, TableRows, TableSchema},
};
use std::{
    io,
    sync::{Arc, Weak},
};

pub(crate) mod checkpoint;
mod decoder;
mod encoder;
mod provenance;
pub(crate) mod recorded;
pub use recorded::{FrameSource, IntoRecordedFrame, RecordedFrame, RecordedTables};
mod wire;

#[cfg(test)]
pub(crate) use self::decoder::count_unselected;
pub(crate) use self::decoder::{advance_table, keyframe_update, not_selected};
pub(crate) use self::wire::decode_update;
pub use self::{
    decoder::FrameDecoder,
    encoder::{FrameContext, FrameEncoder, PreparedFrame, Retired, TableSegment},
};
use engine::allocation;

#[cfg(test)]
use self::{
    encoder::BaselineRows,
    wire::{decode_row_count, decode_rows},
};
#[cfg(test)]
use crate::rows;

pub const TABLE_COUNT: usize = TableId::ALL.len();

/// Wire reason prefix when a keyframe cannot retain a truncated table baseline.
pub const TRUNCATED_KEYFRAME_REASON: &str = "truncated at keyframe: ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub total_ns: u64,
    pub table_ns: [u64; TABLE_COUNT],
    /// Reserved collector storage. Executor bytes include its actual shared
    /// allocation, payload cells, state words, metadata, queue, thread ids
    /// and handles, boxed runner and any enabled help history arrays.
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

/// `Metrics::table_ns` of a table read in an hour whose recording keeps which
/// tables were read but not how long they took. Nonzero, so the read stays a
/// read; never a time.
pub const READ_TIME_NOT_RECORDED: u64 = u64::MAX;

impl Metrics {
    /// The metrics a replay block stores: which tables were read, as
    /// [`READ_TIME_NOT_RECORDED`], and the exact callback total, pool bytes
    /// and dropped-frame counter. Spool frames keep every read time in
    /// nanoseconds.
    pub fn stored(&self) -> Self {
        let mut stored = self.clone();
        for ns in &mut stored.table_ns {
            if *ns != 0 {
                *ns = READ_TIME_NOT_RECORDED;
            }
        }
        stored
    }

    /// Whether this hour's table read times were recorded.
    pub fn table_times_recorded(&self) -> bool {
        !self.table_ns.contains(&READ_TIME_NOT_RECORDED)
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            total_ns: 0,
            table_ns: [0; TABLE_COUNT],
            pool_bytes: 0,
            dropped_frames: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableView {
    Complete(TableRows),
    Truncated { rows: TableRows, missing: u32 },
    Unavailable { reason: String },
}

/// One decoded table, shared between the hours it did not change in.
///
/// A decoder hands the same allocation to every hour whose update leaves the
/// table unchanged, so copying a [`DecodedFrame`] copies no rows. Reading goes
/// through `Deref`; a write (`DerefMut`) copies the table first when another
/// hour still shares it.
#[derive(Clone, Debug)]
pub struct SharedView(Arc<TableView>, Option<Hour>);

impl SharedView {
    /// Last complete acquisition witnessed by this table's own read timing.
    /// Untimed snapshots, including carried keyframes, have unknown provenance;
    /// uncharged shared reads cannot be certified from another table's timing.
    pub fn sampled_at(&self) -> Option<Hour> {
        self.1
    }

    /// This allocation with another last acquisition: the view a frame of a
    /// later hour holds when the table was carried there unchanged, read or
    /// not (as a quiet hour of `Replay::read_block_changes` reports it).
    pub fn with_sampled_at(self, hour: Option<Hour>) -> Self {
        Self(self.0, hour)
    }

    /// Whether two views are the same allocation, not just equal rows.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    /// The view, moved out when this is its only holder and copied otherwise.
    pub fn into_inner(self) -> TableView {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }

    /// The view when this is its only holder; the shared view otherwise.
    pub fn try_unwrap(self) -> Result<TableView, Self> {
        Arc::try_unwrap(self.0).map_err(|shared| Self(shared, self.1))
    }

    /// The allocation's address, for counting distinct tables.
    pub fn as_ptr(&self) -> *const TableView {
        Arc::as_ptr(&self.0)
    }

    /// This allocation's identity, which does not keep its rows alive.
    pub fn identity(&self) -> ViewIdentity {
        ViewIdentity(Arc::downgrade(&self.0))
    }
}

impl PartialEq for SharedView {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SharedView {}

/// A [`SharedView`]'s allocation, named without holding its rows: the
/// allocation's address cannot be reused while the identity exists, so it
/// matches only views of that allocation, whether or not they are alive.
#[derive(Clone, Debug)]
pub struct ViewIdentity(Weak<TableView>);

impl ViewIdentity {
    /// Whether `view` is the named allocation.
    pub fn is(&self, view: &SharedView) -> bool {
        std::ptr::eq(self.0.as_ptr(), Arc::as_ptr(&view.0))
    }

    /// Whether both name one allocation.
    pub fn same(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl PartialEq for ViewIdentity {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl std::ops::Deref for SharedView {
    type Target = TableView;
    fn deref(&self) -> &TableView {
        if view_reads::RECORDING.with(std::cell::Cell::get) {
            view_reads::note(self);
        }
        &self.0
    }
}

/// The table views a computation on this thread reads, for consumers that
/// reuse a computation's result while every view it read is unchanged.
pub mod view_reads {
    use super::{SharedView, ViewIdentity};
    use std::cell::{Cell, RefCell};

    thread_local! {
        pub(super) static RECORDING: Cell<bool> = const { Cell::new(false) };
        static READS: RefCell<Vec<ViewIdentity>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn note(view: &SharedView) {
        READS.with(|reads| {
            let mut reads = reads.borrow_mut();
            if !reads.iter().any(|read| read.is(view)) {
                reads.push(view.identity());
            }
        });
    }

    /// Run `compute` without recording the views it reads: a caller that
    /// records `compute`'s result itself already holds everything it used.
    pub fn unrecorded<T>(compute: impl FnOnce() -> T) -> T {
        let was = RECORDING.with(|recording| recording.replace(false));
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                RECORDING.with(|recording| recording.set(self.0));
            }
        }
        let _restore = Restore(was);
        compute()
    }

    /// Run `compute`, returning the views it read, each once. Recording
    /// nests: an enclosing recording also holds the views read inside.
    pub fn record<T>(compute: impl FnOnce() -> T) -> (T, Vec<ViewIdentity>) {
        let outer = READS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
        let was = RECORDING.with(|recording| recording.replace(true));
        // Restore the enclosing recording even when `compute` unwinds.
        struct Restore(Option<Vec<ViewIdentity>>, bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                RECORDING.with(|recording| recording.set(self.1));
                if let Some(outer) = self.0.take() {
                    READS.with(|reads| *reads.borrow_mut() = outer);
                }
            }
        }
        let mut restore = Restore(Some(outer), was);
        let value = compute();
        let inner = READS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
        let mut outer = restore.0.take().unwrap_or_default();
        // Only an enclosing recording holds the views read inside; an
        // outermost recording leaves nothing behind on this thread.
        if was {
            for view in &inner {
                if !outer.iter().any(|read| read.same(view)) {
                    outer.push(view.clone());
                }
            }
        }
        READS.with(|reads| *reads.borrow_mut() = outer);
        drop(restore);
        (value, inner)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::frame::TableView;

        fn view() -> SharedView {
            TableView::Unavailable {
                reason: "test view".into(),
            }
            .into()
        }

        fn held() -> usize {
            READS.with(|reads| reads.borrow().len())
        }

        #[test]
        fn independent_recordings_hold_nothing_after_they_return() {
            let views: Vec<_> = (0..100).map(|_| view()).collect();
            for view in &views {
                let ((), read) = record(|| {
                    let _ = &**view;
                });
                assert_eq!(read.len(), 1);
                assert_eq!(held(), 0);
            }
        }

        #[test]
        fn a_nested_recording_also_holds_its_reads_in_the_enclosing_one() {
            let (first, second) = (view(), view());
            let (((), inner), outer) = record(|| {
                let _ = &*first;
                record(|| {
                    let _ = &*second;
                })
            });
            assert_eq!(inner.len(), 1);
            assert_eq!(outer.len(), 2);
            assert_eq!(held(), 0);
        }
    }
}

impl std::ops::DerefMut for SharedView {
    fn deref_mut(&mut self) -> &mut TableView {
        self.1 = None;
        Arc::make_mut(&mut self.0)
    }
}

impl From<TableView> for SharedView {
    fn from(view: TableView) -> Self {
        Self(Arc::new(view), None)
    }
}

impl PartialEq<TableView> for SharedView {
    fn eq(&self, other: &TableView) -> bool {
        *self.0 == *other
    }
}

impl PartialEq<SharedView> for TableView {
    fn eq(&self, other: &SharedView) -> bool {
        *self == *other.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedFrame {
    pub branch: BranchId,
    pub hour: Hour,
    pub tables: [SharedView; TABLE_COUNT],
    pub metrics: Metrics,
}

/// Encoder-only input. Compared rows describe the current acquisition boundary
/// and were compared exactly with this encoder's complete baseline. The producer
/// may certify owned data through complete mutation observation; carrying older
/// unchecked values does not satisfy this contract.
/// Decoders and the wire format continue to use only `TableView`/`TableUpdate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableInput {
    Sampled(TableView),
    /// Complete owned replay rows shared with the producer's recovery cache.
    /// Exact identity with this encoder's immutable committed allocation reuses
    /// its row-validation/equality result; enclosing-hour checks still run.
    SharedComplete(Arc<TableRows>),
    ComparedEqual(BaselineToken),
    /// Every current key matches its baseline position, and every omitted row
    /// was compared exactly with data certified for this boundary. Only differing rows own
    /// new replay storage. Membership changes require a complete input.
    ComparedChanges {
        token: BaselineToken,
        indices: Vec<usize>,
        rows: TableRows,
    },
    /// An owner-sliced table as an update of this encoder's shared complete
    /// baseline: every row of the update's owners is replaced by its rows,
    /// the read the producer's merge of the same rows describes
    /// ([`crate::sliced::SliceUpdate`]). It encodes and commits exactly as
    /// that merged complete table would, without materialising it; commit
    /// replaces the rows inside the baseline.
    SliceUpdate {
        token: BaselineToken,
        update: Arc<crate::sliced::SliceUpdate>,
    },
}

impl From<TableView> for TableInput {
    fn from(view: TableView) -> Self {
        Self::Sampled(view)
    }
}

impl TableInput {
    /// Round the stepped fields of the rows this input owns to their display
    /// steps ([`crate::display`]): complete, truncated and replacement rows,
    /// and shared rows only while this input holds their sole reference.
    /// Compared and slice inputs name rows a baseline already holds.
    pub fn round_to_display(&mut self) {
        match self {
            Self::Sampled(TableView::Complete(rows) | TableView::Truncated { rows, .. })
            | Self::ComparedChanges { rows, .. } => rows.round_to_display(),
            Self::SharedComplete(rows) => {
                if let Some(rows) = Arc::get_mut(rows) {
                    rows.round_to_display();
                }
            }
            Self::Sampled(TableView::Unavailable { .. })
            | Self::ComparedEqual(_)
            | Self::SliceUpdate { .. } => {}
        }
    }
}

/// Identifies the exact baseline and destination for a producer's comparison.
/// The encoder checks these fields again before committing any table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineToken {
    encoder: u64,
    revision: u64,
    table: TableId,
    branch: BranchId,
    hour: Hour,
    baseline: Hour,
    row_updates: bool,
}

impl BaselineToken {
    /// Shared recovery snapshots and tables with cross-row constraints cannot
    /// be edited through sparse replacements.
    pub fn allows_row_updates(self) -> bool {
        self.row_updates
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInput {
    pub branch: BranchId,
    pub hour: Hour,
    pub tables: [TableInput; TABLE_COUNT],
    pub metrics: Metrics,
}

impl FrameInput {
    /// [`TableInput::round_to_display`] for every table.
    pub fn round_to_display(&mut self) {
        for table in &mut self.tables {
            table.round_to_display();
        }
    }
}

impl From<DecodedFrame> for FrameInput {
    fn from(source: DecodedFrame) -> Self {
        Self {
            branch: source.branch,
            hour: source.hour,
            tables: source
                .tables
                .map(|table| TableInput::Sampled(table.into_inner())),
            metrics: source.metrics,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableUpdate {
    Snapshot(TableRows),
    Delta {
        baseline: Hour,
        changed: Vec<DeltaRow>,
        deleted: Vec<u128>,
    },
    Unchanged {
        baseline: Hour,
    },
    // A partial walk never asserts that absent keys were deleted.
    Truncated {
        rows: TableRows,
        missing: u32,
    },
    Unavailable {
        reason: String,
    },
    /// Discarded incompatible source update. Retain its length for inspection;
    /// current writers emit only the unavailable reason, never historical rows.
    Incompatible {
        reason: String,
        source_format: crate::format::FormatVersion,
        recorded_bytes: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub branch: BranchId,
    pub hour: Hour,
    pub keyframe: bool,
    pub metrics: Metrics,
    pub tables: [TableUpdate; TABLE_COUNT],
}

/// Validated frame bytes produced without allocating an intermediate delta for
/// each changed row. Its private payload cannot be modified by a spool caller.
pub struct EncodedFrame {
    pub(crate) branch: BranchId,
    pub(crate) hour: Hour,
    pub(crate) keyframe: bool,
    pub(crate) bytes: Vec<u8>,
}

impl EncodedFrame {
    // Reserve the spool envelope once; finalizing it needs neither another
    // allocation nor a copy of the encoded frame.
    pub(crate) const PREFIX_BYTES: usize = crate::spool::RECORD_HEADER_BYTES + 1;
    pub fn decode(&self) -> io::Result<Frame> {
        let mut input = Decoder::new(&self.bytes[Self::PREFIX_BYTES..]);
        let frame = Frame::decode(&mut input)?;
        input.finish()?;
        Ok(frame)
    }
}

fn put_count(out: &mut Vec<u8>, n: usize) -> io::Result<()> {
    allocation::reserve(out, 4)?;
    out.extend_from_slice(
        &u32::try_from(n)
            .map_err(|_| invalid("too many rows"))?
            .to_le_bytes(),
    );
    Ok(())
}

impl DecodedFrame {
    pub fn complete<T: TableSchema>(&self) -> io::Result<&[T::Row]> {
        let table = T::ID;
        match &*self.tables[table.index()] {
            TableView::Complete(rows) => match rows.rows::<T>() {
                Some(rows) => Ok(rows),
                None => Err(crate::codec::invalid(crate::codec::try_format(
                    format_args!("hour {} {table:?} contains another table's rows", self.hour),
                )?)),
            },
            TableView::Truncated { missing, .. } => Err(crate::codec::invalid(
                crate::codec::try_format(format_args!(
                    "hour {} {table:?} is truncated ({missing} missing rows)",
                    self.hour
                ))?,
            )),
            TableView::Unavailable { reason } => {
                Err(crate::codec::invalid(crate::codec::try_format(
                    format_args!("hour {} {table:?} is unavailable: {reason}", self.hour),
                )?))
            }
        }
    }
}
fn validate_rows(table: TableId, rows: &TableRows) -> io::Result<()> {
    if rows.table() != table {
        return Err(invalid(
            "table rows must have matching types and unique ascending keys",
        ));
    }
    rows.validate_keys()?;
    validate_slots(rows)
}
/// [`validate_complete`] of rows whose keys a validated delta application
/// already checked.
fn validate_applied(table: TableId, rows: &TableRows, hour: Hour) -> io::Result<()> {
    if rows.table() != table {
        return Err(invalid(
            "table rows must have matching types and unique ascending keys",
        ));
    }
    validate_slots(rows)?;
    validate_singleton(table, rows, hour)
}
fn validate_slots(rows: &TableRows) -> io::Result<()> {
    if let TableRows::Countries(countries) = rows {
        let mut slots = allocation::vec(countries.len())?;
        slots.extend(countries.iter().map(|country| country.canonical_slot));
        slots.sort_unstable();
        if slots.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("country rows have duplicate canonical slots"));
        }
    }
    Ok(())
}
fn validate_complete(table: TableId, rows: &TableRows, hour: Hour) -> io::Result<()> {
    validate_rows(table, rows)?;
    validate_singleton(table, rows, hour)
}
fn validate_singleton(table: TableId, rows: &TableRows, hour: Hour) -> io::Result<()> {
    if table.key_bits() == 0 && rows.len() != 1 {
        return Err(invalid(
            "complete singleton table must contain exactly one row",
        ));
    }
    if let TableRows::Clock(rows) = rows
        && rows[0].hour != hour
    {
        return Err(invalid("clock row hour differs from frame hour"));
    }
    Ok(())
}

fn validate_delta(table: TableId, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<()> {
    validate_delta_keys(table, changed.iter().map(|row| row.key), deleted)
}

fn validate_delta_keys(
    table: TableId,
    changed: impl Iterator<Item = u128> + Clone,
    deleted: &[u128],
) -> io::Result<()> {
    for key in changed.clone().chain(deleted.iter().copied()) {
        table.check_key(key)?;
    }
    if changed
        .clone()
        .zip(changed.clone().skip(1))
        .any(|(a, b)| a >= b)
        || deleted.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid("delta keys must be unique and ascending"));
    }
    let mut deleted = deleted.iter().peekable();
    for changed_key in changed {
        while deleted.peek().is_some_and(|key| **key < changed_key) {
            deleted.next();
        }
        if deleted.peek().is_some_and(|key| **key == changed_key) {
            return Err(invalid("delta both changes and deletes a key"));
        }
    }
    Ok(())
}

/// The view a current recording decodes for a table its format does not
/// record (a retired table), or None for a recorded table. Test sources that
/// supply every table replace retired ones with it, as decoding would.
#[cfg(test)]
pub(crate) fn current_absence(table: TableId) -> Option<TableView> {
    let format = crate::format::FormatVersion::CURRENT.version();
    let change = table.last_change();
    (!change.present(format)).then(|| TableView::Unavailable {
        reason: change
            .table_gap(table, format)
            .expect("absent table has a catalogued gap"),
    })
}

/// [`current_absence`] applied to every table of a test source, whose timing
/// a current recording also omits.
#[cfg(test)]
pub(crate) fn retire_absent(mut frame: DecodedFrame) -> DecodedFrame {
    for table in TableId::ALL {
        if let Some(view) = current_absence(table) {
            *frame.tables[table.index()] = view;
            frame.metrics.table_ns[table.index()] = 0;
        }
    }
    frame
}

#[cfg(test)]
mod tests;
