
use engine_native_memory::walk::Outcome;
use replay_data::{rows::RowCodec, rows::TableRows, rows::TableSchema};
use replay_frame::frame::TableView;

pub fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: if crate::allocation::failed() {
            String::new()
        } else {
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

#[derive(Default)]
pub struct RowOrder {
    entries: Vec<(u128, usize)>,
}
impl RowOrder {
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(u128, usize)>()
    }

    pub fn table<S: TableSchema, T>(
        &mut self,
        outcome: Outcome,
        source: &[T],
        key: impl Fn(&T) -> u128,
        convert: impl Fn(&T) -> S::Row,
    ) -> TableView {
        crate::table_clock::timed(S::ID, || {
            self.ordered_rows::<S, T>(outcome, source, key, convert)
        })
    }

    fn ordered_rows<S: TableSchema, T>(
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
        let mut natural = true;
        if let Some((first, rest)) = source.split_first() {
            let mut prev = key(first);
            for item in rest {
                let next = key(item);
                if next <= prev {
                    natural = false;
                    break;
                }
                prev = next;
            }
        }
        if !natural {
            self.entries.clear();
            if self.entries.capacity() < source.len()
                && engine_allocation::reserve_exact(&mut self.entries, source.len()).is_err()
            {
                return TableView::Unavailable {
                    reason: String::new(),
                };
            }
            for (index, item) in source.iter().enumerate() {
                self.entries.push((key(item), index));
            }
            self.entries.sort_unstable_by_key(|&(k, _)| k);
            if self.entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
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
        if natural {
            for item in source {
                rows.push(convert(item));
                if crate::allocation::failed() {
                    return TableView::Unavailable {
                        reason: String::new(),
                    };
                }
            }
        } else {
            for &(_, index) in &self.entries {
                rows.push(convert(&source[index]));
                if crate::allocation::failed() {
                    return TableView::Unavailable {
                        reason: String::new(),
                    };
                }
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

pub fn table_iter<S: TableSchema, T>(
    outcome: Outcome,
    source: impl Iterator<Item = T>,
    convert: impl Fn(T) -> S::Row,
) -> TableView {
    crate::table_clock::timed(S::ID, || iter_rows::<S, T>(outcome, source, convert))
}

fn iter_rows<S: TableSchema, T>(
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
            if !rows.windows(2).all(|pair| pair[0].key() < pair[1].key()) {
                rows.sort_unstable_by_key(RowCodec::key);
                if rows.windows(2).any(|rows| rows[0].key() == rows[1].key()) {
                    return unavailable("walker produced duplicate engine ids");
                }
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
