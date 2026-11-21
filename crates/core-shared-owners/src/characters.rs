use crate::{Change, CountryId, Dated, Hour, Interval, Lookup, Owner, Reason, Slot, Tag, Token};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharacterId(pub u32);

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

pub trait Characters: Owner {
    fn retained(&self) -> Option<Hour>;

    fn character(
        &self,
        id: CharacterId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<CharacterNaming>>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CharacterId>) -> io::Result<()>,
    ) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamingLook<'a> {
    pub name_tag: Tag<'a>,
    pub cosmetic_tag: Tag<'a>,
    pub original_or_own_tag: Result<Tag<'a>, Reason<'a>>,
    pub civil_war: Result<bool, Reason<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryNamingFacts<'a> {
    pub slot: Slot,
    pub look: Option<NamingLook<'a>>,
}

pub trait CountryNaming: Owner {
    fn naming(
        &self,
        country: CountryId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<CountryNamingFacts<'_>>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<CountryId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
