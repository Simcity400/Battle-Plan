//! Checked conversion from bounded native rows to recorded table outcomes.

use engine::walk::Outcome;
use replay::{
    frame::TableView,
    rows::{RowCodec, TableRows, TableSchema},
};

pub fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: if crate::allocation::failed() {
            String::new()
        } else {
            // prepare_frame and write_prepared reject the latched terminal
            // state before any unavailable fallback can be published.
            engine::allocation::string(reason).unwrap_or_default()
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
            let mut rows = match engine::allocation::vec(len) {
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
