
use crate::triggers::inputs::{Key, digest, fingerprint, query, query_typed};
use content_reader_country_names::{country_names::CountryNames, country_names::Operands};

pub(crate) struct RecordedNames<'a>(pub(super) CountryNames<'a>);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Query {
    Operands(u32),
    Name(u32),
    NameWith(u32, String),
    RulingGroup(u32),
    ReservedBuilt,
}

impl RecordedNames<'_> {
    pub(crate) fn operands(&self, slot: u32) -> Option<&Result<Operands, String>> {
        query_typed(
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

    pub fn reread(&self, query: &Query) -> u64 {
        match query {
            Query::Operands(slot) => fingerprint(&self.0.operands(*slot)),
            Query::Name(slot) => digest(&self.0.name(*slot)),
            Query::NameWith(slot, suffix) => digest(&self.0.name_with(*slot, suffix)),
            Query::RulingGroup(slot) => digest(&self.0.ruling_group(*slot)),
            Query::ReservedBuilt => digest(&self.0.reserved_built()),
        }
    }
}
