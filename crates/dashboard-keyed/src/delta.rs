//! An hour's values as changes from the last hour observed, and the row comparisons that find them.

use replay_data::rows::RowCodec;
use replay_frame::frame::{SharedView, TableView};
use replay_data::rows::TableSchema;

/// Names an hour no longer holds, then names it holds in whole-observation push order.
#[derive(Default)]
pub struct Delta {
    pub(crate) removed: Vec<String>,
    pub(crate) set: Vec<(String, i128)>,
}

impl Delta {
    /// `name` no longer holds a value, unless the hour also sets it.
    pub fn remove(&mut self, name: String) {
        self.removed.push(name);
    }

    pub fn set(&mut self, name: String, value: i128) {
        self.set.push((name, value));
    }

    pub fn clear(&mut self) {
        self.removed.clear();
        self.set.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.set.is_empty()
    }

    pub fn held(&self, bytes: &mut dashboard_base::prepared::prepass::HeldBytes) {
        bytes
            .heap(&self.removed)
            .add(self.set.capacity() * size_of::<(String, i128)>());
        for (name, _) in &self.set {
            bytes.heap(name);
        }
    }
}

/// The names a scan has numbered: those an observation has read before.
pub struct Names<'a>(pub(crate) &'a std::collections::HashMap<String, u64>);

impl Names<'_> {
    pub fn numbered(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }
}

/// The rows of `T` a view holds when it is complete.
pub fn rows<T: TableSchema>(view: &SharedView) -> Option<&[T::Row]> {
    match &**view {
        TableView::Complete(rows) => rows.rows::<T>(),
        _ => None,
    }
}

/// Visit each key whose rows differ by `same`, in key order; None when keys are out of order.
pub fn changed_rows<'a, R: RowCodec>(
    before: &'a [R],
    now: &'a [R],
    same: impl Fn(&R, &R) -> bool,
    mut visit: impl FnMut(Option<&'a R>, Option<&'a R>),
) -> Option<()> {
    let (mut a, mut b) = (0, 0);
    let mut last: (Option<u128>, Option<u128>) = (None, None);
    loop {
        let old = before.get(a);
        let new = now.get(b);
        let old_key = old.map(RowCodec::key);
        let new_key = new.map(RowCodec::key);
        if old_key.is_some_and(|key| last.0.is_some_and(|held| held >= key))
            || new_key.is_some_and(|key| last.1.is_some_and(|held| held >= key))
        {
            return None;
        }
        match (old_key, new_key) {
            (None, None) => return Some(()),
            (Some(x), Some(y)) if x == y => {
                if !same(old?, new?) {
                    visit(old, new);
                }
                last = (Some(x), Some(y));
                a += 1;
                b += 1;
            }
            (Some(x), y) if y.is_none_or(|y| x < y) => {
                visit(old, None);
                last.0 = Some(x);
                a += 1;
            }
            (_, Some(y)) => {
                visit(None, new);
                last.1 = Some(y);
                b += 1;
            }
            _ => return None,
        }
    }
}

/// Whether `now` holds `before`'s rows as `same` compares them, or both are incomplete.
pub fn same_rows<T: TableSchema>(
    before: Option<&SharedView>,
    now: &SharedView,
    same: impl Fn(&T::Row, &T::Row) -> bool,
) -> bool {
    if before.is_some_and(|before| SharedView::ptr_eq(before, now)) {
        return true;
    }
    match (before.and_then(rows::<T>), rows::<T>(now)) {
        (None, None) => true,
        (Some(before), Some(now)) => {
            let mut equal = true;
            changed_rows(before, now, same, |_, _| equal = false).is_some() && equal
        }
        _ => false,
    }
}

/// The rows of `rows` (ascending by key) whose keys lie in `range`.
pub fn key_range<R: RowCodec>(rows: &[R], range: std::ops::Range<u128>) -> &[R] {
    let first = rows.partition_point(|row| row.key() < range.start);
    let count = rows[first..].partition_point(|row| row.key() < range.end);
    &rows[first..first + count]
}

/// The row of `rows` (ascending by key) at `key`.
pub fn at_key<R: RowCodec>(rows: &[R], key: u128) -> Option<&R> {
    rows.binary_search_by_key(&key, RowCodec::key)
        .ok()
        .map(|at| &rows[at])
}
