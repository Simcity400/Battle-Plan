//! Checked conversion from bounded native rows to recorded table outcomes.

use engine_native::walk::Outcome;
use replay_data::{rows::RowCodec, rows::TableRows, rows::TableSchema};
use replay_frame::frame::TableView;

pub fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: if crate::allocation::failed() {
            String::new()
        } else {
            // prepare_frame and write_prepared reject the latched terminal
            // state before any unavailable fallback can be published.
            engine_allocation::string(reason).unwrap_or_default()
        },
    }
}

pub fn table<S: TableSchema, T>(
    outcome: Outcome,
    source: &[T],
    convert: impl Fn(&T) -> S::Row,
) -> TableView {
    let Some(source) = source.get(..outcome.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    table_iter::<S, _>(outcome, source.iter(), convert)
}

/// A reusable permutation, never a certificate for row values or membership.
/// Every conversion verifies strict replay-key order against the current source.
#[derive(Default)]
pub struct RowOrder {
    indices: Vec<usize>,
}
impl RowOrder {
    pub fn allocated_bytes(&self) -> usize {
        self.indices.capacity() * std::mem::size_of::<usize>()
    }

    /// `key` must be exactly the key produced by `convert`. Source storage is
    /// borrowed unchanged, including its native order and retained capacity.
    pub fn table<S: TableSchema, T>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableView {
        let Some(source) = source.get(..outcome.len()) else {
            return unavailable("walker row count exceeds allocated buffer");
        };
        if crate::allocation::failed() {
            return TableView::Unavailable {
                reason: String::new(),
            };
        }
        if let Outcome::Unavailable { reason } = outcome {
            return unavailable(reason);
        }
        let natural = source.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]));
        if !natural {
            if self.indices.len() != source.len() {
                self.indices.clear();
                if engine_allocation::reserve_exact(&mut self.indices, source.len()).is_err() {
                    return TableView::Unavailable {
                        reason: String::new(),
                    };
                }
                self.indices.extend(0..source.len());
            }
            if !self
                .indices
                .windows(2)
                .all(|pair| key(&source[pair[0]]) < key(&source[pair[1]]))
            {
                self.indices
                    .sort_unstable_by_key(|&index| key(&source[index]));
            }
            if self
                .indices
                .windows(2)
                .any(|pair| key(&source[pair[0]]) == key(&source[pair[1]]))
            {
                return unavailable("walker produced duplicate engine ids");
            }
        }
        let mut rows = match engine_allocation::vec(source.len()) {
            Ok(rows) => rows,
            Err(_) => {
                return TableView::Unavailable {
                    reason: String::new(),
                };
            }
        };
        for index in 0..source.len() {
            rows.push(convert(
                &source[if natural { index } else { self.indices[index] }],
            ));
            if crate::allocation::failed() {
                return TableView::Unavailable {
                    reason: String::new(),
                };
            }
        }
        let rows = TableRows::from_rows::<S>(rows);
        match outcome {
            Outcome::Truncated { unread_rows, .. } if unread_rows != 0 => TableView::Truncated {
                rows,
                missing: unread_rows,
            },
            Outcome::Truncated { .. } => {
                unavailable("walker marked truncation without a missing count")
            }
            _ => TableView::Complete(rows),
        }
    }
}

/// Convert borrowed rows without first materializing a parallel reference list.
pub fn table_iter<S: TableSchema, T>(
    outcome: Outcome,
    source: impl Iterator<Item = T>,
    convert: impl Fn(T) -> S::Row,
) -> TableView {
    if crate::allocation::failed() {
        return TableView::Unavailable {
            reason: String::new(),
        };
    }
    match outcome {
        Outcome::Unavailable { reason } => unavailable(reason),
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => {
            let mut rows = match engine_allocation::vec(len) {
                Ok(rows) => rows,
                Err(_) => {
                    return TableView::Unavailable {
                        reason: String::new(),
                    };
                }
            };
            for row in source {
                if rows.len() == len {
                    return unavailable("walker row count exceeds allocated buffer");
                }
                rows.push(convert(row));
                if crate::allocation::failed() {
                    return TableView::Unavailable {
                        reason: String::new(),
                    };
                }
            }
            if rows.len() != len {
                return unavailable("walker row count exceeds allocated buffer");
            }
            rows.sort_unstable_by_key(RowCodec::key);
            if rows.windows(2).any(|rows| rows[0].key() == rows[1].key()) {
                return unavailable("walker produced duplicate engine ids");
            }
            let rows = TableRows::from_rows::<S>(rows);
            match outcome {
                Outcome::Truncated { unread_rows, .. } if unread_rows != 0 => {
                    TableView::Truncated {
                        rows,
                        missing: unread_rows,
                    }
                }
                Outcome::Truncated { .. } => {
                    unavailable("walker marked truncation without a missing count")
                }
                _ => TableView::Complete(rows),
            }
        }
    }
}
