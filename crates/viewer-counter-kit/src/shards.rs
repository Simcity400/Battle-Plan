
use std::{collections::BTreeMap, fmt, sync::Arc};

const SHIFT: u32 = 6;

pub trait Sharded: Ord + Copy {
    fn shard(&self) -> u32;
}

impl Sharded for u32 {
    fn shard(&self) -> u32 {
        self >> SHIFT
    }
}

impl<T: Ord + Copy> Sharded for (u32, T) {
    fn shard(&self) -> u32 {
        self.0 >> SHIFT
    }
}

pub struct Shards<K, V> {
    shards: BTreeMap<u32, Arc<BTreeMap<K, V>>>,
    len: usize,
}

impl<K, V> Clone for Shards<K, V> {
    fn clone(&self) -> Self {
        Self { shards: self.shards.clone(), len: self.len }
    }
}

impl<K, V> Default for Shards<K, V> {
    fn default() -> Self {
        Self { shards: BTreeMap::new(), len: 0 }
    }
}

impl<K: Sharded, V: Clone + PartialEq> Shards<K, V> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.shards.get(&key.shard())?.get(key)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> + '_ {
        self.shards.values().flat_map(|shard| shard.iter())
    }

    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.iter().map(|(&key, _)| key)
    }

    pub fn insert(&mut self, key: K, value: V) -> bool {
        if self.get(&key) == Some(&value) {
            return false;
        }
        let shard = Arc::make_mut(self.shards.entry(key.shard()).or_default());
        if shard.insert(key, value).is_none() {
            self.len += 1;
        }
        true
    }

    pub fn remove(&mut self, key: &K) -> bool {
        if !self.contains_key(key) {
            return false;
        }
        let at = key.shard();
        let shard = self.shards.get_mut(&at).expect("the key's shard");
        Arc::make_mut(shard).remove(key);
        if shard.is_empty() {
            self.shards.remove(&at);
        }
        self.len -= 1;
        true
    }

    pub fn assign(&mut self, entries: impl IntoIterator<Item = (K, V)>) {
        let made: Self = entries.into_iter().collect();
        let mut kept = BTreeMap::new();
        for (at, shard) in made.shards {
            match self.shards.remove(&at) {
                Some(held) if *held == *shard => kept.insert(at, held),
                _ => kept.insert(at, shard),
            };
        }
        (self.shards, self.len) = (kept, made.len);
    }

    pub fn differing<'a>(&'a self, old: &'a Self) -> impl Iterator<Item = K> + 'a {
        let mut ats: Vec<u32> = self.shards.keys().chain(old.shards.keys()).copied().collect();
        ats.sort_unstable();
        ats.dedup();
        ats.into_iter().flat_map(move |at| {
            let (new, held) = (self.shards.get(&at), old.shards.get(&at));
            let shared = matches!((new, held), (Some(new), Some(held)) if Arc::ptr_eq(new, held));
            let mut keys: Vec<K> = Vec::new();
            if !shared {
                let empty = BTreeMap::new();
                let (new, held) = (new.map_or(&empty, |shard| &**shard), held.map_or(&empty, |shard| &**shard));
                keys.extend(new.iter().filter(|(key, value)| held.get(key) != Some(value)).map(|(key, _)| *key));
                keys.extend(held.keys().filter(|key| !new.contains_key(key)).copied());
                keys.sort_unstable();
            }
            keys
        })
    }

    pub fn keys_from(&self, from: K) -> impl Iterator<Item = K> + '_ {
        self.shards.range(from.shard()..).flat_map(move |(_, shard)| shard.range(from..).map(|(key, _)| *key))
    }

    pub fn shared_with(&self, other: &Self) -> (usize, usize) {
        let shared = self.shards.iter().filter(|(at, shard)| other.shards.get(at).is_some_and(|held| Arc::ptr_eq(shard, held))).count();
        (shared, self.shards.len())
    }

    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }
}

impl<K: Sharded, V: Clone + PartialEq> FromIterator<(K, V)> for Shards<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut shards: BTreeMap<u32, BTreeMap<K, V>> = BTreeMap::new();
        for (key, value) in entries {
            shards.entry(key.shard()).or_default().insert(key, value);
        }
        let len = shards.values().map(BTreeMap::len).sum();
        Self { shards: shards.into_iter().map(|(at, shard)| (at, Arc::new(shard))).collect(), len }
    }
}

impl<K: Sharded, V: Clone + PartialEq> std::ops::Index<&K> for Shards<K, V> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("an entry of the key")
    }
}

impl<K: Sharded, V: Clone + PartialEq> PartialEq for Shards<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.shards.len() == other.shards.len() && self.shards.iter().zip(&other.shards).all(|((a, x), (b, y))| a == b && (Arc::ptr_eq(x, y) || x == y))
    }
}

impl<K: Sharded + fmt::Debug, V: Clone + PartialEq + fmt::Debug> fmt::Debug for Shards<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{BTreeMap, Shards};

    #[test]
    fn a_change_copies_its_shard_alone_and_reads_as_the_whole_map() {
        let old: Shards<u32, u32> = (0..1_000).map(|key| (key, key)).collect();
        let mut new = old.clone();
        assert!(!new.insert(70, 70), "an equal value changes nothing");
        assert_eq!(new.shared_with(&old), (old.shard_count(), old.shard_count()));
        assert!(new.insert(70, 1));
        assert_eq!(new.shared_with(&old), (old.shard_count() - 1, old.shard_count()), "one shard is copied, the rest shared");
        assert!(new.remove(&999) && new.insert(5_000, 2));
        assert_eq!(new.differing(&old).collect::<Vec<_>>(), [70, 999, 5_000]);
        assert_eq!(new.len(), 1_000);
        let whole: BTreeMap<u32, u32> = (0..999).map(|key| (key, if key == 70 { 1 } else { key })).chain([(5_000, 2)]).collect();
        assert!(new.iter().map(|(key, value)| (*key, *value)).eq(whole.clone()), "in key order");
        let mut assigned = old.clone();
        assigned.assign(whole);
        assert_eq!(assigned, new);
        assert_eq!(assigned.shared_with(&old).0, old.shard_count() - 2, "assigning keeps every unchanged shard");
    }
}
