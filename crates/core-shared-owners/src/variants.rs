use crate::{Change, Dated, Hour, Interval, Lookup, Owner, Reason};
use core_events::ObservationRun;
use std::io;

/// The variant owner's id: the same in every family and independent of any family's batching.
/// Across segments the change log's typed references reconcile it, never a family remap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantId(pub u32);

/// VariantDefinitions' id: the game's handle of a design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(pub u64);

/// The identity of a variant's naming operands: equal exactly where the operands are, so a
/// family compares versions without the operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperandsId(pub u64);

/// A variant's naming operands as one complete read gave them, named through the recorded mod
/// stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariantVersion<'a> {
    pub operands: OperandsId,
    /// The equipment type token's spelling; none where no read spelled it.
    pub token: Option<&'a str>,
    /// The design's own name; none where it has none of its own, refused with its reason.
    pub name: Result<Option<&'a str>, Reason<'a>>,
    pub design: Option<&'a str>,
}

/// A complete naming read: every definition's operands are known from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NamingRead {
    /// A definitions census: VariantDefinitions' supplying read.
    Census,
    /// A complete, non-empty VariantChanges ledger between censuses.
    Ledger,
}

/// What a variant-owner change touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantKey {
    /// A variant's operands changed, or a read no longer defines it.
    Version(VariantId),
    /// A complete naming read; the first after a forget makes every version known again.
    Read(NamingRead),
    /// Names were forgotten, every version with them: a gap or an incomplete naming input.
    Forgotten,
}

/// Variant identity and naming, one owner for every family (lead decision 2026-11-03). It holds
/// operands and their read evidence; each family keeps its own selection and dating rules.
pub trait Variants: Owner {
    /// The owner's id of `handle`, assigned at its first read.
    fn id(&self, handle: Handle) -> Option<VariantId>;

    fn handle(&self, id: VariantId) -> Option<Handle>;

    /// `id`'s operands at `hour`, dated at the hour they last changed: an unchanged version
    /// keeps its date across a forget. Missing while names are forgotten or no read defines it.
    fn version(
        &self,
        id: VariantId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<VariantVersion<'_>>>>;

    /// The first complete naming read since names were last forgotten, as of `hour`; missing
    /// while they are forgotten, with the forget's hours and reason.
    fn known_since(&self, hour: Hour) -> io::Result<Lookup<'_, Hour>>;

    /// The last definitions census at or before `hour`; none before the first.
    fn census(&self, hour: Hour) -> io::Result<Option<Hour>>;

    /// Visits the complete naming reads in `hours`, in hour order.
    fn reads(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Hour, NamingRead) -> io::Result<()>,
    ) -> io::Result<()>;

    /// Visits the truth of "VariantChanges is complete and non-empty" over `hours`, as runs.
    fn ledger<'a>(
        &'a self,
        hours: Interval,
        visit: &mut dyn FnMut(ObservationRun<'a>) -> io::Result<()>,
    ) -> io::Result<()>;

    /// The keys whose answers changed in `hours`, in hour order.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<VariantKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}
