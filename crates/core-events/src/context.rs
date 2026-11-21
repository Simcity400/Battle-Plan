use crate::{
    CellValue, DeclarationId, FieldId, Hour, IndexId, Key, Missing, ObservationStream, ProductCell,
    ProductFieldId, RelationId, TableId, Timer,
};
use std::{fmt, io};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<'a, T> {
    Known(T),
    Missing(Missing<'a>),
}

pub trait RowValues: Send + Sync {
    fn value(&self, hour: Hour, slot: usize, field: FieldId) -> io::Result<CellValue<'_>>;
}

#[derive(Clone, Copy)]
pub struct RowView<'a> {
    hour: Hour,
    slot: usize,
    fields: &'a [FieldId],
    values: &'a dyn RowValues,
}

impl<'a> RowView<'a> {
    pub fn new(hour: Hour, slot: usize, fields: &'a [FieldId], values: &'a dyn RowValues) -> Self {
        Self {
            hour,
            slot,
            fields,
            values,
        }
    }
    pub fn hour(self) -> Hour {
        self.hour
    }
    pub fn fields(self) -> &'a [FieldId] {
        self.fields
    }
    pub fn value(self, field: FieldId) -> io::Result<CellValue<'a>> {
        if !self.fields.contains(&field) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.values.value(self.hour, self.slot, field)
    }
}

impl fmt::Debug for RowView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowView")
            .field("hour", &self.hour)
            .field("slot", &self.slot)
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    pub key: Key<'a>,
    pub slot: usize,
    pub fields: RowView<'a>,
}

pub trait Rows {
    fn visit(&self, visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictionaryString<'a> {
    pub id: u32,
    pub value: &'a str,
}

pub enum FilterInsertion<'a> {
    Unchanged,
    Appeared(Row<'a>),
    Missing(Missing<'a>),
}

pub trait CallContext {
    fn hour(&self) -> Hour;
    fn observation(&self, declaration: DeclarationId) -> io::Result<&dyn ObservationStream>;
    fn exact(&self, table: TableId, key: Key<'_>, field: FieldId) -> io::Result<CellValue<'_>>;
    fn previous(&self, table: TableId, key: Key<'_>, field: FieldId) -> io::Result<CellValue<'_>>;
    fn historical(
        &self,
        index: IndexId,
        key: Key<'_>,
        field: ProductFieldId,
        hour: Hour,
    ) -> io::Result<ProductCell<'_>>;
    fn prefix(
        &self,
        declaration: DeclarationId,
        components: &[Key<'_>],
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>>;
    fn secondary(
        &self,
        declaration: DeclarationId,
        value: Key<'_>,
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>>;
    fn members(
        &self,
        relation: RelationId,
        value: Key<'_>,
        visitor: &mut dyn FnMut(Key<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>>;
    fn snapshot(&self, table: TableId) -> io::Result<Lookup<'_, &dyn Rows>>;
    fn take_watch(&mut self, declaration: DeclarationId) -> io::Result<bool>;
    fn schedule(&mut self, timer: Timer<'_>) -> io::Result<()>;
    fn cancel(&mut self, id: Key<'_>) -> io::Result<bool>;
    fn close_window(&mut self, rule: DeclarationId, key: Key<'_>) -> io::Result<()>;
    fn insert_filter(
        &mut self,
        declaration: DeclarationId,
        key: Key<'_>,
    ) -> io::Result<FilterInsertion<'_>>;
    fn remove_filter(&mut self, declaration: DeclarationId, key: Key<'_>) -> io::Result<bool>;
    fn dictionary(&self, id: u32) -> io::Result<Lookup<'_, &str>>;
    fn dictionary_additions(&self) -> &[DictionaryString<'_>];
}
