use crate::{Change, Dated, Hour, Interval, Lookup, Owner, Reason};
use std::io;

/// A province's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProvinceId(pub u32);

/// A state's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateId(pub u32);

/// A province's recorded state and controller; a state of 0 is recorded so and kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceFacts {
    pub state: u32,
    pub controller: u32,
}

/// A state's recorded owner and controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateFacts {
    pub owner: u32,
    pub controller: u32,
}

/// What an ownership change touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlaceKey {
    Province(ProvinceId),
    State(StateId),
}

/// State and province ownership, read by key.
pub trait Ownership: Owner {
    /// Known none where a complete Provinces table holds no such province; missing where the
    /// table is incomplete or the cell unrecorded.
    fn province(
        &self,
        id: ProvinceId,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Option<ProvinceFacts>>>>;

    /// Known none where a complete States table holds no such state.
    fn state(&self, id: StateId, hour: Hour) -> io::Result<Lookup<'_, Dated<Option<StateFacts>>>>;

    /// The keys whose answers changed in `hours`, in hour order.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<PlaceKey>) -> io::Result<()>,
    ) -> io::Result<()>;
}

/// Where a state's name is shown: each use refuses in its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateNameUsage {
    Place,
    Occupation,
    Raid,
}

/// Why a state has no name, as main tells the cases apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError<'a> {
    /// The stored custom or owner name can't be read in the recorded language.
    Stored(Reason<'a>),
    /// The recorded content's state templates can't be read.
    Templates(Reason<'a>),
    /// No recorded-content template names the state.
    NoTemplate,
    /// The template's key is refused.
    TemplateRefused { key: &'a str, reason: Reason<'a> },
    /// The name isn't plain text.
    NotPlain(Reason<'a>),
}

/// The stored state name: the custom name, else the owner name, else the recorded content's
/// template, through the recorded language, as plain text.
pub trait StateNames: Owner {
    fn state_name(
        &self,
        id: StateId,
        usage: StateNameUsage,
        hour: Hour,
    ) -> io::Result<Lookup<'_, Dated<Result<&str, NameError<'_>>>>>;

    /// The states whose names changed in `hours`, in hour order: their custom or owner name,
    /// owner or template content changed.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<StateId>) -> io::Result<()>,
    ) -> io::Result<()>;
}
