
pub struct Recent<K, V, const N: usize> {
    entries: [Option<(K, V)>; N],
    next: usize,
}

impl<K, V, const N: usize> Default for Recent<K, V, N> {
    fn default() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            next: 0,
        }
    }
}

impl<K: PartialEq, V: Copy, const N: usize> Recent<K, V, N> {
    pub fn get(&self, key: &K) -> Option<V> {
        self.entries
            .iter()
            .flatten()
            .find(|(k, _)| k == key)
            .map(|&(_, value)| value)
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.entries[self.next] = Some((key, value));
        self.next = (self.next + 1) % N;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_observations_and_replaces_the_oldest() {
        let mut recent = Recent::<u32, u8, 2>::default();
        assert!(recent.is_empty());
        recent.insert(1, 10);
        recent.insert(2, 20);
        assert_eq!((recent.get(&1), recent.get(&2)), (Some(10), Some(20)));
        recent.insert(3, 30);
        assert_eq!(recent.get(&1), None);
        assert_eq!((recent.get(&2), recent.get(&3)), (Some(20), Some(30)));
    }
}
