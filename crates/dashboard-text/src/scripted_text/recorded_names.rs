//! The hour's country names, read through recorded queries
//! ([`crate::triggers::inputs`]): each lookup records its country and the
//! digest of what it returned.

use crate::triggers::inputs::{Key, digest, digest_debug, query, query_debug};
use installed_content_country_names::{country_names::CountryNames, country_names::Operands};

pub(crate) struct RecordedNames<'a>(pub(super) CountryNames<'a>);

/// A recorded query of the hour's country names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    Operands(u32),
    Name(u32),
    NameWith(u32, String),
    RulingGroup(u32),
    /// Which of slot zero's names its reserved row had built.
    ReservedBuilt,
}

impl RecordedNames<'_> {
    pub(crate) fn operands(&self, slot: u32) -> Option<&Result<Operands, String>> {
        query_debug(
            || Key::Names(Query::Operands(slot)),
            || self.0.operands(slot),
        )
    }

    pub(crate) fn name(&self, slot: u32) -> Result<String, String> {
        query(|| Key::Names(Query::Name(slot)), || self.0.name(slot))
    }

    pub(crate) fn name_with(&self, slot: u32, suffix: &str) -> Result<String, String> {
        query(
            || Key::Names(Query::NameWith(slot, suffix.to_owned())),
            || self.0.name_with(slot, suffix),
        )
    }

    pub(crate) fn ruling_group(&self, slot: u32) -> Result<Option<&str>, String> {
        query(
            || Key::Names(Query::RulingGroup(slot)),
            || self.0.ruling_group(slot),
        )
    }

    pub(crate) fn reserved_built(&self) -> Option<u8> {
        query(
            || Key::Names(Query::ReservedBuilt),
            || self.0.reserved_built(),
        )
    }

    /// The digest `query` reads in this hour's names.
    pub fn reread(&self, query: &Query) -> u64 {
        match query {
            Query::Operands(slot) => digest_debug(&self.0.operands(*slot)),
            Query::Name(slot) => digest(&self.0.name(*slot)),
            Query::NameWith(slot, suffix) => digest(&self.0.name_with(*slot, suffix)),
            Query::RulingGroup(slot) => digest(&self.0.ruling_group(*slot)),
            Query::ReservedBuilt => digest(&self.0.reserved_built()),
        }
    }
}
