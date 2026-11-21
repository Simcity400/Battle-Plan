use crate::{Change, Dated, Hour, Interval, Lookup, Owner, Reason};
use core_events::ObservationRun;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariantId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperandsId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VariantVersion<'a> {
    pub operands: OperandsId,
    pub token: Option<&'a str>,
    pub name: Result<Option<&'a str>, Reason<'a>>,
    pub design: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NamingRead {
    Census,
    Ledger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariantKey {
    Version(VariantId),
    Read(NamingRead),
    Forgotten,
}

pub trait Variants: Owner {
    fn id(&self, handle: Handle) -> Option<VariantId>;

    fn handle(&self, id: VariantId) -> Option<Handle>;

    fn version(
        &self,
        id: VariantId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<VariantVersion<'_>>>>;

    fn known_since(&self, hour: Hour) -> io::Result<Lookup<'_, Hour>>;

    fn census(&self, hour: Hour) -> io::Result<Option<Hour>>;

    fn reads(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Hour, NamingRead) -> io::Result<()>,
    ) -> io::Result<()>;

    fn ledger<'a>(
        &'a self,
        hours: Interval,
        visit: &mut dyn FnMut(ObservationRun<'a>) -> io::Result<()>,
    ) -> io::Result<()>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<VariantKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}
