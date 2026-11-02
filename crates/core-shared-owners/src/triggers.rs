use crate::{Change, Hour, Interval, Lookup, Owner};
use core_events::{FieldId, Key, TableId};
use std::io;

/// A scripted text, compiled from the content the recorded mod stack loads: a trigger block
/// or a scripted localisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextId(pub u32);

/// A registered query of a text; the context allocates it, so families never collide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryId(pub u64);

/// A field a text can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRead {
    pub table: TableId,
    pub field: FieldId,
}

/// What one evaluation read of a field: every row of it, or one row, present or absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadScope<'a> {
    Field,
    Key(Key<'a>),
}

/// One read of a query's latest evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryRead<'a> {
    pub read: TextRead,
    pub scope: ReadScope<'a>,
}

/// The trigger context: one per segment, retaining the static union of every holdable text's
/// declared reads, so a newly registered query is always hydrated exactly. It is a dependency
/// registry, not an evaluator: families evaluate their own queries on their own line, report
/// what each evaluation read, and the context tells them when a read changed or a deadline fell
/// due.
pub trait TriggerContext: Owner {
    /// Visits `text`'s reads from its compiled content, each with whether the context retains
    /// it; known with their count, missing where the content can't compile the text.
    fn reads(
        &self,
        text: TextId,
        visit: &mut dyn FnMut(TextRead, bool) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    /// Registers a query of `text`, invalidated by nothing until its first footprint.
    fn register(&mut self, text: TextId) -> io::Result<QueryId>;

    /// Unregisters a query; one not registered is an error.
    fn unregister(&mut self, query: QueryId) -> io::Result<()>;

    /// Replaces the query's footprint with what its evaluation at `hour` read, and the later
    /// hours at which an unchanged read can change its value, such as a relation's end hour.
    fn footprint(
        &mut self,
        query: QueryId,
        hour: Hour,
        reads: &[QueryRead<'_>],
        deadlines: &[Hour],
    ) -> io::Result<()>;

    /// Visits the query's text's reads the context doesn't retain, so the family can price
    /// evaluating them; known with their count, missing where the text can't compile.
    fn undeclared(
        &self,
        query: QueryId,
        visit: &mut dyn FnMut(TextRead) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    /// The query's first deadline after `hour`; known none without one, missing where the
    /// text can't compile.
    fn deadline(&self, query: QueryId, hour: Hour) -> io::Result<Lookup<'_, Option<Hour>>>;

    /// The registered queries invalidated in `hours`, in hour order: a read in their footprint
    /// changed, or one of their deadlines fell due.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<QueryId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
