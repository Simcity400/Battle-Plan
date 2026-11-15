//! Prepared row and value codecs, independent of frame envelopes.
pub use replay_rows::checkpoint::{rows, values};

pub mod endpoint {
    //! Absolute typed patches. Recording wire deltas never enter this codec.
    use crate::rows::TableRows;
    pub use replay_rows::checkpoint::endpoint::*;
    use std::fmt;

    pub struct EndpointRows {
        pub(crate) rows: TableRows,
        pub(crate) edits: Vec<Edit>,
    }
    impl EndpointRows {
        pub fn len(&self) -> usize {
            self.edits.len()
        }
        pub fn is_empty(&self) -> bool {
            self.edits.is_empty()
        }
        pub fn edits(&self) -> impl ExactSizeIterator<Item = (u128, u64, bool)> + '_ {
            self.edits
                .iter()
                .map(|edit| (edit.key, edit.mask, edit.replace))
        }
        pub fn inserts(&self) -> bool {
            self.edits.iter().any(|edit| edit.replace)
        }
    }
    impl fmt::Debug for EndpointRows {
        fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.debug_struct("EndpointRows")
                .field("table", &self.rows.table())
                .field("len", &self.len())
                .finish()
        }
    }

    #[cfg(test)]
    mod tests;
}
