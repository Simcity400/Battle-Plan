//! How a segment's operator hands on what changed of a read it holds whole: a list by the
//! entries that differ where the list keeps its order of keys, or a map by the keys whose values
//! changed or went.

use std::{collections::HashMap, hash::Hash, mem::size_of};

/// A list as read again: whole, or the positions whose entries changed.
#[derive(replay_spool::Spooled)]
pub enum Listed<T> {
    Whole(Vec<T>),
    Patched(Vec<(u32, T)>),
}

impl<T: Clone + PartialEq> Listed<T> {
    /// What `new` changed of `old`, entries keyed by `key`: none when equal.
    pub fn of<K: PartialEq>(old: Option<&[T]>, new: &[T], key: impl Fn(&T) -> K) -> Option<Self> {
        let Some(old) = old.filter(|old| {
            old.len() == new.len() && old.iter().zip(new).all(|(old, new)| key(old) == key(new))
        }) else {
            return Some(Self::Whole(new.to_vec()));
        };
        let patched: Vec<(u32, T)> = (0_u32..)
            .zip(old.iter().zip(new))
            .filter(|(_, (old, new))| old != new)
            .map(|(position, (_, new))| (position, new.clone()))
            .collect();
        (!patched.is_empty()).then_some(Self::Patched(patched))
    }
}

impl<T> Listed<T> {
    pub fn apply(self, to: &mut Vec<T>) {
        match self {
            Self::Whole(whole) => *to = whole,
            Self::Patched(patched) => {
                for (position, entry) in patched {
                    to[position as usize] = entry;
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Whole(whole) => whole.len(),
            Self::Patched(patched) => patched.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes it holds, `heap` giving each entry's own.
    pub fn bytes(&self, heap: impl Fn(&T) -> usize) -> usize {
        match self {
            Self::Whole(whole) => {
                whole.capacity() * size_of::<T>() + whole.iter().map(&heap).sum::<usize>()
            }
            Self::Patched(patched) => {
                patched.capacity() * size_of::<(u32, T)>()
                    + patched.iter().map(|(_, entry)| heap(entry)).sum::<usize>()
            }
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
