use crate::credit::{Credit, display_products};
use std::{cmp::Ordering, io, sync::Arc};

// Immutable AVL paths share untouched nodes; each node owns its pre-allocation credit.
pub struct CopyIndex<K, V>(Option<Arc<Node<K, V>>>);
struct Node<K, V> {
    key: K,
    value: V,
    left: CopyIndex<K, V>,
    right: CopyIndex<K, V>,
    height: u8,
    len: usize,
    _credit: Credit,
}
impl<K, V> Clone for CopyIndex<K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<K, V> Default for CopyIndex<K, V> {
    fn default() -> Self {
        Self(None)
    }
}
impl<K: Eq, V: Eq> Eq for CopyIndex<K, V> {}
impl<K: Ord + Copy, V: Copy + PartialEq> From<std::collections::BTreeMap<K, V>>
    for CopyIndex<K, V>
{
    fn from(values: std::collections::BTreeMap<K, V>) -> Self {
        values
            .into_iter()
            .try_fold(Self::default(), |root, (key, value)| {
                root.inserted(key, value)
            })
            .expect("persistent fixture admission")
    }
}
impl<K, V> CopyIndex<K, V> {
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |node| node.len)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }
    fn height(&self) -> u8 {
        self.0.as_ref().map_or(0, |node| node.height)
    }
    pub fn iter(&self) -> Iter<'_, K, V> {
        let mut iter = Iter {
            stack: [None; 2 * usize::BITS as usize],
            len: 0,
        };
        iter.left(self);
        iter
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}
impl<K: Ord + Copy, V: Copy + PartialEq> CopyIndex<K, V> {
    pub fn iter_from(&self, key: &K) -> Iter<'_, K, V> {
        let mut iter = Iter {
            stack: [None; 2 * usize::BITS as usize],
            len: 0,
        };
        let mut next = self.0.as_deref();
        while let Some(node) = next {
            if node.key < *key {
                next = node.right.0.as_deref();
            } else {
                iter.stack[iter.len] = Some(node);
                iter.len += 1;
                next = node.left.0.as_deref();
            }
        }
        iter
    }
    pub fn last_at_or_before(&self, key: &K) -> Option<(&K, &V)> {
        let mut at = self.0.as_deref();
        let mut found = None;
        while let Some(node) = at {
            if node.key > *key {
                at = node.left.0.as_deref();
            } else {
                found = Some((&node.key, &node.value));
                at = node.right.0.as_deref();
            }
        }
        found
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        let mut at = self.0.as_deref();
        while let Some(node) = at {
            match key.cmp(&node.key) {
                Ordering::Less => at = node.left.0.as_deref(),
                Ordering::Greater => at = node.right.0.as_deref(),
                Ordering::Equal => return Some(&node.value),
            }
        }
        None
    }
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }
    fn node(key: K, value: V, left: Self, right: Self) -> io::Result<Self> {
        let credit = display_products()
            .admit(size_of::<Node<K, V>>() + 2 * size_of::<usize>() + align_of::<Node<K, V>>())?;
        let height = 1 + left.height().max(right.height());
        let len = 1 + left.len() + right.len();
        Ok(Self(Some(Arc::new(Node {
            key,
            value,
            left,
            right,
            height,
            len,
            _credit: credit,
        }))))
    }
    fn balanced(key: K, value: V, mut left: Self, mut right: Self) -> io::Result<Self> {
        if i16::from(left.height()) - i16::from(right.height()) > 1 {
            let branch = left.0.as_ref().expect("left-heavy index");
            if branch.left.height() < branch.right.height() {
                let middle = branch.right.0.as_ref().expect("inner left rotation");
                left = Self::node(
                    middle.key,
                    middle.value,
                    Self::node(
                        branch.key,
                        branch.value,
                        branch.left.clone(),
                        middle.left.clone(),
                    )?,
                    middle.right.clone(),
                )?;
            }
            let branch = left.0.as_ref().expect("left rotation root");
            return Self::node(
                branch.key,
                branch.value,
                branch.left.clone(),
                Self::node(key, value, branch.right.clone(), right)?,
            );
        }
        if i16::from(right.height()) - i16::from(left.height()) > 1 {
            let branch = right.0.as_ref().expect("right-heavy index");
            if branch.right.height() < branch.left.height() {
                let middle = branch.left.0.as_ref().expect("inner right rotation");
                right = Self::node(
                    middle.key,
                    middle.value,
                    middle.left.clone(),
                    Self::node(
                        branch.key,
                        branch.value,
                        middle.right.clone(),
                        branch.right.clone(),
                    )?,
                )?;
            }
            let branch = right.0.as_ref().expect("right rotation root");
            return Self::node(
                branch.key,
                branch.value,
                Self::node(key, value, left, branch.left.clone())?,
                branch.right.clone(),
            );
        }
        Self::node(key, value, left, right)
    }
    pub fn inserted(&self, key: K, value: V) -> io::Result<Self> {
        let Some(node) = &self.0 else {
            return Self::node(key, value, Self::default(), Self::default());
        };
        match key.cmp(&node.key) {
            Ordering::Equal if value == node.value => Ok(self.clone()),
            Ordering::Equal => Self::node(key, value, node.left.clone(), node.right.clone()),
            Ordering::Less => {
                let left = node.left.inserted(key, value)?;
                if left.ptr_eq(&node.left) {
                    return Ok(self.clone());
                }
                Self::balanced(node.key, node.value, left, node.right.clone())
            }
            Ordering::Greater => {
                let right = node.right.inserted(key, value)?;
                if right.ptr_eq(&node.right) {
                    return Ok(self.clone());
                }
                Self::balanced(node.key, node.value, node.left.clone(), right)
            }
        }
    }
    pub fn removed(&self, key: &K) -> io::Result<Self> {
        let Some(node) = &self.0 else {
            return Ok(self.clone());
        };
        match key.cmp(&node.key) {
            Ordering::Less => {
                let left = node.left.removed(key)?;
                if left.ptr_eq(&node.left) {
                    return Ok(self.clone());
                }
                Self::balanced(node.key, node.value, left, node.right.clone())
            }
            Ordering::Greater => {
                let right = node.right.removed(key)?;
                if right.ptr_eq(&node.right) {
                    return Ok(self.clone());
                }
                Self::balanced(node.key, node.value, node.left.clone(), right)
            }
            Ordering::Equal if node.left.is_empty() => Ok(node.right.clone()),
            Ordering::Equal if node.right.is_empty() => Ok(node.left.clone()),
            Ordering::Equal => {
                let (next, value) = node.right.iter().next().expect("nonempty successor");
                Self::balanced(*next, *value, node.left.clone(), node.right.removed(next)?)
            }
        }
    }
}
impl<K: PartialEq, V: PartialEq> PartialEq for CopyIndex<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for CopyIndex<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}
pub struct Iter<'a, K, V> {
    // An AVL tree addressable by usize cannot reach twice usize::BITS levels.
    stack: [Option<&'a Node<K, V>>; 2 * usize::BITS as usize],
    len: usize,
}
impl<'a, K, V> Iter<'a, K, V> {
    fn left(&mut self, tree: &'a CopyIndex<K, V>) {
        let mut next = tree.0.as_deref();
        while let Some(node) = next {
            self.stack[self.len] = Some(node);
            self.len += 1;
            next = node.left.0.as_deref();
        }
    }
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.len = self.len.checked_sub(1)?;
        let node = self.stack[self.len].take()?;
        self.left(&node.right);
        Some((&node.key, &node.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn check(tree: &CopyIndex<u32, u32>) -> (u8, usize) {
        let Some(node) = &tree.0 else { return (0, 0) };
        let (lh, ln) = check(&node.left);
        let (rh, rn) = check(&node.right);
        assert!((i16::from(lh) - i16::from(rh)).abs() <= 1);
        assert_eq!(node.height, 1 + lh.max(rh));
        assert_eq!(node.len, 1 + ln + rn);
        (node.height, node.len)
    }
    #[test]
    fn path_updates_match_independent_ordered_map_without_cloning_unmodified_roots() {
        let mut tree = CopyIndex::default();
        let mut oracle = BTreeMap::new();
        for id in (0..5003u32).map(|at| at * 997 % 5003) {
            tree = tree.inserted(id, id + 7).unwrap();
            oracle.insert(id, id + 7);
            check(&tree);
        }
        let old = tree.clone();
        assert!(tree.ptr_eq(&tree.inserted(1500, 1507).unwrap()));
        for id in (0..5003u32).step_by(3) {
            tree = tree.removed(&id).unwrap();
            oracle.remove(&id);
            check(&tree);
        }
        assert_eq!(
            tree.iter().collect::<Vec<_>>(),
            oracle.iter().collect::<Vec<_>>()
        );
        assert_eq!(old.len(), 5003);
        assert_eq!(old.get(&1500), Some(&1507));
    }
}
