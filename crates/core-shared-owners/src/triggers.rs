use crate::{Change, Hour, Interval, Lookup, Owner};
use core_events::{FieldId, Key, TableId};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRead {
    pub table: TableId,
    pub field: FieldId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadScope<'a> {
    Field,
    Key(Key<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryRead<'a> {
    pub read: TextRead,
    pub scope: ReadScope<'a>,
}

pub trait TriggerContext: Owner {
    fn reads(
        &self,
        text: TextId,
        visit: &mut dyn FnMut(TextRead, bool) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    fn register(&mut self, text: TextId) -> io::Result<QueryId>;

    fn unregister(&mut self, query: QueryId) -> io::Result<()>;

    fn footprint(
        &mut self,
        query: QueryId,
        hour: Hour,
        reads: &[QueryRead<'_>],
        deadlines: &[Hour],
    ) -> io::Result<()>;

    fn undeclared(
        &self,
        query: QueryId,
        visit: &mut dyn FnMut(TextRead) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    fn deadline(&self, query: QueryId, hour: Hour) -> io::Result<Lookup<'_, Option<Hour>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<QueryId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
