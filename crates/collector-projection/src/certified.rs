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
fn refresh<R: RowCodec>(n: &mut Node<R>) {
    n.height = 1 + height(&n.left).max(height(&n.right));
    n.len = 1 + len(&n.left) + len(&n.right);
    n.bytes =
        std::mem::size_of::<Node<R>>() + n.row.heap_bytes() + bytes(&n.left) + bytes(&n.right);
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
fn rotate_right_mut<R: RowCodec>(link: &mut Link<R>) -> Result<(), &'static str> {
    let left_unique = Arc::strong_count(link.as_ref().unwrap().left.as_ref().unwrap()) == 1;
    if left_unique {
        let mut root = link.take().unwrap();
        let mut left = Arc::get_mut(&mut root).unwrap().left.take().unwrap();
        let lr = Arc::get_mut(&mut left).unwrap().right.take();
        {
            let r = Arc::get_mut(&mut root).unwrap();
            r.left = lr;
            refresh(r);
        }
        {
            let l = Arc::get_mut(&mut left).unwrap();
            l.right = Some(root);
            refresh(l);
        }
        *link = Some(left);
        return Ok(());
    }
    let (copied, ll, lr) = {
        let l = link.as_ref().unwrap().left.as_ref().unwrap();
        (copy(&l.row)?, l.left.clone(), l.right.clone())
    };
    let mut root = link.take().unwrap();
    {
        let r = Arc::get_mut(&mut root).unwrap();
        r.left = lr;
        refresh(r);
    }
    *link = Some(node(copied, ll, Some(root)));
    Ok(())
}
fn rotate_left_mut<R: RowCodec>(link: &mut Link<R>) -> Result<(), &'static str> {
    let right_unique = Arc::strong_count(link.as_ref().unwrap().right.as_ref().unwrap()) == 1;
    if right_unique {
        let mut root = link.take().unwrap();
        let mut right = Arc::get_mut(&mut root).unwrap().right.take().unwrap();
        let rl = Arc::get_mut(&mut right).unwrap().left.take();
        {
            let r = Arc::get_mut(&mut root).unwrap();
            r.right = rl;
            refresh(r);
        }
        {
            let right_node = Arc::get_mut(&mut right).unwrap();
            right_node.left = Some(root);
            refresh(right_node);
        }
        *link = Some(right);
        return Ok(());
    }
    let (copied, rl, rr) = {
        let r = link.as_ref().unwrap().right.as_ref().unwrap();
        (copy(&r.row)?, r.left.clone(), r.right.clone())
    };
    let mut root = link.take().unwrap();
    {
        let n = Arc::get_mut(&mut root).unwrap();
        n.right = rl;
        refresh(n);
    }
    *link = Some(node(copied, Some(root), rr));
    Ok(())
}
fn balance_mut<R: RowCodec>(link: &mut Link<R>) -> Result<(), &'static str> {
    let (lh, rh) = {
        let n = link.as_ref().unwrap();
        (height(&n.left), height(&n.right))
    };
    if lh > rh + 1 {
        let need_lr = {
            let l = link.as_ref().unwrap().left.as_ref().unwrap();
            height(&l.right) > height(&l.left)
        };
        if need_lr {
            let l_unique = Arc::strong_count(link.as_ref().unwrap().left.as_ref().unwrap()) == 1;
            if l_unique {
                rotate_left_mut(&mut Arc::get_mut(link.as_mut().unwrap()).unwrap().left)?;
            } else {
                let rebuilt = {
                    let l = link.as_ref().unwrap().left.as_ref().unwrap();
                    let r = l.right.as_ref().unwrap();
                    node(
                        copy(&r.row)?,
                        Some(node(copy(&l.row)?, l.left.clone(), r.left.clone())),
                        r.right.clone(),
                    )
                };
                Arc::get_mut(link.as_mut().unwrap()).unwrap().left = Some(rebuilt);
            }
        }
        return rotate_right_mut(link);
    }
    if rh > lh + 1 {
        let need_rl = {
            let r = link.as_ref().unwrap().right.as_ref().unwrap();
            height(&r.left) > height(&r.right)
        };
        if need_rl {
            let r_unique = Arc::strong_count(link.as_ref().unwrap().right.as_ref().unwrap()) == 1;
            if r_unique {
                rotate_right_mut(&mut Arc::get_mut(link.as_mut().unwrap()).unwrap().right)?;
            } else {
                let rebuilt = {
                    let r = link.as_ref().unwrap().right.as_ref().unwrap();
                    let l = r.left.as_ref().unwrap();
                    node(
                        copy(&l.row)?,
                        l.left.clone(),
                        Some(node(copy(&r.row)?, l.right.clone(), r.right.clone())),
                    )
                };
                Arc::get_mut(link.as_mut().unwrap()).unwrap().right = Some(rebuilt);
            }
        }
        return rotate_left_mut(link);
    }
    refresh(Arc::get_mut(link.as_mut().unwrap()).unwrap());
    Ok(())
}
fn put_shared<R: RowCodec>(
    root: &Link<R>,
    row: &R,
) -> Result<Option<Arc<Node<R>>>, &'static str> {
    let Some(n) = root else {
        return Ok(Some(node(copy(row)?, None, None)));
    };
    match row.key().cmp(&n.row.key()) {
        std::cmp::Ordering::Less => {
            let Some(left) = put_shared(&n.left, row)? else {
                return Ok(None);
            };
            Ok(Some(balance(copy(&n.row)?, Some(left), n.right.clone())?))
        }
        std::cmp::Ordering::Greater => {
            let Some(right) = put_shared(&n.right, row)? else {
                return Ok(None);
            };
            Ok(Some(balance(copy(&n.row)?, n.left.clone(), Some(right))?))
        }
        std::cmp::Ordering::Equal => {
            if &n.row == row {
                return Ok(None);
            }
            Ok(Some(node(copy(row)?, n.left.clone(), n.right.clone())))
        }
    }
}
fn put_mut<R: RowCodec>(link: &mut Link<R>, row: &R) -> Result<bool, &'static str> {
    if link.is_none() {
        *link = Some(node(copy(row)?, None, None));
        return Ok(true);
    }
    if let Some(n) = link.as_mut().and_then(Arc::get_mut) {
        let changed = match row.key().cmp(&n.row.key()) {
            std::cmp::Ordering::Less => put_mut(&mut n.left, row)?,
            std::cmp::Ordering::Greater => put_mut(&mut n.right, row)?,
            std::cmp::Ordering::Equal => {
                if &n.row == row {
                    return Ok(false);
                }
                n.row = copy(row)?;
                refresh(n);
                return Ok(true);
            }
        };
        if changed {
            balance_mut(link)?;
        }
        return Ok(changed);
    }
    let Some(updated) = put_shared(link, row)? else {
        return Ok(false);
    };
    *link = Some(updated);
    Ok(true)
}
fn remove_shared<R: RowCodec>(root: &Link<R>, key: u128) -> Result<Link<R>, &'static str> {
    let Some(n) = root else {
        return Ok(None);
    };
    Ok(match key.cmp(&n.row.key()) {
        std::cmp::Ordering::Less => Some(balance(
            copy(&n.row)?,
            remove_shared(&n.left, key)?,
            n.right.clone(),
        )?),
        std::cmp::Ordering::Greater => Some(balance(
            copy(&n.row)?,
            n.left.clone(),
            remove_shared(&n.right, key)?,
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
                    remove_shared(&n.right, successor.row.key())?,
                )?)
            }
        },
    })
}
fn take_min_mut<R: RowCodec>(link: &mut Link<R>) -> Result<R, &'static str> {
    if link.as_ref().unwrap().left.is_none() {
        if Arc::strong_count(link.as_ref().unwrap()) == 1 {
            let node_arc = link.take().unwrap();
            let node = Arc::into_inner(node_arc).unwrap();
            *link = node.right;
            return Ok(node.row);
        }
        let copied = copy(&link.as_ref().unwrap().row)?;
        let right = link.as_ref().unwrap().right.clone();
        *link = right;
        return Ok(copied);
    }
    if let Some(n) = link.as_mut().and_then(Arc::get_mut) {
        let min = take_min_mut(&mut n.left)?;
        balance_mut(link)?;
        return Ok(min);
    }
    let mut successor = link.as_deref().unwrap();
    while let Some(left) = &successor.left {
        successor = left;
    }
    let copied = copy(&successor.row)?;
    let key = copied.key();
    *link = remove_shared(link, key)?;
    Ok(copied)
}
fn remove_mut<R: RowCodec>(link: &mut Link<R>, key: u128) -> Result<(), &'static str> {
    if link.is_none() {
        return Ok(());
    }
    if Arc::strong_count(link.as_ref().unwrap()) > 1 {
        *link = remove_shared(link, key)?;
        return Ok(());
    }
    let cmp = key.cmp(&link.as_ref().unwrap().row.key());
    match cmp {
        std::cmp::Ordering::Less => {
            remove_mut(&mut Arc::get_mut(link.as_mut().unwrap()).unwrap().left, key)?;
            balance_mut(link)?;
        }
        std::cmp::Ordering::Greater => {
            remove_mut(
                &mut Arc::get_mut(link.as_mut().unwrap()).unwrap().right,
                key,
            )?;
            balance_mut(link)?;
        }
        std::cmp::Ordering::Equal => {
            let has_left = link.as_ref().unwrap().left.is_some();
            let has_right = link.as_ref().unwrap().right.is_some();
            match (has_left, has_right) {
                (false, _) => {
                    let right = Arc::get_mut(link.as_mut().unwrap()).unwrap().right.take();
                    *link = right;
                }
                (_, false) => {
                    let left = Arc::get_mut(link.as_mut().unwrap()).unwrap().left.take();
                    *link = left;
                }
                (true, true) => {
                    let successor =
                        take_min_mut(&mut Arc::get_mut(link.as_mut().unwrap()).unwrap().right)?;
                    Arc::get_mut(link.as_mut().unwrap()).unwrap().row = successor;
                    balance_mut(link)?;
                }
            }
        }
    }
    Ok(())
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
        if !put_mut(&mut self.root, &row)? {
            return Ok(());
        }
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
            remove_mut(&mut self.root, key)?;
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
    fn repeated_replacements_reuse_private_paths_without_changing_published_snapshots() {
        let mut store = Store::default();
        for ordinal in 0..100 {
            store.insert(row(1, ordinal)).unwrap();
        }
        let original = store.finish().unwrap();
        let original_rows = original.complete::<tables::DivisionHistory>().unwrap();
        let changed = |ordinal, name: &[u8]| {
            let mut value = (*row(1, ordinal)).clone();
            value.name = name.to_vec();
            Arc::new(value)
        };
        let first = changed(20, b"first replacement");
        store.insert(first).unwrap();
        let root = Arc::as_ptr(store.root.as_ref().unwrap());
        let last = changed(20, b"a longer replacement with a different byte count");
        collector_test_support::with_allocations_denied(|| {
            store.insert(Arc::clone(&last)).unwrap();
        });
        assert_eq!(root, Arc::as_ptr(store.root.as_ref().unwrap()));
        assert_eq!(
            original.complete::<tables::DivisionHistory>().unwrap(),
            original_rows
        );
        let updated = store.finish().unwrap();
        assert_eq!(updated.parent, original.revision);
        assert_eq!(updated.len(), original.len());
        assert_eq!(updated.changed.as_slice(), std::slice::from_ref(&last));
        assert_eq!(
            updated.ranges.as_slice(),
            std::slice::from_ref(&(last.key()..last.key() + 1))
        );
        let replay_data::rows::TableRows::DivisionHistory(rows) =
            updated.complete::<tables::DivisionHistory>().unwrap()
        else {
            panic!()
        };
        assert_eq!(rows[20], last);
        let replay_data::rows::TableRows::DivisionHistory(previous) = original_rows else {
            panic!()
        };
        for ordinal in (0..100).filter(|&ordinal| ordinal != 20) {
            assert!(Arc::ptr_eq(&rows[ordinal], &previous[ordinal]));
        }
        let mut fresh = Store::default();
        for row in rows {
            fresh.insert(row).unwrap();
        }
        assert_eq!(bytes(&store.root), bytes(&fresh.root));
        store.insert(changed(20, b"after publication")).unwrap();
        assert_eq!(updated.changed[0], last);
        assert!(Arc::ptr_eq(
            &store.finish().unwrap(),
            &store.finish().unwrap()
        ));
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
        for number in (0..1000u32).step_by(11) {
            let key = row(1, number).key();
            store.remove_range(key..key + 1).unwrap();
            expected.remove(&key);
        }
        let before = store.finish().unwrap();
        let before_expected: Vec<_> = expected.values().cloned().collect();
        for number in (0..1000u32).step_by(3) {
            let key = row(1, number).key();
            store.remove_range(key..key + 1).unwrap();
            expected.remove(&key);
        }
        for pass in 0..2 {
            for number in (0..1000u32).step_by(7).filter(|number| number % 3 != 0 && number % 11 != 0) {
                let mut value = (*row(1, number)).clone();
                value.name = format!("replacement {number}/{pass}").into_bytes();
                let value = Arc::new(value);
                expected.insert(value.key(), Arc::clone(&value));
                store.insert(value).unwrap();
            }
        }
        let after = store.finish().unwrap();
        let replay_data::rows::TableRows::DivisionHistory(actual_before) =
            before.complete::<tables::DivisionHistory>().unwrap()
        else {
            panic!()
        };
        assert_eq!(actual_before, before_expected);
        let all = after.complete::<tables::DivisionHistory>().unwrap();
        let replay_data::rows::TableRows::DivisionHistory(actual) = all else {
            panic!()
        };
        assert_eq!(actual, expected.into_values().collect::<Vec<_>>());
    }
}
