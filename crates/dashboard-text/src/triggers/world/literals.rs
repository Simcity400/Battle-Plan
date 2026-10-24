use replay_data::rows::LexicalTokenRow;
use std::collections::BTreeMap;

pub(super) enum Lookup<'a> {
    Sorted(replay_frame::frame::Rows<'a, LexicalTokenRow>),
    Map(BTreeMap<u32, &'a [u8]>),
}

impl<'a> Lookup<'a> {
    pub(super) fn new(rows: impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>) -> Self {
        let rows = rows.into();
        if rows.is_sorted_by(|a, b| a.token < b.token) {
            Self::Sorted(rows)
        } else {
            // Preserve last-row wins for unordered or duplicate tokens, including zero.
            Self::Map(
                rows.iter()
                    .map(|row| (row.token, row.literal.as_slice()))
                    .collect(),
            )
        }
    }

    pub(super) fn get(&self, token: u32) -> Option<&'a [u8]> {
        match self {
            Self::Sorted(rows) => rows
                .binary_search_by_key(&token, |row| row.token)
                .ok()
                .map(|index| rows.get(index).unwrap().literal.as_slice()),
            Self::Map(rows) => rows.get(&token).copied(),
        }
    }
}
