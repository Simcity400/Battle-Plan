
use crate::{
    BranchId, Hour,
    codec::{Decoder, invalid_static as invalid},
    rows::{DeltaRow, TableId, TableRows, TableSchema},
    scope::{self, CaptureMode},
};
use std::{
    io,
    sync::{Arc, Weak},
};

pub(crate) mod checkpoint;
mod columns;
pub use columns::{Unsayable, cells_rows, cells_rows_of, cells_view, field, stored_frame, table_field};
mod tables;
pub use tables::{HourFrame, Rowset, RowsetIter, Tables};
mod decoder;
mod hour_input;
pub use hour_input::{ChangeDiscontinuity, HourChanges, HourInput, TableChanges};
#[cfg(any(test, feature = "capture", feature = "test-support"))]
mod encoder;
pub mod endpoint_step;
pub mod kept;
mod log;
pub use log::{FrameLog, LoggedFrames};
mod provenance;
mod source;
#[cfg(any(test, feature = "test-support"))]
mod spooled;
pub use source::{
    CommandAllocation, CommandAllocations, DecodedSource, Division, DivisionName, DivisionNames,
    Divisions, Leader, Leaders, OperandStatus, Province, Provinces, RecordedSource, Rows,
    SharedCharacters, SharedLexical,
};
#[doc(hidden)]
pub mod recorded;
pub use recorded::{FrameSource, IntoRecordedFrame, RecordedFrame, RecordedTables};
mod undeclared;
pub use undeclared::{NOT_DECLARED, discovering, discovering_fields, discovering_within, not_declared, outside_reads, reading, undeclared_reads};
mod wire;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use self::decoder::count_unselected;
#[doc(hidden)]
pub use self::decoder::{
    NOT_SELECTED, advance_public_table, advance_public_table_reusing, advance_table,
    keyframe_update, not_selected,
};
#[doc(hidden)]
pub use self::wire::{decode_recorded_update, decode_update};
pub use self::{
    decoder::{FrameAdvance, FrameDecoder, Spare, Spares},
};
#[cfg(any(test, feature = "capture", feature = "test-support"))]
pub use encoder::{
    BaselineRetention, FrameContext, FrameEncoder, PreparedFrame, Retired, TableSegment,
};
use engine_allocation as allocation;

#[cfg(test)]
use self::{
    encoder::BaselineRows,
    wire::{decode_row_count, decode_rows},
};
#[cfg(test)]
use crate::rows;

pub use replay_data::rows::TABLE_COUNT;

pub const TRUNCATED_KEYFRAME_REASON: &str = "truncated at keyframe: ";

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Metrics {
    pub total_ns: u64,
    pub table_ns: [u64; TABLE_COUNT],
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

pub const READ_TIME_NOT_RECORDED: u64 = u64::MAX;

impl Metrics {
    pub fn stored(&self) -> Self {
        let mut stored = self.clone();
        for ns in &mut stored.table_ns {
            if *ns != 0 {
                *ns = READ_TIME_NOT_RECORDED;
            }
        }
        stored
    }

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
    Truncated {
        rows: TableRows,
        missing: u32,
    },
    Unavailable {
        reason: String,
    },
    NotRecorded {
        mode: CaptureMode,
    },
}

impl TableView {
    pub fn is_not_selected(&self) -> bool {
        matches!(self, Self::Unavailable { reason } if reason == NOT_SELECTED)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Keys {
    All,
    These(Vec<u128>),
    Range(u128, u128),
    Ranges(Vec<(u128, u128)>),
}

impl Keys {
    pub fn contains(&self, key: u128) -> bool {
        match self {
            Self::All => true,
            Self::These(keys) => keys.binary_search(&key).is_ok(),
            Self::Range(lo, hi) => (*lo..=*hi).contains(&key),
            Self::Ranges(ranges) => ranges.get(ranges.partition_point(|&(_, hi)| hi < key)).is_some_and(|&(lo, _)| lo <= key),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SharedView(
    Arc<TableView>,
    Option<Hour>,
    Option<Arc<ChangeFacts>>,
    Option<Arc<FieldStamp>>,
    Option<u64>,
    Option<Arc<Keys>>,
);

impl SharedView {
    pub fn sampled_at(&self) -> Option<Hour> {
        if undeclared::is(&self.0) {
            undeclared::read(&self.0);
        }
        view_reads::note_sample(self);
        self.1
    }

    pub fn changes_since(&self, previous: &ViewIdentity) -> Option<&ChangeFacts> {
        self.2
            .as_deref()
            .filter(|facts| facts.previous.same(previous))
    }

    pub fn changes_for(&self, previous: &ViewIdentity, fields: u64) -> Option<&ChangeFacts> {
        self.2
            .as_deref()
            .filter(|facts| previous.fields_same(&facts.previous, fields))
    }

    pub fn with_fields(self, fields: u64) -> Self {
        Self(self.0, self.1, self.2, self.3, Some(fields), self.5)
    }

    pub fn fields(&self) -> Option<u64> {
        self.4
    }

    pub fn with_keys(self, keys: Option<Arc<Keys>>) -> Self {
        Self(self.0, self.1, self.2, self.3, self.4, keys)
    }

    pub fn keys(&self) -> &Keys {
        self.5.as_deref().unwrap_or(&Keys::All)
    }

    pub fn held(&self) -> &TableView {
        &self.0
    }

    pub fn with_sampled_at(self, hour: Option<Hour>) -> Self {
        Self(self.0, hour, self.2, self.3, self.4, self.5)
    }

    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    pub fn is_not_declared(&self) -> bool {
        undeclared::is(&self.0)
    }

    pub fn changed_fields(&self, other: &Self) -> Option<u64> {
        if Self::ptr_eq(self, other) {
            return Some(0);
        }
        let (Some(this), Some(other)) = (self.3.as_deref(), other.3.as_deref()) else {
            return None;
        };
        (Arc::ptr_eq(&this.epoch, &other.epoch) && this.membership == other.membership).then(|| {
            (0..64)
                .filter(|&bit| this.fields[bit] != other.fields[bit])
                .fold(0, |fields, bit| fields | (1_u64 << bit))
        })
    }

    pub fn into_inner(self) -> TableView {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }

    pub fn try_unwrap(self) -> Result<TableView, Self> {
        Arc::try_unwrap(self.0).map_err(|shared| Self(shared, self.1, self.2, self.3, self.4, self.5))
    }

    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }

    pub fn as_ptr(&self) -> *const TableView {
        Arc::as_ptr(&self.0)
    }

    pub fn identity(&self) -> ViewIdentity {
        ViewIdentity(
            Arc::downgrade(&self.0),
            self.3.clone(),
            self.0.is_not_selected(),
        )
    }
}

#[derive(Debug)]
pub struct Stamp(Option<Arc<ChangeFacts>>, Arc<FieldStamp>);

impl Stamp {
    pub fn new(hour: Hour) -> Self {
        Self(None, decoder::field_stamp(None, None, hour))
    }
}

impl SharedView {
    pub fn edited(&self, changed: Vec<RowChange>, deleted: Vec<u128>, hour: Hour) -> Stamp {
        let facts = ChangeFacts {
            previous: self.identity(),
            fields: changed.iter().fold(0, |fields, row| fields | row.fields),
            membership: !deleted.is_empty() || changed.iter().any(|row| row.inserted),
            changed,
            deleted,
        };
        let stamp = decoder::field_stamp(self.3.as_deref(), Some(&facts), hour);
        Stamp(Some(Arc::new(facts)), stamp)
    }

    pub fn with_stamp(mut self, stamp: Stamp) -> Self {
        self.2 = stamp.0;
        self.3 = Some(stamp.1);
        self
    }

    pub fn reclaim_changes(&mut self) {
        if self
            .2
            .as_ref()
            .is_some_and(|facts| Arc::strong_count(facts) == 1)
        {
            self.2 = None;
        }
    }
}

impl PartialEq for SharedView {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SharedView {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RowChange {
    pub key: u128,
    pub fields: u64,
    pub inserted: bool,
}

#[derive(Debug)]
pub struct ChangeFacts {
    previous: ViewIdentity,
    changed: Vec<RowChange>,
    deleted: Vec<u128>,
    fields: u64,
    membership: bool,
}
impl ChangeFacts {
    pub fn changed(&self) -> &[RowChange] {
        &self.changed
    }
    pub fn deleted(&self) -> &[u128] {
        &self.deleted
    }
    pub fn row(&self, key: u128) -> Option<&RowChange> {
        self.changed
            .binary_search_by_key(&key, |row| row.key)
            .ok()
            .map(|at| &self.changed[at])
    }
    pub fn touches(&self, fields: u64) -> bool {
        self.membership || self.fields & fields != 0
    }
}

#[derive(Clone, Debug)]
pub struct ViewIdentity(Weak<TableView>, Option<Arc<FieldStamp>>, bool);

#[derive(Debug)]
struct FieldStamp {
    epoch: Arc<()>,
    fields: [i64; 64],
    membership: i64,
}

impl ViewIdentity {
    pub fn not_selected(&self) -> bool {
        self.2
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_alive(&self) -> bool {
        self.0.strong_count() != 0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn owner_count(&self) -> usize {
        self.0.strong_count()
    }

    pub fn is(&self, view: &SharedView) -> bool {
        std::ptr::eq(self.0.as_ptr(), Arc::as_ptr(&view.0))
            && Self::same_epoch(self.1.as_deref(), view.3.as_deref())
    }

    pub fn fields_unchanged(&self, view: &SharedView, fields: u64) -> bool {
        self.is(view) || Self::stamps_same(self.1.as_deref(), view.3.as_deref(), fields)
    }

    fn same_epoch(previous: Option<&FieldStamp>, current: Option<&FieldStamp>) -> bool {
        match (previous, current) {
            (None, None) => true,
            (Some(previous), Some(current)) => Arc::ptr_eq(&previous.epoch, &current.epoch),
            _ => false,
        }
    }

    fn fields_same(&self, other: &Self, fields: u64) -> bool {
        self.same(other) || Self::stamps_same(self.1.as_deref(), other.1.as_deref(), fields)
    }

    fn stamps_same(
        previous: Option<&FieldStamp>,
        current: Option<&FieldStamp>,
        mut fields: u64,
    ) -> bool {
        let (Some(previous), Some(current)) = (previous, current) else {
            return false;
        };
        if !Arc::ptr_eq(&previous.epoch, &current.epoch)
            || previous.membership != current.membership
        {
            return false;
        }
        while fields != 0 {
            let bit = fields.trailing_zeros() as usize;
            if previous.fields[bit] != current.fields[bit] {
                return false;
            }
            fields &= fields - 1;
        }
        true
    }

    pub fn same(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0) && Self::same_epoch(self.1.as_deref(), other.1.as_deref())
    }
}

impl PartialEq for ViewIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl std::ops::Deref for SharedView {
    type Target = TableView;
    fn deref(&self) -> &TableView {
        if view_reads::RECORDING.with(std::cell::Cell::get) {
            view_reads::note(self);
        }
        if undeclared::is(&self.0) {
            undeclared::read(&self.0);
        }
        if let Some(held) = self.4
            && let TableView::Complete(rows) | TableView::Truncated { rows, .. } = &*self.0
            && let Some(refused) = undeclared::outside(rows.table(), held)
        {
            return refused;
        }
        &self.0
    }
}

pub mod view_reads {
    use super::{SharedView, ViewIdentity};
    use crate::rows::TableId;
    use std::cell::{Cell, RefCell};

    thread_local! {
        pub(super) static RECORDING: Cell<bool> = const { Cell::new(false) };
        static READS: RefCell<Vec<ViewIdentity>> = const { RefCell::new(Vec::new()) };
        static SAMPLES: RefCell<Option<Vec<ViewIdentity>>> = const { RefCell::new(None) };
        static WALKED: RefCell<Vec<TableId>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn note(view: &SharedView) {
        READS.with(|reads| {
            let mut reads = reads.borrow_mut();
            if !reads.iter().any(|read| read.is(view)) {
                reads.push(view.identity());
            }
        });
    }

    pub(super) fn note_sample(view: &SharedView) {
        SAMPLES.with(|samples| {
            if let Some(samples) = samples.borrow_mut().as_mut()
                && !samples.iter().any(|read| read.is(view)) {
                samples.push(view.identity());
            }
        });
    }

    pub fn samples<T>(compute: impl FnOnce() -> T) -> (T, Vec<ViewIdentity>) {
        let outer = SAMPLES.with(|samples| samples.borrow_mut().replace(Vec::new()));
        struct Restore(Option<Option<Vec<ViewIdentity>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(outer) = self.0.take() {
                    SAMPLES.with(|samples| *samples.borrow_mut() = outer);
                }
            }
        }
        let mut restore = Restore(Some(outer));
        let value = compute();
        let inner = SAMPLES.with(|samples| samples.borrow_mut().take()).unwrap_or_default();
        let mut outer = restore.0.take().flatten();
        if let Some(outer) = outer.as_mut() {
            for view in &inner {
                if !outer.iter().any(|read| read.same(view)) { outer.push(view.clone()); }
            }
        }
        SAMPLES.with(|samples| *samples.borrow_mut() = outer);
        (value, inner)
    }

    pub(crate) fn note_walked(table: TableId) {
        if RECORDING.with(Cell::get) {
            WALKED.with(|walked| {
                let mut walked = walked.borrow_mut();
                if !walked.contains(&table) {
                    walked.push(table);
                }
            });
        }
    }

    pub fn credit_walked(tables: &[TableId]) {
        for &table in tables {
            note_walked(table);
        }
    }

    pub fn record_walked<T>(
        compute: impl FnOnce() -> T,
    ) -> (T, Vec<ViewIdentity>, Vec<TableId>) {
        let outer = WALKED.with(|walked| std::mem::take(&mut *walked.borrow_mut()));
        struct Restore(Option<Vec<TableId>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(outer) = self.0.take() {
                    WALKED.with(|walked| *walked.borrow_mut() = outer);
                }
            }
        }
        let mut restore = Restore(Some(outer));
        let was = recording();
        let (value, views) = record_views(compute);
        let inner = WALKED.with(|walked| std::mem::take(&mut *walked.borrow_mut()));
        let mut outer = restore.0.take().unwrap_or_default();
        if was {
            for &table in &inner {
                if !outer.contains(&table) {
                    outer.push(table);
                }
            }
        }
        WALKED.with(|walked| *walked.borrow_mut() = outer);
        (value, views, inner)
    }

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

    pub fn recording() -> bool {
        RECORDING.with(Cell::get)
    }

    pub fn credit(views: &[ViewIdentity]) {
        if !recording() {
            return;
        }
        READS.with(|reads| {
            let mut reads = reads.borrow_mut();
            for view in views {
                if !reads.iter().any(|read| read.same(view)) {
                    reads.push(view.clone());
                }
            }
        });
    }

    pub fn record<T>(compute: impl FnOnce() -> T) -> (T, Vec<ViewIdentity>) {
        let (value, views, _) = record_walked(compute);
        (value, views)
    }

    fn record_views<T>(compute: impl FnOnce() -> T) -> (T, Vec<ViewIdentity>) {
        let outer = READS.with(|reads| std::mem::take(&mut *reads.borrow_mut()));
        let was = RECORDING.with(|recording| recording.replace(true));
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

        #[test]
        fn views_a_helper_thread_read_are_credited_to_the_recording_caller() {
            let (first, second) = (view(), view());
            let ((), read) = record(|| {
                let helper = std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            record(|| {
                                let _ = &*second;
                            })
                            .1
                        })
                        .join()
                        .unwrap()
                });
                assert!(recording());
                credit(&helper);
                credit(&helper);
                let _ = &*first;
            });
            assert_eq!(read.len(), 2);
            assert!(!recording());
            assert_eq!(held(), 0);
        }
    }
}

impl std::ops::DerefMut for SharedView {
    fn deref_mut(&mut self) -> &mut TableView {
        self.1 = None;
        self.2 = None;
        self.3 = None;
        Arc::make_mut(&mut self.0)
    }
}

impl From<TableView> for SharedView {
    fn from(view: TableView) -> Self {
        Self(Arc::new(view), None, None, None, None, None)
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DecodedFrame {
    pub branch: BranchId,
    pub hour: Hour,
    pub tables: [SharedView; TABLE_COUNT],
    pub metrics: Metrics,
}

fn split_shared_tables(
    frame: Arc<DecodedFrame>,
) -> (endpoint_step::Point, [SharedView; TABLE_COUNT]) {
    let point = endpoint_step::Point {
        branch: frame.branch,
        hour: frame.hour,
    };
    let tables = match Arc::try_unwrap(frame) {
        Ok(frame) => frame.tables,
        Err(frame) => std::array::from_fn(|index| frame.tables[index].clone()),
    };
    (point, tables)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableInput {
    Sampled(TableView),
    SharedComplete(Arc<TableRows>),
    ComparedEqual(BaselineToken),
    ComparedChanges {
        token: BaselineToken,
        indices: Vec<usize>,
        rows: TableRows,
    },
    SliceUpdate {
        token: BaselineToken,
        update: Arc<crate::sliced::SliceUpdate>,
    },
    UniformOptionalI32 {
        token: BaselineToken,
        column: usize,
        value: Option<i32>,
        update: Arc<crate::sliced::SliceUpdate>,
    },
}

impl From<TableView> for TableInput {
    fn from(view: TableView) -> Self {
        Self::Sampled(view)
    }
}

impl TableInput {
    pub fn round_to_display(&mut self) {
        match self {
            Self::Sampled(TableView::Complete(rows) | TableView::Truncated { rows, .. })
            | Self::ComparedChanges { rows, .. } => rows.round_to_display(),
            Self::SharedComplete(rows) => {
                if let Some(rows) = Arc::get_mut(rows) {
                    rows.round_to_display();
                }
            }
            Self::Sampled(TableView::Unavailable { .. } | TableView::NotRecorded { .. })
            | Self::ComparedEqual(_)
            | Self::SliceUpdate { .. }
            | Self::UniformOptionalI32 { .. } => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineToken {
    encoder: u64,
    revision: u64,
    table: TableId,
    branch: BranchId,
    hour: Hour,
    baseline: Hour,
    row_updates: bool,
    untracked: bool,
}

impl BaselineToken {
    pub fn allows_row_updates(self) -> bool {
        self.row_updates
    }

    pub fn baseline(self) -> (u64, BranchId, Hour) {
        (self.encoder, self.branch, self.baseline)
    }

    pub fn destination(self) -> (u64, BranchId, Hour) {
        (self.encoder, self.branch, self.hour)
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    Truncated {
        rows: TableRows,
        missing: u32,
    },
    Unavailable {
        reason: String,
    },
    NotRecorded {
        mode: CaptureMode,
    },
    Incompatible {
        reason: String,
        source_format: crate::format::FormatVersion,
        recorded_bytes: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Frame {
    pub branch: BranchId,
    pub hour: Hour,
    pub keyframe: bool,
    pub metrics: Metrics,
    pub tables: [TableUpdate; TABLE_COUNT],
}

pub struct EncodedFrame {
    pub(crate) branch: BranchId,
    pub(crate) hour: Hour,
    pub(crate) keyframe: bool,
    pub(crate) bytes: Vec<u8>,
}

impl EncodedFrame {
    pub(crate) const PREFIX_BYTES: usize = crate::envelope::RECORD_HEADER_BYTES + 1;
    pub fn branch(&self) -> BranchId {
        self.branch
    }
    pub fn hour(&self) -> Hour {
        self.hour
    }
    pub fn keyframe(&self) -> bool {
        self.keyframe
    }

    pub fn into_spool_record(mut self) -> io::Result<Vec<u8>> {
        self.bytes[crate::envelope::RECORD_HEADER_BYTES] = 2;
        crate::envelope::seal_record(self.bytes)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn record_storage(&self) -> (*const u8, usize) {
        (self.bytes.as_ptr(), self.bytes.capacity())
    }

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
    pub fn complete_fields<T: TableSchema>(&self, fields: u64) -> io::Result<&[T::Row]> {
        let rows = undeclared::reading(fields, || self.complete::<T>())?;
        if let Some(held) = self.tables[T::ID.index()].fields() {
            undeclared::held_rows(rows.as_ptr_range().start as usize..rows.as_ptr_range().end as usize, T::ID, held);
        }
        Ok(rows)
    }

    pub fn complete<T: TableSchema>(&self) -> io::Result<&[T::Row]> {
        match self.complete_table(T::ID)?.rows::<T>() {
            Some(rows) => Ok(rows),
            None => Err(self.wrong_table(T::ID)?),
        }
    }

    #[cold]
    fn wrong_table(&self, table: TableId) -> io::Result<io::Error> {
        Ok(crate::codec::invalid(crate::codec::try_format(format_args!(
            "hour {} {table:?} contains another table's rows", self.hour,
        ))?))
    }

    #[inline(never)]
    fn complete_table(&self, table: TableId) -> io::Result<&TableRows> {
        match &*self.tables[table.index()] {
            TableView::Complete(rows) => Ok(rows),
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
            TableView::NotRecorded { mode } => Err(crate::codec::invalid(
                crate::codec::try_format(format_args!(
                    "hour {} {table:?} is not recorded in a {mode} recording",
                    self.hour
                ))?,
            )),
        }
    }

    pub fn capture_mode(&self) -> io::Result<CaptureMode> {
        capture_mode(|index| match *self.tables[index] {
            TableView::NotRecorded { mode } => Some(mode),
            _ => None,
        })
    }

    pub fn recorded(&self, table: TableId) -> bool {
        !matches!(*self.tables[table.index()].0, TableView::NotRecorded { .. })
    }
}

impl FrameInput {
    pub fn capture_mode(&self) -> io::Result<CaptureMode> {
        capture_mode(|index| match self.tables[index] {
            TableInput::Sampled(TableView::NotRecorded { mode }) => Some(mode),
            _ => None,
        })
    }

    pub fn scope_to(&mut self, mode: CaptureMode) {
        for table in TableId::ALL {
            if recorded_here(table) && !scope::records(mode, table) {
                self.tables[table.index()] = TableView::NotRecorded { mode }.into();
                self.metrics.table_ns[table.index()] = 0;
            }
        }
    }
}

impl Frame {
    pub fn capture_mode(&self) -> io::Result<CaptureMode> {
        capture_mode(|index| match self.tables[index] {
            TableUpdate::NotRecorded { mode } => Some(mode),
            _ => None,
        })
    }
}

fn recorded_here(table: TableId) -> bool {
    table
        .last_change()
        .present(crate::format::FormatVersion::CURRENT.version())
}

fn capture_mode(not_recorded: impl Fn(usize) -> Option<CaptureMode>) -> io::Result<CaptureMode> {
    let named = TableId::ALL
        .into_iter()
        .filter(|table| recorded_here(*table))
        .find_map(|table| not_recorded(table.index()));
    let mode = named.unwrap_or(CaptureMode::Full);
    for table in TableId::ALL {
        if !recorded_here(table) {
            continue;
        }
        match not_recorded(table.index()) {
            Some(other) if other != mode => {
                return Err(invalid("frame names more than one capture mode"));
            }
            None if named.is_some() && !scope::records(mode, table) => {
                return Err(invalid(
                    "frame holds a table its capture mode does not record",
                ));
            }
            _ => {}
        }
    }
    Ok(mode)
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

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn current_absence(table: TableId) -> Option<TableView> {
    let format = crate::format::FormatVersion::CURRENT.version();
    let change = table.last_change();
    if !change.present(format) {
        return Some(TableView::Unavailable {
            reason: change
                .table_gap(table, format)
                .expect("absent table has a catalogued gap"),
        });
    }
    (!scope::records(CaptureMode::Full, table)).then_some(TableView::NotRecorded {
        mode: CaptureMode::Full,
    })
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn retire_absent(mut frame: DecodedFrame) -> DecodedFrame {
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
