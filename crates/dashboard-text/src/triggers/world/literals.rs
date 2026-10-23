use replay_data::rows::LexicalTokenRow;
use std::collections::BTreeMap;

pub(super) enum Lookup<'a> {
    Sorted(&'a [LexicalTokenRow]),
    Map(BTreeMap<u32, &'a [u8]>),
}

impl<'a> Lookup<'a> {
    pub(super) fn new(rows: &'a [LexicalTokenRow]) -> Self {
        if rows.windows(2).all(|pair| pair[0].token < pair[1].token) {
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
                .map(|index| rows[index].literal.as_slice()),
            Self::Map(rows) => rows.get(&token).copied(),
        }
    }
}
