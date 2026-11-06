//! A source's per-branch memo of its latest read.

use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

/// Each branch's latest value by key; an earlier one lives only while a request holds it.
pub struct BranchLatest<K, V> {
    latest: HashMap<u64, (K, Arc<V>)>,
    earlier: Vec<(u64, K, Weak<V>)>,
}

impl<K, V> Default for BranchLatest<K, V> {
    fn default() -> Self {
        Self {
            latest: HashMap::new(),
            earlier: Vec::new(),
        }
    }
}

impl<K: PartialEq, V> BranchLatest<K, V> {
    pub fn get(&self, branch: u64, key: &K) -> Option<Arc<V>> {
        if let Some((held, value)) = self.latest.get(&branch)
            && held == key
        {
            return Some(Arc::clone(value));
        }
        self.earlier
            .iter()
            .find(|(at, held, _)| *at == branch && held == key)
            .and_then(|(_, _, value)| value.upgrade())
    }

    pub fn put(&mut self, branch: u64, key: K, value: &Arc<V>) {
        self.earlier.retain(|(at, held, earlier)| {
            earlier.strong_count() > 0 && !(*at == branch && *held == key)
        });
        if let Some((held, earlier)) = self.latest.insert(branch, (key, Arc::clone(value)))
            && self.latest[&branch].0 != held
        {
            self.earlier.push((branch, held, Arc::downgrade(&earlier)));
        }
    }

    /// The values still alive: each branch's latest, and earlier ones a request holds.
    pub fn held(&self) -> usize {
        self.latest.len()
            + self
                .earlier
                .iter()
                .filter(|(_, _, value)| value.strong_count() > 0)
                .count()
    }
}
