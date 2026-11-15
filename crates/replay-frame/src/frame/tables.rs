//! A frame's tables read row by row wherever they are held ([`Tables`]): a decoded frame lends
//! its rows ([`Rowset`]).

use super::{DecodedFrame, DecodedSource, Metrics, OperandStatus, TableView};
use crate::{
    BranchId, Hour,
    rows::{RowCodec, TableId, TableSchema},
};
use std::{borrow::Cow, io};

/// One table's rows in key order, lent by a decoded frame.
pub enum Rowset<'a, R> {
    Decoded(&'a [R]),
}

impl<R> Clone for Rowset<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Rowset<'_, R> {}

impl<'a, R: RowCodec> Rowset<'a, R> {
    fn rows(&self) -> &'a [R] {
        match *self {
            Self::Decoded(rows) => rows,
        }
    }

    pub fn len(&self) -> usize {
        self.rows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows keyed from `low` through `high`, in key order.
    pub fn span(self, low: u128, high: u128) -> Self {
        let rows = self.rows();
        let start = rows.partition_point(|row| row.key() < low);
        let end = rows.partition_point(|row| row.key() <= high);
        Self::Decoded(&rows[start..end.max(start)])
    }

    /// The text field `field` of the row keyed `key`, borrowed; never a copy of the row.
    pub fn text(&self, key: u128, field: fn(&R) -> &[u8], column: usize) -> io::Result<Option<&'a [u8]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    /// The same rows: a mask names fields only a built row would leave out.
    pub fn with_mask(self, _mask: u64) -> Self {
        self
    }

    /// Each row, in key order.
    pub fn iter(&self) -> RowsetIter<'a, R> {
        RowsetIter(self.rows().iter())
    }

    /// Each row's key, in order.
    pub fn keys(&self) -> impl Iterator<Item = u128> + use<'a, R> {
        self.rows().iter().map(RowCodec::key)
    }

    /// The row keyed `key`.
    pub fn get(&self, key: u128) -> io::Result<Option<Cow<'a, R>>> {
        let rows = self.rows();
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| Cow::Borrowed(&rows[at])))
    }

    /// Each row's text field `field`, in key order, borrowed; never a copy of the row.
    pub fn texts(&self, field: fn(&R) -> &[u8], column: usize) -> impl Iterator<Item = io::Result<&'a [u8]>> + use<'a, R> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        rows.iter().map(move |row| Ok(field(row)))
    }

    /// The rows whose keys `keep` passes, in key order.
    pub fn keyed<'k>(&self, mut keep: impl FnMut(u128) -> bool + 'k) -> Box<dyn Iterator<Item = io::Result<Cow<'a, R>>> + 'k>
    where
        'a: 'k,
        R: 'k,
    {
        Box::new(self.rows().iter().filter(move |row| keep(row.key())).map(|row| Ok(Cow::Borrowed(row))))
    }

    /// The rows keyed within `keys`, in key order.
    pub fn range(&self, keys: std::ops::RangeInclusive<u128>) -> RowsetIter<'a, R> {
        let (low, high) = (*keys.start(), *keys.end());
        let rows = self.rows();
        let first = rows.partition_point(|row| row.key() < low);
        let end = first + rows[first..].partition_point(|row| row.key() <= high);
        RowsetIter(rows[first..end].iter())
    }

    /// Whether it holds a row keyed `key`.
    pub fn contains(&self, key: u128) -> bool {
        self.rows().binary_search_by_key(&key, RowCodec::key).is_ok()
    }
}

/// A [`Rowset`]'s rows in key order.
pub struct RowsetIter<'a, R>(std::slice::Iter<'a, R>);

impl<'a, R: RowCodec> Iterator for RowsetIter<'a, R> {
    type Item = io::Result<Cow<'a, R>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|row| Ok(Cow::Borrowed(row)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len();
        (len, Some(len))
    }
}

impl<R: RowCodec> DoubleEndedIterator for RowsetIter<'_, R> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|row| Ok(Cow::Borrowed(row)))
    }
}

impl<R: RowCodec> ExactSizeIterator for RowsetIter<'_, R> {}

impl<'a, R: RowCodec> Rowset<'a, R> {
    /// The list field `field` of the row keyed `key`, borrowed.
    pub fn u32s(&self, key: u128, field: fn(&R) -> &[u32], column: usize) -> io::Result<Option<&'a [u32]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    /// [`Self::u32s`] of a list of 64-bit words.
    pub fn u64s(&self, key: u128, field: fn(&R) -> &[u64], column: usize) -> io::Result<Option<&'a [u64]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    /// Its rows at positions `at` in key order.
    pub fn within(self, at: std::ops::Range<usize>) -> Self {
        Self::Decoded(&self.rows()[at])
    }

    /// Each row with its text field `field`, the text borrowed.
    pub fn with_text(self, field: fn(&R) -> &[u8], column: usize) -> impl Iterator<Item = io::Result<(Cow<'a, R>, &'a [u8])>> + use<'a, R> {
        self.iter().zip(self.texts(field, column)).map(|(row, text)| Ok((row?, text?)))
    }

    /// The row at position `at` in key order.
    pub fn nth(&self, at: usize) -> io::Result<Option<Cow<'a, R>>> {
        Ok(self.rows().get(at).map(Cow::Borrowed))
    }

    /// The rows keyed from `low` through `high`, borrowed.
    pub fn slice(self, low: u128, high: u128) -> io::Result<Cow<'a, [R]>>
    where
        R: Clone,
    {
        Ok(Cow::Borrowed(self.span(low, high).rows()))
    }

    /// The position of the row keyed `key` among its rows, in key order.
    pub fn position_of(&self, key: u128) -> Option<usize> {
        self.rows().binary_search_by_key(&key, RowCodec::key).ok()
    }
}

/// One hour's tables as the scripted-text engine reads them: a decoded frame's. Copying it
/// copies a reference; every row it lends lives as long as the frame.
#[derive(Clone, Copy)]
pub enum HourFrame<'a> {
    Decoded(&'a dyn DecodedSource),
}

impl std::fmt::Debug for HourFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HourFrame").field("kind", &"Decoded").field("branch", &self.branch()).field("hour", &self.hour()).finish()
    }
}

impl<'a> From<&'a dyn DecodedSource> for HourFrame<'a> {
    fn from(frame: &'a dyn DecodedSource) -> Self {
        Self::Decoded(frame)
    }
}

impl<'a> From<&'a DecodedFrame> for HourFrame<'a> {
    fn from(frame: &'a DecodedFrame) -> Self {
        Self::Decoded(frame)
    }
}

impl<'a> HourFrame<'a> {
    fn frame(self) -> &'a dyn DecodedSource {
        match self {
            Self::Decoded(frame) => frame,
        }
    }

    pub fn hour(self) -> Hour {
        self.frame().hour()
    }

    pub fn branch(self) -> BranchId {
        self.frame().branch()
    }

    pub fn metrics(self) -> &'a Metrics {
        &self.frame().decoded_frame().metrics
    }

    /// The decoded frame it reads.
    pub fn decoded(self) -> Option<&'a dyn DecodedSource> {
        Some(self.frame())
    }

    /// `table`'s status this hour.
    pub fn status(self, table: TableId) -> OperandStatus<'a> {
        self.frame().status(table)
    }

    /// The hour of `table`'s last complete acquisition, when known.
    pub fn sampled_at(self, table: TableId) -> Option<Hour> {
        self.frame().sampled_at(table.index())
    }

    /// A decoded view's allocation identity.
    pub fn identity(self, table: TableId) -> Option<super::ViewIdentity> {
        self.frame().identity(table.index())
    }

    /// The rows of `T` the hour holds, a complete table's or those a truncated one read; None
    /// where it holds none.
    pub fn rows<T: TableSchema>(self, mask: u64) -> Option<Rowset<'a, T::Row>> {
        <dyn DecodedSource as Tables>::rows::<T>(self.frame(), mask)
    }

    /// [`Self::rows`], empty where the hour holds none.
    pub fn held<T: TableSchema>(self, mask: u64) -> Rowset<'a, T::Row> {
        self.rows::<T>(mask).unwrap_or(Rowset::Decoded(&[]))
    }

    /// The rows of `T` when complete, reading the fields `mask` names; refused, saying why,
    /// otherwise.
    pub fn complete<T: TableSchema>(self, mask: u64) -> io::Result<Rowset<'a, T::Row>> {
        self.frame().decoded_frame().complete_fields::<T>(mask).map(Rowset::Decoded)
    }
}

impl Tables for HourFrame<'_> {
    fn of_branch(&self) -> BranchId {
        self.branch()
    }

    fn at_hour(&self) -> Hour {
        self.hour()
    }

    fn metrics(&self) -> &Metrics {
        HourFrame::metrics(*self)
    }

    fn table_status(&self, table: TableId) -> OperandStatus<'_> {
        self.status(table)
    }

    fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
        self.sampled_at(table)
    }

    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>> {
        HourFrame::rows::<T>(*self, mask)
    }

    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
        HourFrame::complete::<T>(*self, mask)
    }
}

/// A frame's tables read row by row: a decoded frame's, or a columnar walk's hour read through
/// its views.
pub trait Tables {
    fn of_branch(&self) -> BranchId;
    fn at_hour(&self) -> Hour;
    fn metrics(&self) -> &Metrics;
    /// `table`'s status this hour.
    fn table_status(&self, table: TableId) -> OperandStatus<'_>;
    /// The hour of `table`'s last complete acquisition, when known.
    fn table_sampled_at(&self, table: TableId) -> Option<Hour>;
    /// The rows of `T` the hour holds, a complete table's or those a truncated one read, each
    /// holding at least the fields `mask` names; None where it holds none.
    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>>;
    /// The rows of `T` when complete, each holding at least the fields `mask` names; refused,
    /// saying why, otherwise.
    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>>;
}

impl Tables for DecodedFrame {
    fn of_branch(&self) -> BranchId {
        self.branch
    }

    fn at_hour(&self) -> Hour {
        self.hour
    }

    fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    fn table_status(&self, table: TableId) -> OperandStatus<'_> {
        OperandStatus::for_table(&self.tables[table.index()], table)
    }

    fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
        self.tables[table.index()].sampled_at()
    }

    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>> {
        match super::undeclared::reading(mask, || &*self.tables[T::ID.index()]) {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
                let rows = rows.rows::<T>()?;
                if let Some(held) = self.tables[T::ID.index()].fields() {
                    let range = rows.as_ptr_range();
                    super::undeclared::held_rows(range.start as usize..range.end as usize, T::ID, held);
                }
                Some(Rowset::Decoded(rows))
            }
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => None,
        }
    }

    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
        self.complete_fields::<T>(mask).map(Rowset::Decoded)
    }
}

impl Tables for dyn DecodedSource + '_ {
    fn of_branch(&self) -> BranchId {
        self.decoded_frame().of_branch()
    }

    fn at_hour(&self) -> Hour {
        self.decoded_frame().at_hour()
    }

    fn metrics(&self) -> &Metrics {
        &self.decoded_frame().metrics
    }

    fn table_status(&self, table: TableId) -> OperandStatus<'_> {
        self.decoded_frame().table_status(table)
    }

    fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
        self.decoded_frame().table_sampled_at(table)
    }

    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>> {
        self.decoded_frame().rows::<T>(mask)
    }

    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
        self.decoded_frame().complete_rows::<T>(mask)
    }
}

/// A frame's tables through the handles that hold it, as [`DecodedSource`] reads them.
macro_rules! lent_tables {
    ($($handle:ty),+) => {$(
        impl<T: Tables + ?Sized> Tables for $handle {
            fn of_branch(&self) -> BranchId {
                (**self).of_branch()
            }

            fn at_hour(&self) -> Hour {
                (**self).at_hour()
            }

            fn metrics(&self) -> &Metrics {
                (**self).metrics()
            }

            fn table_status(&self, table: TableId) -> OperandStatus<'_> {
                (**self).table_status(table)
            }

            fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
                (**self).table_sampled_at(table)
            }

            fn rows<S: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, S::Row>> {
                (**self).rows::<S>(mask)
            }

            fn complete_rows<S: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, S::Row>> {
                (**self).complete_rows::<S>(mask)
            }
        }
    )+};
}

lent_tables!(&T, Box<T>, std::sync::Arc<T>);
