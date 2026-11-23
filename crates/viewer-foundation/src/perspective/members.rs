use std::collections::{BTreeSet, btree_set};

pub(super) enum Members<T> {
    Empty,
    One(T),
    Many(BTreeSet<T>),
}
impl<T> Default for Members<T> {
    fn default() -> Self {
        Self::Empty
    }
}
impl<T: Ord + Copy> Members<T> {
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(_) => 1,
            Self::Many(values) => values.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn insert(&mut self, value: T) {
        match self {
            Self::Empty => *self = Self::One(value),
            Self::One(held) if *held != value => {
                *self = Self::Many(BTreeSet::from([*held, value]));
            }
            Self::One(_) => {}
            Self::Many(values) => {
                values.insert(value);
            }
        }
    }
    pub fn remove(&mut self, value: &T) {
        match self {
            Self::One(held) if held == value => *self = Self::Empty,
            Self::Many(values) => {
                values.remove(value);
                if values.len() == 1 {
                    *self = Self::One(*values.first().unwrap());
                }
            }
            _ => {}
        }
    }
    pub fn last(&self) -> Option<&T> {
        match self {
            Self::Empty => None,
            Self::One(value) => Some(value),
            Self::Many(values) => values.last(),
        }
    }
    pub fn iter(&self) -> Iter<'_, T> {
        match self {
            Self::Empty => Iter::One(None),
            Self::One(value) => Iter::One(Some(value)),
            Self::Many(values) => Iter::Many(values.iter()),
        }
    }
}
pub(super) enum Iter<'a, T> {
    One(Option<&'a T>),
    Many(btree_set::Iter<'a, T>),
}
impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(value) => value.take(),
            Self::Many(values) => values.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self {
            Self::One(value) => usize::from(value.is_some()),
            Self::Many(values) => values.len(),
        };
        (len, Some(len))
    }
}
impl<T: Ord + Copy> ExactSizeIterator for Iter<'_, T> {}
impl<'a, T: Ord + Copy> IntoIterator for &'a Members<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_members_preserve_order_across_singleton_and_dense_transitions() {
        for count in [3u64, 1500, 5000] {
            let mut actual = Members::default();
            let mut expected = BTreeSet::new();
            let keys: Vec<_> = (0..count)
                .map(|key| key.wrapping_mul(0x9e3779b97f4a7c15))
                .chain([u64::MAX, 1 << 63])
                .collect();
            for &key in &keys {
                actual.insert(key);
                actual.insert(key);
                expected.insert(key);
                assert_eq!(actual.len(), expected.len());
            }
            assert!(actual.iter().copied().eq(expected.iter().copied()));
            assert_eq!(actual.last(), expected.last());
            for key in keys {
                assert_eq!(
                    actual.iter().any(|held| *held == key),
                    expected.contains(&key)
                );
                actual.remove(&key);
                expected.remove(&key);
                assert_eq!(actual.len(), expected.len());
                assert!(actual.iter().copied().eq(expected.iter().copied()));
            }
            assert!(actual.is_empty());
            actual.insert(7);
            assert!(matches!(actual, Members::One(7)));
            actual.remove(&8);
            assert!(matches!(actual, Members::One(7)));
            actual.remove(&7);
            assert!(matches!(actual, Members::Empty));
        }
    }
}
