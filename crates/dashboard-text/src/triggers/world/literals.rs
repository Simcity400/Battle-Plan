use crate::triggers::Failure;
use replay_data::rows::LexicalTokenRow;
use replay_frame::frame::Rowset;
use std::collections::BTreeMap;

pub(super) enum Lookup<'a> {
    Sorted(Rowset<'a, LexicalTokenRow>),
    Map(BTreeMap<u32, &'a [u8]>),
}

impl<'a> Lookup<'a> {
    pub(super) fn new(rows: Rowset<'a, LexicalTokenRow>) -> Self {
        match rows {
            Rowset::Decoded(slice) if slice.is_sorted_by(|a, b| a.token < b.token) => {
                Self::Sorted(rows)
            }
            // Preserve last-row wins for unordered or duplicate tokens, including zero.
            Rowset::Decoded(slice) => Self::Map(
                slice
                    .iter()
                    .map(|row| (row.token, row.literal.as_slice()))
                    .collect(),
            ),
        }
    }

    pub(super) fn get(&self, token: u32) -> Result<Option<&'a [u8]>, Failure> {
        match self {
            Self::Sorted(rows) => super::literal(rows, token),
            Self::Map(rows) => Ok(rows.get(&token).copied()),
        }
    }
}
