use std::{cmp::Ordering, io, sync::Arc};

// Owned values share immutable AVL paths.
pub struct ValueIndex<K, V>(Option<Arc<Node<K, V>>>);
struct Node<K, V> {
    key: K,
    value: V,
    left: ValueIndex<K, V>,
    right: ValueIndex<K, V>,
    height: u8,
    len: usize,
}
impl<K, V> Clone for ValueIndex<K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<K, V> Default for ValueIndex<K, V> {
    fn default() -> Self {
        Self(None)
    }
}
impl<K, V> ValueIndex<K, V> {
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |node| node.len)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
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
impl<K: Ord + Clone, V: Clone + PartialEq> ValueIndex<K, V> {
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }
    pub fn try_from_iter(items: impl IntoIterator<Item = (K, V)>) -> io::Result<Self> {
        let mut index = Self::default();
        for (key, value) in items {
            index = index.inserted(key, value)?;
        }
        Ok(index)
    }
    // Sorted unique edits rebuild each affected ancestor once, including across AVL rotations.
    pub fn patched(&self, edits: &[(K, Option<V>)]) -> io::Result<Self> {
        if edits.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "index edits are not strictly ordered",
            ));
        }
        self.patch_ordered(edits)
    }
    fn joined(key: K, value: V, left: Self, right: Self) -> io::Result<Self> {
        if i16::from(left.height()) - i16::from(right.height()) > 1 {
            let node = left.0.as_ref().expect("left join");
            let tail = Self::joined(key, value, node.right.clone(), right)?;
            return Self::balanced(
                node.key.clone(),
                node.value.clone(),
                node.left.clone(),
                tail,
            );
        }
        if i16::from(right.height()) - i16::from(left.height()) > 1 {
            let node = right.0.as_ref().expect("right join");
            let head = Self::joined(key, value, left, node.left.clone())?;
            return Self::balanced(
                node.key.clone(),
                node.value.clone(),
                head,
                node.right.clone(),
            );
        }
        Self::node(key, value, left, right)
    }
    fn patch_ordered(&self, edits: &[(K, Option<V>)]) -> io::Result<Self> {
        if edits.is_empty() {
            return Ok(self.clone());
        }
        let Some(node) = &self.0 else {
            let middle = edits.len() / 2;
            let left = Self::default().patch_ordered(&edits[..middle])?;
            let right = Self::default().patch_ordered(&edits[middle + 1..])?;
            return match &edits[middle].1 {
                Some(value) => Self::joined(edits[middle].0.clone(), value.clone(), left, right),
                None => Self::concatenated(left, right),
            };
        };
        let middle = edits.partition_point(|(key, _)| key < &node.key);
        let replaces = middle < edits.len() && edits[middle].0 == node.key;
        let left = node.left.patch_ordered(&edits[..middle])?;
        let right = node
            .right
            .patch_ordered(&edits[middle + usize::from(replaces)..])?;
        let value = if replaces {
            edits[middle].1.as_ref()
        } else {
            Some(&node.value)
        };
        if let Some(value) = value {
            if left.ptr_eq(&node.left) && right.ptr_eq(&node.right) && value == &node.value {
                return Ok(self.clone());
            }
            Self::joined(node.key.clone(), value.clone(), left, right)
        } else {
            Self::concatenated(left, right)
        }
    }
    fn concatenated(left: Self, right: Self) -> io::Result<Self> {
        if left.is_empty() {
            return Ok(right);
        }
        if right.is_empty() {
            return Ok(left);
        }
        let (key, value) = right.iter().next().expect("join successor");
        Self::joined(key.clone(), value.clone(), left, right.removed(key)?)
    }
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
    fn node(key: K, value: V, left: Self, right: Self) -> io::Result<Self> {
        let height = 1 + left.height().max(right.height());
        let len = 1 + left.len() + right.len();
        Ok(Self(Some(Arc::new(Node {
            key,
            value,
            left,
            right,
            height,
            len,
        }))))
    }
    fn balanced(key: K, value: V, mut left: Self, mut right: Self) -> io::Result<Self> {
        if i16::from(left.height()) - i16::from(right.height()) > 1 {
            let branch = left.0.as_ref().expect("left-heavy index");
            if branch.left.height() < branch.right.height() {
                let middle = branch.right.0.as_ref().expect("inner left rotation");
                left = Self::node(
                    middle.key.clone(),
                    middle.value.clone(),
                    Self::node(
                        branch.key.clone(),
                        branch.value.clone(),
                        branch.left.clone(),
                        middle.left.clone(),
                    )?,
                    middle.right.clone(),
                )?;
            }
            let branch = left.0.as_ref().expect("left rotation root");
            return Self::node(
                branch.key.clone(),
                branch.value.clone(),
                branch.left.clone(),
                Self::node(key, value, branch.right.clone(), right)?,
            );
        }
        if i16::from(right.height()) - i16::from(left.height()) > 1 {
            let branch = right.0.as_ref().expect("right-heavy index");
            if branch.right.height() < branch.left.height() {
                let middle = branch.left.0.as_ref().expect("inner right rotation");
                right = Self::node(
                    middle.key.clone(),
                    middle.value.clone(),
                    middle.left.clone(),
                    Self::node(
                        branch.key.clone(),
                        branch.value.clone(),
                        middle.right.clone(),
                        branch.right.clone(),
                    )?,
                )?;
            }
            let branch = right.0.as_ref().expect("right rotation root");
            return Self::node(
                branch.key.clone(),
                branch.value.clone(),
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
                Self::balanced(
                    node.key.clone(),
                    node.value.clone(),
                    left,
                    node.right.clone(),
                )
            }
            Ordering::Greater => {
                let right = node.right.inserted(key, value)?;
                if right.ptr_eq(&node.right) {
                    return Ok(self.clone());
                }
                Self::balanced(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    right,
                )
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
                Self::balanced(
                    node.key.clone(),
                    node.value.clone(),
                    left,
                    node.right.clone(),
                )
            }
            Ordering::Greater => {
                let right = node.right.removed(key)?;
                if right.ptr_eq(&node.right) {
                    return Ok(self.clone());
                }
                Self::balanced(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    right,
                )
            }
            Ordering::Equal if node.left.is_empty() => Ok(node.right.clone()),
            Ordering::Equal if node.right.is_empty() => Ok(node.left.clone()),
            Ordering::Equal => {
                let (next, value) = node.right.iter().next().expect("nonempty successor");
                Self::balanced(
                    next.clone(),
                    value.clone(),
                    node.left.clone(),
                    node.right.removed(next)?,
                )
            }
        }
    }
}
impl<K: PartialEq, V: PartialEq> PartialEq for ValueIndex<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for ValueIndex<K, V> {
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
    fn left(&mut self, tree: &'a ValueIndex<K, V>) {
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

impl<K: Eq, V: Eq> Eq for ValueIndex<K, V> {}
impl<K: Ord + Clone, V: Clone + PartialEq> ValueIndex<K, V> {
    pub fn visit_changes(
        &self,
        previous: &Self,
        mut visit: impl FnMut(&K, Option<&V>, Option<&V>),
    ) {
        fn root<'a, K: Ord, V>(tree: &'a ValueIndex<K, V>, key: &K) -> Option<&'a Arc<Node<K, V>>> {
            let mut at = tree.0.as_ref();
            while let Some(node) = at {
                match key.cmp(&node.key) {
                    Ordering::Less => at = node.left.0.as_ref(),
                    Ordering::Greater => at = node.right.0.as_ref(),
                    Ordering::Equal => return Some(node),
                }
            }
            None
        }
        fn walk<K: Ord, V: PartialEq>(
            tree: &ValueIndex<K, V>,
            other: &ValueIndex<K, V>,
            removed: bool,
            visit: &mut impl FnMut(&K, Option<&V>, Option<&V>),
        ) {
            let Some(node) = &tree.0 else {
                return;
            };
            let matching = root(other, &node.key);
            if matching.is_some_and(|other| Arc::ptr_eq(node, other)) {
                return;
            }
            walk(&node.left, other, removed, visit);
            if removed {
                if matching.is_none() {
                    visit(&node.key, Some(&node.value), None);
                }
            } else if matching.is_none_or(|other| other.value != node.value) {
                visit(
                    &node.key,
                    matching.map(|other| &other.value),
                    Some(&node.value),
                );
            }
            walk(&node.right, other, removed, visit);
        }
        fn compare<K: Ord, V: PartialEq>(
            new: &ValueIndex<K, V>,
            old: &ValueIndex<K, V>,
            visit: &mut impl FnMut(&K, Option<&V>, Option<&V>),
        ) {
            if new.ptr_eq(old) {
                return;
            }
            if let (Some(a), Some(b)) = (&new.0, &old.0)
                && a.key == b.key
            {
                compare(&a.left, &b.left, visit);
                if a.value != b.value {
                    visit(&a.key, Some(&b.value), Some(&a.value));
                }
                compare(&a.right, &b.right, visit);
            } else {
                walk(new, old, false, visit);
                walk(old, new, true, visit);
            }
        }
        compare(self, previous, &mut visit);
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, value)| value)
    }
    pub fn at(&self, mut index: usize) -> Option<(&K, &V)> {
        let mut node = self.0.as_deref();
        while let Some(at) = node {
            let left = at.left.len();
            if index < left {
                node = at.left.0.as_deref();
            } else if index == left {
                return Some((&at.key, &at.value));
            } else {
                index -= left + 1;
                node = at.right.0.as_deref();
            }
        }
        None
    }
}
impl<K: Ord + Clone + product_codec::Product, V: Clone + PartialEq + product_codec::Product>
    product_codec::Product for ValueIndex<K, V>
{
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (self.len() as u64).encode(out)?;
        for (key, value) in self.iter() {
            key.encode(out)?;
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        let count = u64::decode(input)?;
        let mut result = Self::default();
        for _ in 0..count {
            let key = K::decode(input)?;
            if result
                .last_at_or_before(&key)
                .is_some_and(|(previous, _)| previous == &key)
                || result
                    .at(result.len().saturating_sub(1))
                    .is_some_and(|(previous, _)| previous >= &key)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unordered persistent index",
                ));
            }
            result = result.inserted(key, V::decode(input)?)?;
        }
        Ok(result)
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("ordered-map");
        K::schema(schema);
        V::schema(schema);
    }
}

impl<K: Ord + Clone, V: Clone + PartialEq> std::ops::Index<&K> for ValueIndex<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &V {
        self.get(key).expect("key is present in persistent index")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn checked(tree: &ValueIndex<u32, RetainedValue>) -> (u8, usize) {
        let Some(node) = &tree.0 else { return (0, 0) };
        let (a, an) = checked(&node.left);
        let (b, bn) = checked(&node.right);
        assert!((i16::from(a) - i16::from(b)).abs() <= 1);
        assert_eq!(node.height, 1 + a.max(b));
        assert_eq!(node.len, 1 + an + bn);
        (node.height, node.len)
    }
    type RetainedValue = crate::retained::Retained<String>;
    #[test]
    fn owned_values_and_changed_paths_match_an_independent_map_through_rotations() {
        let mut tree = ValueIndex::default();
        let mut oracle = BTreeMap::new();
        for id in (0..1501u32).map(|i| i * 997 % 1501) {
            let text = id.to_string();
            tree = tree
                .inserted(id, RetainedValue::new(text.clone()))
                .unwrap();
            oracle.insert(id, text);
            checked(&tree);
        }
        let old = tree.clone();
        let old_oracle = oracle.clone();
        assert!(tree.ptr_eq(&tree.inserted(100, tree.get(&100).unwrap().clone()).unwrap()));
        for id in (0..1501u32).step_by(5) {
            tree = tree.removed(&id).unwrap();
            oracle.remove(&id);
            checked(&tree);
        }
        for id in (1..1501u32).step_by(7) {
            let text = format!("changed {id}");
            tree = tree
                .inserted(id, RetainedValue::new(text.clone()))
                .unwrap();
            oracle.insert(id, text);
        }
        let mut changes = BTreeMap::new();
        tree.visit_changes(&old, |key, before, after| {
            assert!(
                changes
                    .insert(
                        *key,
                        (before.map(|x| x.to_string()), after.map(|x| x.to_string()))
                    )
                    .is_none()
            );
        });
        let expected: BTreeMap<_, _> = old_oracle
            .keys()
            .chain(oracle.keys())
            .filter_map(|key| {
                let (a, b) = (old_oracle.get(key), oracle.get(key));
                (a != b).then(|| (*key, (a.cloned(), b.cloned())))
            })
            .collect();
        assert_eq!(changes, expected);
        assert_eq!(
            tree.iter()
                .map(|(k, v)| (*k, v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            oracle
        );
        let mut quiet = 0;
        tree.visit_changes(&tree, |_, _, _| quiet += 1);
        assert_eq!(quiet, 0);
    }
    #[test]
    fn ordered_batch_edits_preserve_avl_metadata_and_the_unchanged_root() {
        let mut tree = ValueIndex::default();
        let mut oracle = BTreeMap::new();
        for round in 0..17u32 {
            let edits: Vec<_> = (0..3001)
                .filter(|key| (key + round) % 5 != 0)
                .map(|key| {
                    let value = (key % (round + 2) != 0).then(|| format!("{round}:{key}"));
                    (
                        key,
                        value.map(RetainedValue::new),
                    )
                })
                .collect();
            for (key, value) in &edits {
                match value {
                    Some(value) => {
                        oracle.insert(*key, value.to_string());
                    }
                    None => {
                        oracle.remove(key);
                    }
                }
            }
            tree = tree.patched(&edits).unwrap();
            checked(&tree);
            assert_eq!(
                tree.iter()
                    .map(|(key, value)| (*key, value.to_string()))
                    .collect::<BTreeMap<_, _>>(),
                oracle
            );
            assert!(tree.ptr_eq(&tree.patched(&edits).unwrap()));
            assert!(tree.ptr_eq(&tree.patched(&[]).unwrap()));
        }
        assert!(tree.patched(&[(2, None), (1, None)]).is_err());
        assert!(tree.patched(&[(2, None), (2, None)]).is_err());
        let removed: Vec<_> = oracle.keys().map(|key| (*key, None)).collect();
        assert!(tree.patched(&removed).unwrap().is_empty());
    }
    #[test]
    fn persistent_map_codec_retains_order_and_rejects_duplicate_keys() {
        let mut values = ValueIndex::default();
        for id in 0..1501u32 {
            values = values.inserted(id, id + 1).unwrap();
        }
        let bytes = product_codec::encode(&values).unwrap();
        assert_eq!(
            product_codec::decode::<ValueIndex<u32, u32>>(&bytes).unwrap(),
            values
        );
        let bytes: Vec<_> = 2u64
            .to_le_bytes()
            .into_iter()
            .chain([2u32, 4, 2, 5].into_iter().flat_map(u32::to_le_bytes))
            .collect();
        assert!(product_codec::decode::<ValueIndex<u32, u32>>(&bytes).is_err());
    }
}

impl<K: Ord + Clone, V: Clone + PartialEq> TryFrom<std::collections::BTreeMap<K, V>>
    for ValueIndex<K, V>
{
    type Error = io::Error;
    fn try_from(values: std::collections::BTreeMap<K, V>) -> io::Result<Self> {
        Self::try_from_iter(values)
    }
}

impl<'a, K, V> IntoIterator for &'a ValueIndex<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
