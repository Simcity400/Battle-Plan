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

mod decoder;
mod encoder;
mod wire;

pub use self::{
    decoder::FrameDecoder,
    encoder::{FrameEncoder, PreparedFrame},
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub total_ns: u64,
    pub table_ns: [u64; TABLE_COUNT],
    pub pool_bytes: u64,
    pub dropped_frames: u64,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedView(Arc<TableView>);

impl SharedView {
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
        Arc::try_unwrap(self.0).map_err(Self)
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
}

impl PartialEq for ViewIdentity {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl std::ops::Deref for SharedView {
    type Target = TableView;
    fn deref(&self) -> &TableView {
        &self.0
    }
}

impl std::ops::DerefMut for SharedView {
    fn deref_mut(&mut self) -> &mut TableView {
        Arc::make_mut(&mut self.0)
    }
}

impl From<TableView> for SharedView {
    fn from(view: TableView) -> Self {
        Self(Arc::new(view))
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
}

impl From<TableView> for TableInput {
    fn from(view: TableView) -> Self {
        Self::Sampled(view)
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
            TableView::Complete(rows) => rows.rows::<T>().ok_or_else(|| {
                crate::codec::invalid(format!(
                    "hour {} {table:?} contains another table's rows",
                    self.hour
                ))
            }),
            TableView::Truncated { missing, .. } => Err(crate::codec::invalid(format!(
                "hour {} {table:?} is truncated ({missing} missing rows)",
                self.hour
            ))),
            TableView::Unavailable { reason } => Err(crate::codec::invalid(format!(
                "hour {} {table:?} is unavailable: {reason}",
                self.hour
            ))),
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
    for key in changed
        .iter()
        .map(|row| row.key)
        .chain(deleted.iter().copied())
    {
        table.check_key(key)?;
    }
    if changed.windows(2).any(|w| w[0].key >= w[1].key) || deleted.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid("delta keys must be unique and ascending"));
    }
    let mut deleted = deleted.iter().peekable();
    for row in changed {
        while deleted.peek().is_some_and(|key| **key < row.key) {
            deleted.next();
        }
        if deleted.peek().is_some_and(|key| **key == row.key) {
            return Err(invalid("delta both changes and deletes a key"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
