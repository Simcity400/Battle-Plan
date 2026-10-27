use map_display_data::credit::{Credit, MemoryBudget};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, OnceLock},
};

pub trait Key: Copy + Ord {
    fn bits(self) -> u64;
}
impl Key for u32 {
    fn bits(self) -> u64 {
        self as u64
    }
}
impl Key for u64 {
    fn bits(self) -> u64 {
        self
    }
}
const RADIX: usize = 4;
// At most three pending siblings for each two-bit key position.
const MAX_STACK: usize = u64::BITS as usize / 2 * (RADIX - 1) + 1;

struct Node<K, V> {
    key: K,
    mask: u64,
    value: Option<V>,
    children: [Option<Arc<Node<K, V>>>; RADIX],
    _credit: Credit,
}
fn split(a: u64, b: u64) -> u64 {
    let xor = a ^ b;
    if xor == 0 {
        0
    } else {
        3 << ((63 - xor.leading_zeros()) & !1)
    }
}
fn slot(key: u64, mask: u64) -> usize {
    ((key & mask) >> mask.trailing_zeros()) as usize
}
pub struct Values<K, V> {
    root: Option<Arc<Node<K, V>>>,
    flat: OnceLock<BTreeMap<K, V>>,
}
impl<K, V> Clone for Values<K, V> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            flat: OnceLock::new(),
        }
    }
}
impl<K, V> Default for Values<K, V> {
    fn default() -> Self {
        Self {
            root: None,
            flat: OnceLock::new(),
        }
    }
}
impl<K: Key, V: Clone + Eq> Values<K, V> {
    pub fn same_root(&self, other: &Self) -> bool {
        match (&self.root, &other.root) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        let mut n = self.root.as_deref()?;
        loop {
            if n.mask == 0 {
                return (n.key == *key).then_some(n.value.as_ref().unwrap());
            }
            n = n.children[slot(key.bits(), n.mask)].as_deref()?;
        }
    }
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }
    pub fn insert(&mut self, key: K, value: V, budget: &MemoryBudget) -> io::Result<()> {
        if self.get(&key) == Some(&value) {
            return Ok(());
        }
        self.root = Some(edit(self.root.as_ref(), key, Some(value), budget)?.unwrap());
        self.flat.take();
        Ok(())
    }
    pub fn remove(&mut self, key: &K, budget: &MemoryBudget) -> io::Result<()> {
        if !self.contains_key(key) {
            return Ok(());
        }
        self.root = edit(self.root.as_ref(), *key, None, budget)?;
        self.flat.take();
        Ok(())
    }
    pub fn from_iter(
        values: impl IntoIterator<Item = (K, V)>,
        budget: &MemoryBudget,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        for (k, v) in values {
            out.insert(k, v, budget)?;
        }
        Ok(out)
    }
    pub fn iter(&self) -> Iter<'_, K, V> {
        let mut out = Iter {
            stack: [None; MAX_STACK],
            len: 0,
        };
        if let Some(n) = self.root.as_deref() {
            out.push(n);
        }
        out
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
    // Legacy diagnostics only; display consumers use shared roots and changed keys.
    pub fn flat(&self) -> &BTreeMap<K, V> {
        self.flat
            .get_or_init(|| self.iter().map(|(k, v)| (*k, v.clone())).collect())
    }
    pub fn changed(&self, previous: &Self, mut visit: impl FnMut(K)) {
        difference(previous.root.as_ref(), self.root.as_ref(), &mut visit);
    }
}
fn node<K: Key, V: Clone>(
    key: K,
    mask: u64,
    value: Option<V>,
    children: [Option<Arc<Node<K, V>>>; RADIX],
    budget: &MemoryBudget,
) -> io::Result<Arc<Node<K, V>>> {
    let credit = budget.admit(size_of::<Node<K, V>>() + 2 * size_of::<usize>())?;
    Ok(Arc::new(Node {
        key,
        mask,
        value,
        children,
        _credit: credit,
    }))
}
fn edit<K: Key, V: Clone>(
    old: Option<&Arc<Node<K, V>>>,
    key: K,
    value: Option<V>,
    budget: &MemoryBudget,
) -> io::Result<Option<Arc<Node<K, V>>>> {
    let Some(old) = old else {
        return value
            .map(|v| node(key, 0, Some(v), std::array::from_fn(|_| None), budget))
            .transpose();
    };
    let mask = split(old.key.bits(), key.bits());
    if mask > old.mask {
        let Some(value) = value else {
            return Ok(Some(Arc::clone(old)));
        };
        let leaf = node(key, 0, Some(value), std::array::from_fn(|_| None), budget)?;
        let mut children = std::array::from_fn(|_| None);
        children[slot(old.key.bits(), mask)] = Some(Arc::clone(old));
        children[slot(key.bits(), mask)] = Some(leaf);
        return node(old.key, mask, None, children, budget).map(Some);
    }
    if old.mask == 0 {
        return value
            .map(|v| node(key, 0, Some(v), std::array::from_fn(|_| None), budget))
            .transpose();
    }
    let index = slot(key.bits(), old.mask);
    let changed = edit(old.children[index].as_ref(), key, value, budget)?;
    let mut children = old.children.clone();
    children[index] = changed;
    let mut occupied = children.iter().flatten();
    let Some(first) = occupied.next() else {
        return Ok(None);
    };
    if occupied.next().is_none() {
        return Ok(Some(Arc::clone(first)));
    }
    node(first.key, old.mask, None, children, budget).map(Some)
}
fn walk<K: Key, V>(n: Option<&Arc<Node<K, V>>>, visit: &mut impl FnMut(K)) {
    if let Some(n) = n {
        if n.mask == 0 {
            visit(n.key)
        } else {
            for child in &n.children {
                walk(child.as_ref(), visit);
            }
        }
    }
}
fn difference<K: Key, V: Eq>(
    a: Option<&Arc<Node<K, V>>>,
    b: Option<&Arc<Node<K, V>>>,
    visit: &mut impl FnMut(K),
) {
    let (Some(a), Some(b)) = (a, b) else {
        walk(a, visit);
        walk(b, visit);
        return;
    };
    if Arc::ptr_eq(a, b) {
        return;
    }
    let mask = split(a.key.bits(), b.key.bits());
    if mask > a.mask.max(b.mask) {
        walk(Some(a), visit);
        walk(Some(b), visit)
    } else if a.mask > b.mask {
        let common = slot(b.key.bits(), a.mask);
        for (index, child) in a.children.iter().enumerate() {
            if index == common {
                difference(child.as_ref(), Some(b), visit);
            } else {
                walk(child.as_ref(), visit);
            }
        }
    } else if b.mask > a.mask {
        let common = slot(a.key.bits(), b.mask);
        for (index, child) in b.children.iter().enumerate() {
            if index == common {
                difference(Some(a), child.as_ref(), visit);
            } else {
                walk(child.as_ref(), visit);
            }
        }
    } else if a.mask == 0 {
        if a.value != b.value {
            visit(a.key)
        }
    } else {
        for (left, right) in a.children.iter().zip(&b.children) {
            difference(left.as_ref(), right.as_ref(), visit);
        }
    }
}
pub struct Iter<'a, K, V> {
    stack: [Option<&'a Node<K, V>>; MAX_STACK],
    len: usize,
}
impl<'a, K, V> Iter<'a, K, V> {
    fn push(&mut self, n: &'a Node<K, V>) {
        self.stack[self.len] = Some(n);
        self.len += 1;
    }
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        while self.len > 0 {
            self.len -= 1;
            let n = self.stack[self.len].take().unwrap();
            if n.mask == 0 {
                return Some((&n.key, n.value.as_ref().unwrap()));
            }
            for child in n.children.iter().rev().flatten() {
                self.push(child);
            }
        }
        None
    }
}
impl<K: Key + std::fmt::Debug, V: Clone + Eq + std::fmt::Debug> std::fmt::Debug for Values<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}
impl<K: Key, V: Clone + Eq> PartialEq for Values<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.same_root(other) || self.iter().eq(other.iter())
    }
}
impl<K: Key, V: Clone + Eq> Eq for Values<K, V> {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_paths_patch_delete_and_diff_without_copying_quiet_values() {
        let budget = MemoryBudget::new(16 << 20);
        let mut values =
            Values::from_iter((0u32..5000).map(|key| (key, key as u8)), &budget).unwrap();
        let before = values.clone();
        let resident = budget.used();
        values.insert(17, 233, &budget).unwrap();
        values.remove(&1023, &budget).unwrap();
        values.insert(9000, 7, &budget).unwrap();
        assert_eq!(before.get(&17), Some(&17));
        assert_eq!(values.get(&17), Some(&233));
        assert_eq!(values.get(&1023), None);
        let mut changed = Vec::new();
        values.changed(&before, |key| changed.push(key));
        changed.sort_unstable();
        assert_eq!(changed, [17, 1023, 9000]);
        assert!(budget.used() - resident < 64 * size_of::<Node<u32, u8>>());
        let mut oracle = before.flat().clone();
        oracle.insert(17, 233);
        oracle.remove(&1023);
        oracle.insert(9000, 7);
        assert_eq!(values.flat(), &oracle);
        drop(before);
        drop(values);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn full_width_radix_branches_preserve_order_changes_and_admission() {
        for count in [3u64, 1500, 5000] {
            let budget = MemoryBudget::new(16 << 20);
            let mut expected: BTreeMap<_, _> = (0..count)
                .map(|key| (key.wrapping_mul(0x9e3779b97f4a7c15), key as u8))
                .chain([(u64::MAX, 4), (1 << 63, 5)])
                .collect();
            let before = Values::from_iter(expected.clone(), &budget).unwrap();
            let mut after = before.clone();
            let old = expected.clone();
            for key in [2, (1 << 32) - 1, 1 << 32, 1 << 63, u64::MAX] {
                after.insert(key, 91, &budget).unwrap();
                expected.insert(key, 91);
            }
            after.remove(&0, &budget).unwrap();
            expected.remove(&0);
            let mut changed = Vec::new();
            after.changed(&before, |key| changed.push(key));
            changed.sort_unstable();
            let expected_changes: Vec<_> = old
                .keys()
                .chain(expected.keys())
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter(|key| old.get(key) != expected.get(key))
                .collect();
            assert_eq!(changed, expected_changes);
            assert_eq!(before.flat(), &old);
            assert_eq!(after.flat(), &expected);
            for (&key, value) in &expected {
                assert_eq!(after.get(&key), Some(value));
            }
            drop(before);
            drop(after);
            assert_eq!(budget.used(), 0);
        }
    }
    #[test]
    fn denied_path_growth_keeps_the_old_root_and_releases_temporary_nodes() {
        let budget = MemoryBudget::new(4096);
        let mut values = Values::from_iter([(1u64, 1u8), (3, 3)], &budget).unwrap();
        let held = budget.admit(budget.limit() - budget.used()).unwrap();
        let error = values.insert(2, 2, &budget).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(values.get(&1), Some(&1));
        assert_eq!(values.get(&2), None);
        drop(held);
        values.insert(2, 2, &budget).unwrap();
        drop(values);
        assert_eq!(budget.used(), 0);
    }
}
