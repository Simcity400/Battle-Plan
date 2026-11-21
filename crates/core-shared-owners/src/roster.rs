use crate::{Dated, Hour, Interval, Lookup, Owner, Reason};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Link(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountryId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag<'a>(pub &'a [u8]);

impl<'a> Tag<'a> {
    pub fn text(self) -> Option<&'a str> {
        std::str::from_utf8(self.0).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RosterKey {
    Link(Link),
    Holders(Slot),
    NameTag(Slot),
    Country(CountryId),
    OriginalTag(CountryId),
    ReservedZero,
    Epoch,
    Census,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change<K> {
    pub hour: Hour,
    pub key: K,
}

pub trait Roster: Owner {
    fn slot_of(&self, link: Link, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Slot>>>>;

    fn country(&self, country: CountryId, hour: Hour)
    -> io::Result<Lookup<'_, Dated<Option<Slot>>>>;

    fn countries<'a>(
        &'a self,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, usize>>;

    fn holders<'a>(
        &'a self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Tag<'a>) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, Dated<usize>>>;

    fn name_tag(&self, slot: Slot, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Tag<'_>>>>>;

    fn original_tag(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>>;

    fn reserved_zero_original_tag(
        &self,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>>;

    fn tagged(
        &self,
        tag: Tag<'_>,
        hour: Hour,
        visit: &mut dyn FnMut(Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    fn links(
        &self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(Link) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    fn epoch(&self, hour: Hour) -> io::Result<Lookup<'_, Dated<u64>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<RosterKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}
