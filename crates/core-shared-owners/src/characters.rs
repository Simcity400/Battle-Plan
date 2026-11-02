use crate::{Change, CountryId, Dated, Hour, Interval, Lookup, Owner, Reason, Slot, Tag, Token};
use std::io;

/// A character's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharacterId(pub u32);

/// The fields a character's name is built from; its token's spelling comes from
/// [`crate::Spellings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterNaming {
    pub name_token_id: u32,
    pub token: Token,
    pub template_present: bool,
    pub template_tag: u32,
    pub template_file: u32,
    pub template_line: u32,
    pub template_generated: bool,
}

/// Character naming with its history, so a name can be read at an earlier hour.
pub trait Characters: Owner {
    /// The first hour the owner still answers; asking earlier is an error. It retains at least
    /// what its consumers declare.
    fn retained(&self) -> Option<Hour>;

    /// Known none where a complete Characters table holds no such character.
    fn character(
        &self,
        id: CharacterId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<CharacterNaming>>>>;

    /// The characters whose naming changed in `hours`, in hour order, including a change of
    /// their token's spelling.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CharacterId>) -> io::Result<()>,
    ) -> io::Result<()>;
}

/// A country's appearance as its name reads it; empty tags stay empty, as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamingLook<'a> {
    pub name_tag: Tag<'a>,
    pub cosmetic_tag: Tag<'a>,
    /// Its original tag, else its own, resolved through the reference and reserved slot zero.
    pub original_or_own_tag: Result<Tag<'a>, Reason<'a>>,
    /// False for a country that isn't dynamic; refused only by the civil-war resolution.
    pub civil_war: Result<bool, Reason<'a>>,
}

/// A country as its name is built: its slot, and its appearance where it has a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryNamingFacts<'a> {
    pub slot: Slot,
    /// None where the country has no CountryAppearance row: a known absence.
    pub look: Option<NamingLook<'a>>,
}

/// Country naming with civil wars, in main's row order and error precedence. Each field's
/// result stands alone, and a reason doesn't carry the hour: the family adds the hour it asks
/// at, so a lasting failure isn't a change every hour. Missing only where the inputs are
/// unrecorded or incomplete. War end hours are the owner's own deadlines.
pub trait CountryNaming: Owner {
    fn naming(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<CountryNamingFacts<'_>>>>;

    /// The countries whose naming changed in `hours`, in hour order.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CountryId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
