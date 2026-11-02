use crate::{Dated, Hour, Interval, Lookup, Owner, Reason};
use std::io;

/// CountryLinks' id: the handle a row names a country by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Link(pub u32);

/// A country's canonical slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot(pub u32);

/// Countries' id: one native country identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountryId(pub u32);

/// A tag's recorded bytes; a tag that isn't UTF-8 is kept as recorded, never replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag<'a>(pub &'a [u8]);

impl<'a> Tag<'a> {
    pub fn text(self) -> Option<&'a str> {
        std::str::from_utf8(self.0).ok()
    }
}

/// What a roster change touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RosterKey {
    /// The slot a link names.
    Link(Link),
    /// A slot's Countries rows.
    Holders(Slot),
    /// A slot's name tag.
    NameTag(Slot),
    /// A Countries row's own canonical slot.
    Country(CountryId),
    /// A country's original tag.
    OriginalTag(CountryId),
    /// Reserved slot zero's original tag.
    ReservedZero,
    /// The identity epoch.
    Epoch,
    /// The census became complete or was lost: reverse lookups and absences change with it.
    Census,
}

/// A key whose answer changed at `hour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change<K> {
    pub hour: Hour,
    pub key: K,
}

/// The tag-by-link roster: links to canonical slots, each slot's Countries rows and tags, with
/// identity history. Each family reads the mode it needs from the rows: the last row's tag,
/// or a slot's unique holder. Visited values borrow the owner, as answers do.
pub trait Roster: Owner {
    /// The slot `link` names; known none where a complete CountryLinks has no such link.
    fn slot_of(&self, link: Link, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Slot>>>>;

    /// A Countries row's own canonical slot, which a link may alias to another slot; known
    /// none where a complete Countries has no such row.
    fn country(&self, country: CountryId, hour: Hour)
    -> io::Result<Lookup<'_, Dated<Option<Slot>>>>;

    /// Visits the live Countries rows at `hour`, ascending by id, each with its own slot;
    /// known with their count once a complete census holds, missing before.
    fn countries<'a>(
        &'a self,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, usize>>;

    /// Visits the Countries rows at `slot` in table order; known with their count, which is 0
    /// where the slot is known to have no holder.
    fn holders<'a>(
        &'a self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(CountryId, Tag<'a>) -> io::Result<()>,
    ) -> io::Result<Lookup<'a, Dated<usize>>>;

    /// The tag a dynamic country at `slot` is named after (CountryAppearance's name tag);
    /// known none where it isn't dynamic.
    fn name_tag(&self, slot: Slot, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<Tag<'_>>>>>;

    /// A country's original tag, from CountryAppearance's original tag and its reference;
    /// refused with main's reason where they don't resolve.
    fn original_tag(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>>;

    /// Reserved slot zero's original tag, which names owner 0; refused with main's reason.
    fn reserved_zero_original_tag(
        &self,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<Tag<'_>, Reason<'_>>>>>;

    /// Visits the slots whose rows carry `tag`, by their first holder's id; known with their
    /// count once a complete census holds, missing before.
    fn tagged(
        &self,
        tag: Tag<'_>,
        hour: Hour,
        visit: &mut dyn FnMut(Slot) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    /// Visits the links naming `slot`, ascending; known with their count once a complete
    /// census holds, missing before.
    fn links(
        &self,
        slot: Slot,
        hour: Hour,
        visit: &mut dyn FnMut(Link) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, usize>>;

    /// The identity epoch: it moves wherever country identities are rebuilt, so their
    /// dependents rebuild even where no single key changed.
    fn epoch(&self, hour: Hour) -> io::Result<Lookup<'_, Dated<u64>>>;

    /// The keys whose answers changed in `hours`, in hour order.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<RosterKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}
