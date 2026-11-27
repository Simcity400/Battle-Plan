
use super::{DecodedFrame, DecodedSource, Metrics, OperandStatus, TableView};
use crate::{
    BranchId, Hour,
    rows::{RowCodec, TableId, TableSchema},
};
use std::{borrow::Cow, io};

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

    pub fn span(self, low: u128, high: u128) -> Self {
        let rows = self.rows();
        let start = rows.partition_point(|row| row.key() < low);
        let end = rows.partition_point(|row| row.key() <= high);
        Self::Decoded(&rows[start..end.max(start)])
    }

    pub fn text(&self, key: u128, field: fn(&R) -> &[u8], column: usize) -> io::Result<Option<&'a [u8]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    pub fn with_mask(self, _mask: u64) -> Self {
        self
    }

    pub fn iter(&self) -> RowsetIter<'a, R> {
        RowsetIter(self.rows().iter())
    }

    pub fn keys(&self) -> impl Iterator<Item = u128> + use<'a, R> {
        self.rows().iter().map(RowCodec::key)
    }

    pub fn get(&self, key: u128) -> io::Result<Option<Cow<'a, R>>> {
        let rows = self.rows();
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| Cow::Borrowed(&rows[at])))
    }

    pub fn texts(&self, field: fn(&R) -> &[u8], column: usize) -> impl Iterator<Item = io::Result<&'a [u8]>> + use<'a, R> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        rows.iter().map(move |row| Ok(field(row)))
    }

    pub fn keyed<'k>(&self, mut keep: impl FnMut(u128) -> bool + 'k) -> Box<dyn Iterator<Item = io::Result<Cow<'a, R>>> + 'k>
    where
        'a: 'k,
        R: 'k,
    {
        Box::new(self.rows().iter().filter(move |row| keep(row.key())).map(|row| Ok(Cow::Borrowed(row))))
    }

    pub fn range(&self, keys: std::ops::RangeInclusive<u128>) -> RowsetIter<'a, R> {
        let (low, high) = (*keys.start(), *keys.end());
        let rows = self.rows();
        let first = rows.partition_point(|row| row.key() < low);
        let end = first + rows[first..].partition_point(|row| row.key() <= high);
        RowsetIter(rows[first..end].iter())
    }

    pub fn contains(&self, key: u128) -> bool {
        self.rows().binary_search_by_key(&key, RowCodec::key).is_ok()
    }
}

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
    pub fn u32s(&self, key: u128, field: fn(&R) -> &[u32], column: usize) -> io::Result<Option<&'a [u32]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    pub fn u64s(&self, key: u128, field: fn(&R) -> &[u64], column: usize) -> io::Result<Option<&'a [u64]>> {
        let rows = self.rows();
        super::undeclared::column_read(rows.as_ptr() as usize, column);
        Ok(rows.binary_search_by_key(&key, RowCodec::key).ok().map(|at| field(&rows[at])))
    }

    pub fn within(self, at: std::ops::Range<usize>) -> Self {
        Self::Decoded(&self.rows()[at])
    }

    pub fn with_text(self, field: fn(&R) -> &[u8], column: usize) -> impl Iterator<Item = io::Result<(Cow<'a, R>, &'a [u8])>> + use<'a, R> {
        self.iter().zip(self.texts(field, column)).map(|(row, text)| Ok((row?, text?)))
    }

    pub fn nth(&self, at: usize) -> io::Result<Option<Cow<'a, R>>> {
        Ok(self.rows().get(at).map(Cow::Borrowed))
    }

    pub fn slice(self, low: u128, high: u128) -> io::Result<Cow<'a, [R]>>
    where
        R: Clone,
    {
        Ok(Cow::Borrowed(self.span(low, high).rows()))
    }

    pub fn position_of(&self, key: u128) -> Option<usize> {
        self.rows().binary_search_by_key(&key, RowCodec::key).ok()
    }
}

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

    pub fn decoded(self) -> Option<&'a dyn DecodedSource> {
        Some(self.frame())
    }

    pub fn status(self, table: TableId) -> OperandStatus<'a> {
        self.frame().status(table)
    }

    pub fn sampled_at(self, table: TableId) -> Option<Hour> {
        self.frame().sampled_at(table.index())
    }

    pub fn identity(self, table: TableId) -> Option<super::ViewIdentity> {
        self.frame().identity(table.index())
    }

    pub fn rows<T: TableSchema>(self, mask: u64) -> Option<Rowset<'a, T::Row>> {
        <dyn DecodedSource as Tables>::rows::<T>(self.frame(), mask)
    }

    pub fn held<T: TableSchema>(self, mask: u64) -> Rowset<'a, T::Row> {
        self.rows::<T>(mask).unwrap_or(Rowset::Decoded(&[]))
    }

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

pub trait Tables {
    fn of_branch(&self) -> BranchId;
    fn at_hour(&self) -> Hour;
    fn metrics(&self) -> &Metrics;
    fn table_status(&self, table: TableId) -> OperandStatus<'_>;
    fn table_sampled_at(&self, table: TableId) -> Option<Hour>;
    fn rows<T: TableSchema>(&self, mask: u64) -> Option<Rowset<'_, T::Row>>;
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
        super::undeclared::status(|| OperandStatus::for_table(&self.tables[table.index()], table))
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
