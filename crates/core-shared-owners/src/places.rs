use crate::{Change, Dated, Hour, Interval, Lookup, Owner, Reason};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProvinceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceFacts {
    pub state: u32,
    pub controller: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateFacts {
    pub owner: u32,
    pub controller: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceKey {
    Province(ProvinceId),
    State(StateId),
}

pub trait Ownership: Owner {
    fn province(
        &self,
        id: ProvinceId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<ProvinceFacts>>>>;

    fn state(&self, id: StateId, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<StateFacts>>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<PlaceKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateNameUsage {
    Place,
    Occupation,
    Raid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError<'a> {
    Stored(Reason<'a>),
    Templates(Reason<'a>),
    NoTemplate,
    TemplateRefused { key: &'a str, reason: Reason<'a> },
    NotPlain(Reason<'a>),
}

pub trait StateNames: Owner {
    fn state_name(
        &self,
        id: StateId,
        usage: StateNameUsage,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<&str, NameError<'_>>>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<StateId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
