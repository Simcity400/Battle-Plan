//! A frame's tables read row by row wherever they are held ([`Tables`]): a decoded frame lends
//! its rows; a columnar walk's views build each row of the fields its reader names alone
//! ([`Rowset`]). Readers shared by both paths read through it.

use super::{
    ColumnFrame, Columns, DecodedFrame, DecodedSource, Metrics, OperandStatus, RecordedSource,
    TableView,
};
use crate::{
    BranchId, Hour,
    rows::{RowCodec, TableId, TableSchema},
};
use std::{borrow::Cow, io};

/// One table's rows in key order: a decoded frame's, or a walk's view building each row of the
/// fields `mask` names, every other default.
pub enum Rowset<'a, R> {
    Decoded(&'a [R]),
    Walked(Columns<'a>, u64),
}

impl<R> Clone for Rowset<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Rowset<'_, R> {}

impl<'a, R: RowCodec> Rowset<'a, R> {
    pub fn len(&self) -> usize {
        match self {
            Self::Decoded(rows) => rows.len(),
            Self::Walked(rows, _) => rows.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows keyed from `low` through `high`, in key order.
    pub fn span(self, low: u128, high: u128) -> Self {
        match self {
            Self::Decoded(rows) => {
                let start = rows.partition_point(|row| row.key() < low);
                let end = rows.partition_point(|row| row.key() <= high);
                Self::Decoded(&rows[start..end.max(start)])
            }
            Self::Walked(rows, mask) => Self::Walked(rows.span(low, high), mask),
        }
    }

    /// The text field of the row keyed `key`, borrowed where it is held: `field` of a decoded
    /// row, column `column` of a walked one; never a copy of the row.
    pub fn text(
        &self,
        key: u128,
        field: fn(&R) -> &[u8],
        column: usize,
    ) -> io::Result<Option<&'a [u8]>> {
        match *self {
            Self::Decoded(rows) => Ok(rows
                .binary_search_by_key(&key, RowCodec::key)
                .ok()
                .map(|at| field(&rows[at]))),
            Self::Walked(rows, _) => match rows.find(key) {
                Some(slot) => rows.text(slot, column).map(Some),
                None => Ok(None),
            },
        }
    }

    /// The same rows, a walk's building the fields `mask` names in place of its own.
    pub fn with_mask(self, mask: u64) -> Self {
        match self {
            Self::Walked(rows, _) => Self::Walked(rows, mask),
            decoded => decoded,
        }
    }

    /// Each row, in key order.
    pub fn iter(&self) -> RowsetIter<'a, R> {
        RowsetIter(match *self {
            Self::Decoded(rows) => Iter::Decoded(rows.iter()),
            Self::Walked(rows, mask) => Iter::Walked(rows, mask, rows.slots().iter()),
        })
    }

    /// Each row's key, in order.
    pub fn keys(&self) -> impl Iterator<Item = u128> + use<'a, R> {
        let (decoded, walked) = match *self {
            Self::Decoded(rows) => (rows, None),
            Self::Walked(rows, _) => (&[][..], Some(rows)),
        };
        decoded.iter().map(RowCodec::key).chain(
            walked
                .into_iter()
                .flat_map(|rows| rows.slots().iter().map(move |&slot| rows.key(slot))),
        )
    }

    /// The row keyed `key`.
    pub fn get(&self, key: u128) -> io::Result<Option<Cow<'a, R>>> {
        match *self {
            Self::Decoded(rows) => Ok(rows
                .binary_search_by_key(&key, RowCodec::key)
                .ok()
                .map(|at| Cow::Borrowed(&rows[at]))),
            Self::Walked(rows, mask) => match rows.find(key) {
                Some(slot) => rows.row(slot, mask).map(|row| Some(Cow::Owned(row))),
                None => Ok(None),
            },
        }
    }

    /// Each row's text field, in key order, borrowed where it is held: `field` of a decoded row,
    /// column `column` of a walked one; never a copy of the row.
    pub fn texts(
        &self,
        field: fn(&R) -> &[u8],
        column: usize,
    ) -> impl Iterator<Item = io::Result<&'a [u8]>> + use<'a, R> {
        let (decoded, walked) = match *self {
            Self::Decoded(rows) => (rows, None),
            Self::Walked(rows, _) => (&[][..], Some(rows)),
        };
        decoded.iter().map(move |row| Ok(field(row))).chain(
            walked.into_iter().flat_map(move |rows| {
                rows.slots().iter().map(move |&slot| rows.text(slot, column))
            }),
        )
    }

    /// The rows whose keys `keep` passes, in key order: a walk builds only those.
    pub fn keyed<'k>(
        &self,
        mut keep: impl FnMut(u128) -> bool + 'k,
    ) -> Box<dyn Iterator<Item = io::Result<Cow<'a, R>>> + 'k>
    where
        'a: 'k,
        R: 'k,
    {
        match *self {
            Self::Decoded(rows) => Box::new(
                rows.iter()
                    .filter(move |row| keep(row.key()))
                    .map(|row| Ok(Cow::Borrowed(row))),
            ),
            Self::Walked(rows, mask) => Box::new(
                rows.slots()
                    .iter()
                    .filter(move |&&slot| keep(rows.key(slot)))
                    .map(move |&slot| rows.row(slot, mask).map(Cow::Owned)),
            ),
        }
    }

    /// The rows keyed within `keys`, in key order: a walk builds only those.
    pub fn range(&self, keys: std::ops::RangeInclusive<u128>) -> RowsetIter<'a, R> {
        let (low, high) = (*keys.start(), *keys.end());
        RowsetIter(match *self {
            Self::Decoded(rows) => {
                let first = rows.partition_point(|row| row.key() < low);
                let end = first + rows[first..].partition_point(|row| row.key() <= high);
                Iter::Decoded(rows[first..end].iter())
            }
            Self::Walked(rows, mask) => {
                let first = rows.position(low).unwrap_or_else(|at| at);
                let end = match rows.position(high) {
                    Ok(at) => at + 1,
                    Err(at) => at,
                }
                .max(first);
                Iter::Walked(rows, mask, rows.slots()[first..end].iter())
            }
        })
    }

    /// Whether it holds a row keyed `key`.
    pub fn contains(&self, key: u128) -> bool {
        match *self {
            Self::Decoded(rows) => rows.binary_search_by_key(&key, RowCodec::key).is_ok(),
            Self::Walked(rows, _) => rows.find(key).is_some(),
        }
    }
}

/// A [`Rowset`]'s rows in key order; a walk's built as they are reached.
pub struct RowsetIter<'a, R>(Iter<'a, R>);

enum Iter<'a, R> {
    Decoded(std::slice::Iter<'a, R>),
    Walked(Columns<'a>, u64, std::slice::Iter<'a, u32>),
}

impl<'a, R: RowCodec> Iterator for RowsetIter<'a, R> {
    type Item = io::Result<Cow<'a, R>>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Iter::Decoded(rows) => rows.next().map(|row| Ok(Cow::Borrowed(row))),
            Iter::Walked(rows, mask, slots) => slots
                .next()
                .map(|&slot| rows.row(slot, *mask).map(Cow::Owned)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match &self.0 {
            Iter::Decoded(rows) => rows.len(),
            Iter::Walked(_, _, slots) => slots.len(),
        };
        (len, Some(len))
    }
}

impl<R: RowCodec> DoubleEndedIterator for RowsetIter<'_, R> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Iter::Decoded(rows) => rows.next_back().map(|row| Ok(Cow::Borrowed(row))),
            Iter::Walked(rows, mask, slots) => slots
                .next_back()
                .map(|&slot| rows.row(slot, *mask).map(Cow::Owned)),
        }
    }
}

impl<R: RowCodec> ExactSizeIterator for RowsetIter<'_, R> {}

impl<'a, R: RowCodec> Rowset<'a, R> {
    /// The list field of the row keyed `key`, borrowed where it is held: `field` of a decoded
    /// row, column `column` of a walked one.
    pub fn u32s(
        &self,
        key: u128,
        field: fn(&R) -> &[u32],
        column: usize,
    ) -> io::Result<Option<&'a [u32]>> {
        match *self {
            Self::Decoded(rows) => Ok(rows
                .binary_search_by_key(&key, RowCodec::key)
                .ok()
                .map(|at| field(&rows[at]))),
            Self::Walked(rows, _) => match rows.find(key) {
                Some(slot) => rows.u32s(slot, column)?.map(Some).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{:?} column {column} was not recorded", rows.id()),
                    )
                }),
                None => Ok(None),
            },
        }
    }

    /// [`Self::u32s`] of a list of 64-bit words.
    pub fn u64s(
        &self,
        key: u128,
        field: fn(&R) -> &[u64],
        column: usize,
    ) -> io::Result<Option<&'a [u64]>> {
        match *self {
            Self::Decoded(rows) => Ok(rows
                .binary_search_by_key(&key, RowCodec::key)
                .ok()
                .map(|at| field(&rows[at]))),
            Self::Walked(rows, _) => match rows.find(key) {
                Some(slot) => rows.u64s(slot, column)?.map(Some).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{:?} column {column} was not recorded", rows.id()),
                    )
                }),
                None => Ok(None),
            },
        }
    }

    /// Its rows at positions `at` in key order.
    pub fn within(self, at: std::ops::Range<usize>) -> Self {
        match self {
            Self::Decoded(rows) => Self::Decoded(&rows[at]),
            Self::Walked(rows, mask) => Self::Walked(rows.within(at), mask),
        }
    }

    /// Each row with its text field, the text borrowed where it is held: `field` of a decoded
    /// row, column `column` of a walked one, which its mask need not name.
    pub fn with_text(
        self,
        field: fn(&R) -> &[u8],
        column: usize,
    ) -> impl Iterator<Item = io::Result<(Cow<'a, R>, &'a [u8])>> + use<'a, R> {
        self.iter()
            .zip(self.texts(field, column))
            .map(|(row, text)| Ok((row?, text?)))
    }

    /// The row at position `at` in key order.
    pub fn nth(&self, at: usize) -> io::Result<Option<Cow<'a, R>>> {
        match *self {
            Self::Decoded(rows) => Ok(rows.get(at).map(Cow::Borrowed)),
            Self::Walked(rows, mask) => rows
                .slot(at)
                .map(|slot| rows.row(slot, mask).map(Cow::Owned))
                .transpose(),
        }
    }

    /// The rows keyed from `low` through `high`, borrowed where a decoded frame holds them.
    pub fn slice(self, low: u128, high: u128) -> io::Result<Cow<'a, [R]>>
    where
        R: Clone,
    {
        match self.span(low, high) {
            Self::Decoded(rows) => Ok(Cow::Borrowed(rows)),
            walked => walked
                .iter()
                .map(|row| row.map(Cow::into_owned))
                .collect::<io::Result<Vec<_>>>()
                .map(Cow::Owned),
        }
    }

    /// The position of the row keyed `key` among its rows, in key order.
    pub fn position_of(&self, key: u128) -> Option<usize> {
        match *self {
            Self::Decoded(rows) => rows.binary_search_by_key(&key, RowCodec::key).ok(),
            Self::Walked(rows, _) => rows.position(key).ok(),
        }
    }
}

/// One hour's tables as the scripted-text engine reads them: a decoded frame's, or a columnar
/// walk's views of the tables its lanes declared. Copying it copies a reference or a walk's
/// frame; every row it lends lives as long as the frame.
#[derive(Clone, Copy)]
pub enum HourFrame<'a> {
    Decoded(&'a dyn DecodedSource),
    Walked(ColumnFrame<'a>),
}

impl std::fmt::Debug for HourFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Decoded(_) => "Decoded",
            Self::Walked(_) => "Walked",
        };
        f.debug_struct("HourFrame")
            .field("kind", &kind)
            .field("branch", &self.branch())
            .field("hour", &self.hour())
            .finish()
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

impl<'a> From<ColumnFrame<'a>> for HourFrame<'a> {
    fn from(frame: ColumnFrame<'a>) -> Self {
        Self::Walked(frame)
    }
}

impl<'a> HourFrame<'a> {
    pub fn hour(self) -> Hour {
        match self {
            Self::Decoded(frame) => frame.hour(),
            Self::Walked(frame) => frame.hour,
        }
    }

    pub fn branch(self) -> BranchId {
        match self {
            Self::Decoded(frame) => frame.branch(),
            Self::Walked(frame) => frame.branch,
        }
    }

    pub fn metrics(self) -> &'a Metrics {
        match self {
            Self::Decoded(frame) => &frame.decoded_frame().metrics,
            Self::Walked(frame) => frame.metrics,
        }
    }

    /// The decoded frame it reads, if it reads one.
    pub fn decoded(self) -> Option<&'a dyn DecodedSource> {
        match self {
            Self::Decoded(frame) => Some(frame),
            Self::Walked(_) => None,
        }
    }

    /// `table`'s status this hour.
    pub fn status(self, table: TableId) -> OperandStatus<'a> {
        match self {
            Self::Decoded(frame) => frame.status(table),
            Self::Walked(frame) => frame.view(table).status(),
        }
    }

    /// The hour of `table`'s last complete acquisition, when known.
    pub fn sampled_at(self, table: TableId) -> Option<Hour> {
        match self {
            Self::Decoded(frame) => frame.sampled_at(table.index()),
            Self::Walked(frame) => frame.sampled_at(table),
        }
    }

    /// A decoded view's allocation identity; a walk's views have none.
    pub fn identity(self, table: TableId) -> Option<super::ViewIdentity> {
        match self {
            Self::Decoded(frame) => frame.identity(table.index()),
            Self::Walked(_) => None,
        }
    }

    /// The rows of `T` the hour holds, a complete table's or those a truncated one read, a
    /// walk's building the fields `mask` names; None where it holds none.
    pub fn rows<T: TableSchema>(self, mask: u64) -> Option<Rowset<'a, T::Row>> {
        match self {
            Self::Decoded(frame) => {
                <dyn DecodedSource as Tables>::rows::<T>(frame, mask)
            }
            Self::Walked(frame) => frame.view(T::ID).rows().map(|rows| Rowset::Walked(rows, mask)),
        }
    }

    /// [`Self::rows`], empty where the hour holds none.
    pub fn held<T: TableSchema>(self, mask: u64) -> Rowset<'a, T::Row> {
        self.rows::<T>(mask).unwrap_or(Rowset::Decoded(&[]))
    }

    /// The rows of `T` when complete, a walk's building the fields `mask` names; refused,
    /// saying why, otherwise, as a decoded frame refuses.
    pub fn complete<T: TableSchema>(self, mask: u64) -> io::Result<Rowset<'a, T::Row>> {
        match self {
            Self::Decoded(frame) => frame.decoded_frame().complete::<T>().map(Rowset::Decoded),
            Self::Walked(frame) => frame.complete(T::ID).map(|rows| Rowset::Walked(rows, mask)),
        }
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

    fn rows<T: TableSchema>(&self, _: u64) -> Option<Rowset<'_, T::Row>> {
        match &*self.tables[T::ID.index()] {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
                rows.rows::<T>().map(Rowset::Decoded)
            }
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => None,
        }
    }

    fn complete_rows<T: TableSchema>(&self, _: u64) -> io::Result<Rowset<'_, T::Row>> {
        self.complete::<T>().map(Rowset::Decoded)
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

impl Tables for ColumnFrame<'_> {
    fn of_branch(&self) -> BranchId {
        self.branch
    }

    fn at_hour(&self) -> Hour {
        self.hour
    }

    fn metrics(&self) -> &Metrics {
        self.metrics
    }

    fn table_status(&self, table: TableId) -> OperandStatus<'_> {
        self.view(table).status()
    }

    fn table_sampled_at(&self, table: TableId) -> Option<Hour> {
        ColumnFrame::sampled_at(self, table)
    }

    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>> {
        self.view(T::ID).rows().map(|rows| Rowset::Walked(rows, mask))
    }

    fn complete_rows<T: TableSchema>(&self, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
        self.complete(T::ID).map(|rows| Rowset::Walked(rows, mask))
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
