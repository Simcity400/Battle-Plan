//! Immutable, path-copied rows certified by writer observation. A changed
//! range costs logarithmic storage; quiet hours share the complete root.
use engine_allocation as allocation;
use replay_data::rows::{RowCodec, TableSchema};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static REVISION: AtomicU64 = AtomicU64::new(1);
#[derive(Debug)]
struct Node<R> {
    row: R,
    left: Link<R>,
    right: Link<R>,
    height: u32,
    len: usize,
    bytes: usize,
}
type Link<R> = Option<Arc<Node<R>>>;
fn height<R>(link: &Link<R>) -> u32 {
    link.as_ref().map_or(0, |n| n.height)
}
fn len<R>(link: &Link<R>) -> usize {
    link.as_ref().map_or(0, |n| n.len)
}
fn bytes<R>(link: &Link<R>) -> usize {
    link.as_ref().map_or(0, |n| n.bytes)
}
fn node<R: RowCodec>(row: R, left: Link<R>, right: Link<R>) -> Arc<Node<R>> {
    let height = 1 + height(&left).max(height(&right));
    let len = 1 + len(&left) + len(&right);
    let bytes = std::mem::size_of::<Node<R>>() + row.heap_bytes() + bytes(&left) + bytes(&right);
    Arc::new(Node {
        row,
        left,
        right,
        height,
        len,
        bytes,
    })
}
fn copy<R: RowCodec>(row: &R) -> Result<R, &'static str> {
    row.try_clone_validated()
        .map_err(|_| "certified row allocation failed")
}
fn balance<R: RowCodec>(
    row: R,
    mut left: Link<R>,
    mut right: Link<R>,
) -> Result<Arc<Node<R>>, &'static str> {
    if height(&left) > height(&right) + 1 {
        let l = left.as_ref().unwrap();
        if height(&l.right) > height(&l.left) {
            let r = l.right.as_ref().unwrap();
            left = Some(node(
                copy(&r.row)?,
                Some(node(copy(&l.row)?, l.left.clone(), r.left.clone())),
                r.right.clone(),
            ));
        }
        let l = left.as_ref().unwrap();
        return Ok(node(
            copy(&l.row)?,
            l.left.clone(),
            Some(node(row, l.right.clone(), right)),
        ));
    }
    if height(&right) > height(&left) + 1 {
        let r = right.as_ref().unwrap();
        if height(&r.left) > height(&r.right) {
            let l = r.left.as_ref().unwrap();
            right = Some(node(
                copy(&l.row)?,
                l.left.clone(),
                Some(node(copy(&r.row)?, l.right.clone(), r.right.clone())),
            ));
        }
        let r = right.as_ref().unwrap();
        return Ok(node(
            copy(&r.row)?,
            Some(node(row, left, r.left.clone())),
            r.right.clone(),
        ));
    }
    Ok(node(row, left, right))
}
fn put<R: RowCodec>(root: &Link<R>, row: R) -> Result<Arc<Node<R>>, &'static str> {
    let Some(n) = root else {
        return Ok(node(row, None, None));
    };
    Ok(match row.key().cmp(&n.row.key()) {
        std::cmp::Ordering::Less => {
            balance(copy(&n.row)?, Some(put(&n.left, row)?), n.right.clone())?
        }
        std::cmp::Ordering::Greater => {
            balance(copy(&n.row)?, n.left.clone(), Some(put(&n.right, row)?))?
        }
        std::cmp::Ordering::Equal => node(row, n.left.clone(), n.right.clone()),
    })
}
fn remove<R: RowCodec>(root: &Link<R>, key: u128) -> Result<Link<R>, &'static str> {
    let Some(n) = root else {
        return Ok(None);
    };
    Ok(match key.cmp(&n.row.key()) {
        std::cmp::Ordering::Less => Some(balance(
            copy(&n.row)?,
            remove(&n.left, key)?,
            n.right.clone(),
        )?),
        std::cmp::Ordering::Greater => Some(balance(
            copy(&n.row)?,
            n.left.clone(),
            remove(&n.right, key)?,
        )?),
        std::cmp::Ordering::Equal => match (&n.left, &n.right) {
            (None, _) => n.right.clone(),
            (_, None) => n.left.clone(),
            (_, Some(right)) => {
                let mut successor = right.as_ref();
                while let Some(left) = &successor.left {
                    successor = left;
                }
                Some(balance(
                    copy(&successor.row)?,
                    n.left.clone(),
                    remove(&n.right, successor.row.key())?,
                )?)
            }
        },
    })
}
fn visit<R>(root: &Link<R>, range: &Range<u128>, f: &mut impl FnMut(&R))
where
    R: RowCodec,
{
    if let Some(n) = root {
        let key = n.row.key();
        if range.start < key {
            visit(&n.left, range, f);
        }
        if range.contains(&key) {
            f(&n.row);
        }
        if key < range.end {
            visit(&n.right, range, f);
        }
    }
}
#[derive(Debug)]
pub struct Snapshot<R> {
    root: Link<R>,
    pub revision: u64,
    pub parent: u64,
    pub ranges: Vec<Range<u128>>,
    pub changed: Vec<R>,
}
impl<R: RowCodec> Snapshot<R> {
    /// Find the first row at or above the range start without a row scan.
    pub fn has_range(&self, range: Range<u128>) -> bool {
        let mut next = self.root.as_deref();
        while let Some(node) = next {
            let key = node.row.key();
            if key < range.start {
                next = node.right.as_deref();
            } else if key >= range.end {
                next = node.left.as_deref();
            } else {
                return true;
            }
        }
        false
    }
    pub fn visit_range(&self, range: std::ops::Range<u128>, mut visit_row: impl FnMut(&R)) {
        visit(&self.root, &range, &mut visit_row);
    }
    pub fn len(&self) -> usize {
        len(&self.root)
    }
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }
    pub fn allocated_bytes(&self) -> usize {
        bytes(&self.root)
            + self.changed.capacity() * std::mem::size_of::<R>()
            + self.ranges.capacity() * std::mem::size_of::<Range<u128>>()
    }
    pub fn complete<S: TableSchema<Row = R>>(
        &self,
    ) -> Result<replay_data::rows::TableRows, &'static str> {
        let mut rows =
            allocation::vec(self.len()).map_err(|_| "certified snapshot allocation failed")?;
        let mut failure = None;
        visit(&self.root, &(0..u128::MAX), &mut |row| match copy(row) {
            Ok(row) => rows.push(row),
            Err(reason) => failure = Some(reason),
        });
        if let Some(reason) = failure {
            return Err(reason);
        }
        Ok(S::wrap(rows))
    }
}
pub struct Store<R> {
    root: Link<R>,
    held: Option<Arc<Snapshot<R>>>,
    edits: BTreeMap<u128, Option<R>>,
}
impl<R> Default for Store<R> {
    fn default() -> Self {
        Self {
            root: None,
            held: None,
            edits: BTreeMap::new(),
        }
    }
}
impl<R: RowCodec> Store<R> {
    /// A rare withdrawal edits only dependent ranges, sharing every other path.
    pub fn from_snapshot(snapshot: Arc<Snapshot<R>>) -> Self {
        Self {
            root: snapshot.root.clone(),
            held: Some(snapshot),
            edits: BTreeMap::new(),
        }
    }
    pub fn clear(&mut self) {
        self.root = None;
        self.held = None;
        self.edits.clear();
    }
    pub fn get(&self, key: u128) -> Option<&R> {
        let mut next = self.root.as_deref();
        while let Some(n) = next {
            match key.cmp(&n.row.key()) {
                std::cmp::Ordering::Equal => return Some(&n.row),
                std::cmp::Ordering::Less => next = n.left.as_deref(),
                std::cmp::Ordering::Greater => next = n.right.as_deref(),
            }
        }
        None
    }
    pub fn insert(&mut self, row: R) -> Result<(), &'static str> {
        if self.get(row.key()) == Some(&row) {
            return Ok(());
        }
        self.root = Some(put(&self.root, copy(&row)?)?);
        self.edits.insert(row.key(), Some(row));
        Ok(())
    }
    pub fn remove_range(&mut self, range: Range<u128>) -> Result<(), &'static str> {
        let mut keys = Vec::new();
        let mut failure = false;
        visit(&self.root, &range, &mut |row| {
            if allocation::reserve(&mut keys, 1).is_err() {
                failure = true;
            } else {
                keys.push(row.key());
            }
        });
        if failure {
            return Err("certified deletion allocation failed");
        }
        for key in keys {
            self.root = remove(&self.root, key)?;
            self.edits.insert(key, None);
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<Arc<Snapshot<R>>, &'static str> {
        if self.edits.is_empty()
            && let Some(held) = &self.held
        {
            return Ok(Arc::clone(held));
        }
        let mut ranges =
            allocation::vec(self.edits.len()).map_err(|_| "certified ranges allocation failed")?;
        let mut changed =
            allocation::vec(self.edits.len()).map_err(|_| "certified changes allocation failed")?;
        for (key, row) in std::mem::take(&mut self.edits) {
            ranges.push(key..key.checked_add(1).ok_or("certified key overflow")?);
            if let Some(row) = row {
                changed.push(row);
            }
        }
        let snapshot = Arc::new(Snapshot {
            root: self.root.clone(),
            revision: REVISION.fetch_add(1, Ordering::Relaxed),
            parent: self.held.as_ref().map_or(0, |held| held.revision),
            ranges,
            changed,
        });
        self.held = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }
    pub fn allocated_bytes(&self) -> usize {
        bytes(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{DivisionHistoryRow, SharedDivisionHistoryRow, tables};
    fn row(division: u64, ordinal: u32) -> SharedDivisionHistoryRow {
        Arc::new(DivisionHistoryRow {
            division,
            list: DivisionHistoryRow::CAREER,
            ordinal,
            available: 1,
            name: format!("entry {division}/{ordinal}").into_bytes(),
            ..Default::default()
        })
    }
    #[test]
    fn withdrawal_forks_only_selected_ranges_and_keeps_the_original_snapshot() {
        let mut store = Store::default();
        let first = row(1, 0);
        let independent = row(2, 0);
        store.insert(first.clone()).unwrap();
        store.insert(independent.clone()).unwrap();
        let original = store.finish().unwrap();
        collector_test_support::with_allocations_denied(|| {
            assert!(!original.has_range(0..0));
            assert!(original.has_range(first.key()..first.key() + 1));
            assert!(!original.has_range(first.key() + 1..independent.key()));
            assert!(original.has_range(independent.key()..independent.key() + 1));
            assert!(!original.has_range(independent.key() + 1..u128::MAX));
        });
        let mut seen = Vec::new();
        original.visit_range(first.key()..first.key() + 1, |row| seen.push(row.clone()));
        assert_eq!(seen.as_slice(), std::slice::from_ref(&first));
        let mut withdrawn = Store::from_snapshot(Arc::clone(&original));
        withdrawn
            .remove_range(first.key()..first.key() + 1)
            .unwrap();
        let changed = withdrawn.finish().unwrap();
        assert!(!changed.has_range(first.key()..first.key() + 1));
        assert_eq!(changed.parent, original.revision);
        assert_eq!(
            changed.ranges.as_slice(),
            std::slice::from_ref(&(first.key()..first.key() + 1))
        );
        assert_eq!(original.len(), 2);
        let replay_data::rows::TableRows::DivisionHistory(rows) =
            changed.complete::<tables::DivisionHistory>().unwrap()
        else {
            panic!()
        };
        assert_eq!(rows.len(), 1);
        assert!(Arc::ptr_eq(&rows[0], &independent));
    }

    #[test]
    fn one_late_award_publishes_one_range_and_keeps_every_unchanged_entry_allocation() {
        let mut store = Store::default();
        for division in 1..=5000 {
            for ordinal in 0..3 {
                store.insert(row(division, ordinal)).unwrap();
            }
        }
        let first = store.finish().unwrap();
        let first_rows = first.complete::<tables::DivisionHistory>().unwrap();
        let held = store.finish().unwrap();
        assert!(Arc::ptr_eq(&first, &held));
        store.insert(row(2500, 3)).unwrap();
        let changed = store.finish().unwrap();
        assert_eq!(changed.parent, first.revision);
        assert_eq!(changed.ranges.len(), 1);
        assert_eq!(changed.changed.len(), 1);
        assert_eq!(changed.len(), 15_001);
        let all = changed.complete::<tables::DivisionHistory>().unwrap();
        let replay_data::rows::TableRows::DivisionHistory(previous) = first_rows else {
            panic!()
        };
        let replay_data::rows::TableRows::DivisionHistory(current) = all else {
            panic!()
        };
        for old in &previous {
            let at = current
                .binary_search_by_key(&old.key(), RowCodec::key)
                .unwrap();
            assert!(Arc::ptr_eq(old, &current[at]));
        }
        store
            .remove_range(row(2500, 0).key()..row(2501, 0).key())
            .unwrap();
        let removed = store.finish().unwrap();
        assert_eq!(removed.ranges.len(), 4);
        assert!(removed.changed.is_empty());
        assert_eq!(removed.len(), 14_997);
    }
    #[test]
    fn persistent_insert_delete_and_replace_match_an_independent_ordered_map() {
        let mut store = Store::default();
        let mut expected = BTreeMap::new();
        for number in (0..1000u32).map(|i| (i * 619) % 1000) {
            let value = row(1, number);
            expected.insert(value.key(), value.clone());
            store.insert(value).unwrap();
        }
        let before = store.finish().unwrap();
        for number in (0..1000u32).step_by(3) {
            let key = row(1, number).key();
            store.remove_range(key..key + 1).unwrap();
            expected.remove(&key);
        }
        let after = store.finish().unwrap();
        let all = after.complete::<tables::DivisionHistory>().unwrap();
        let replay_data::rows::TableRows::DivisionHistory(actual) = all else {
            panic!()
        };
        assert_eq!(actual, expected.into_values().collect::<Vec<_>>());
        assert_eq!(before.len(), 1000);
    }
}
