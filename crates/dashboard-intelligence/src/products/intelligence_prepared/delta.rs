//! How a segment's operator hands on what changed of a read it holds whole: a list or map in
//! ascending key order by the entries that changed or came and the keys gone, or a hash map by
//! the keys whose values changed or went.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    hash::Hash,
    mem::size_of,
};

/// What `new` changed of `old`, both in ascending order of `key` with each key once, as tables
/// hold their rows: the entries that changed or came, in order, and the keys of those gone, in
/// order.
pub fn sorted<T: Clone + PartialEq, K: Ord>(
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> K,
) -> (Vec<T>, Vec<K>) {
    let (mut set, mut gone) = (Vec::new(), Vec::new());
    let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
    loop {
        match (old.peek(), new.peek()) {
            (Some(before), Some(after)) => match key(before).cmp(&key(after)) {
                Ordering::Less => {
                    gone.push(key(before));
                    old.next();
                }
                Ordering::Greater => {
                    set.push((*after).clone());
                    new.next();
                }
                Ordering::Equal => {
                    if before != after {
                        set.push((*after).clone());
                    }
                    old.next();
                    new.next();
                }
            },
            (Some(before), None) => {
                gone.push(key(before));
                old.next();
            }
            (None, Some(after)) => {
                set.push((*after).clone());
                new.next();
            }
            (None, None) => return (set, gone),
        }
    }
}

/// What `new` changed of `old`: the entries that changed or came, in key order, and the keys of
/// those gone.
pub fn mapped<K: Ord + Clone, V: Clone + PartialEq>(
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
) -> (Vec<(K, V)>, Vec<K>) {
    let (mut set, mut gone) = (Vec::new(), Vec::new());
    let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
    loop {
        match (old.peek(), new.peek()) {
            (Some((before, held)), Some((after, value))) => match before.cmp(after) {
                Ordering::Less => {
                    gone.push((*before).clone());
                    old.next();
                }
                Ordering::Greater => {
                    set.push(((*after).clone(), (*value).clone()));
                    new.next();
                }
                Ordering::Equal => {
                    if held != value {
                        set.push(((*after).clone(), (*value).clone()));
                    }
                    old.next();
                    new.next();
                }
            },
            (Some((before, _)), None) => {
                gone.push((*before).clone());
                old.next();
            }
            (None, Some((after, value))) => {
                set.push(((*after).clone(), (*value).clone()));
                new.next();
            }
            (None, None) => return (set, gone),
        }
    }
}

/// A map as read again: the keys whose values changed, and those gone; whole replaces it.
#[derive(replay_spool::Spooled)]
pub struct Keyed<K, V> {
    pub whole: bool,
    pub set: Vec<(K, V)>,
    pub gone: Vec<K>,
}

impl<K: Clone + Eq + Hash, V: Clone + PartialEq> Keyed<K, V> {
    /// What `new` changed of `old`: none when equal.
    pub fn of(old: Option<&HashMap<K, V>>, new: &HashMap<K, V>) -> Option<Self> {
        let Some(old) = old else {
            return Some(Self {
                whole: true,
                set: new.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                gone: Vec::new(),
            });
        };
        let set: Vec<(K, V)> = new
            .iter()
            .filter(|(key, value)| old.get(*key) != Some(*value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let gone: Vec<K> = old
            .keys()
            .filter(|key| !new.contains_key(*key))
            .cloned()
            .collect();
        (!set.is_empty() || !gone.is_empty()).then_some(Self {
            whole: false,
            set,
            gone,
        })
    }
}

impl<K: Eq + Hash, V> Keyed<K, V> {
    pub fn apply(self, to: &mut HashMap<K, V>) {
        if self.whole {
            to.clear();
        }
        for key in self.gone {
            to.remove(&key);
        }
        to.extend(self.set);
    }

    pub fn bytes(&self, heap: impl Fn(&V) -> usize) -> usize {
        self.set.capacity() * size_of::<(K, V)>()
            + self.set.iter().map(|(_, value)| heap(value)).sum::<usize>()
            + self.gone.capacity() * size_of::<K>()
    }
}

/// What an optional string holds.
pub fn optional(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}
