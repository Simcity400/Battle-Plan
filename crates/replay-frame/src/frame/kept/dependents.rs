
use super::{Change, Changes, Read};
use crate::rows::TableId;
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::Arc,
};

pub trait Grouped {
    fn table(&self) -> Option<TableId>;
}

#[derive(Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
struct Readers<K> {
    rows: HashMap<u128, Vec<(K, u64)>>,
    ranges: Vec<(u128, u128, u64, K)>,
    widest: u128,
    whole: Vec<(u64, K)>,
    fields: u64,
}

impl<K> Default for Readers<K> {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            ranges: Vec::new(),
            widest: 0,
            whole: Vec::new(),
            fields: 0,
        }
    }
}

impl<K: Clone> Readers<K> {
    fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.ranges.is_empty() && self.whole.is_empty()
    }

    fn reading(&self, fields: u64, out: &mut Vec<K>) {
        for readers in self.rows.values() {
            out.extend(readers.iter().filter(|(_, read)| read & fields != 0).map(|(k, _)| k.clone()));
        }
        out.extend(self.ranges.iter().filter(|r| r.2 & fields != 0).map(|r| r.3.clone()));
        out.extend(self.whole.iter().filter(|(read, _)| read & fields != 0).map(|(_, k)| k.clone()));
    }

    fn changed(&self, changed: &[(u128, u64)], deleted: &[u128], out: &mut Vec<K>) {
        let mut wrote = 0;
        for &(key, fields) in changed {
            self.at(key, fields, out);
            wrote |= fields;
        }
        for &key in deleted {
            self.at(key, u64::MAX, out);
        }
        let rows = if deleted.is_empty() { wrote } else { u64::MAX };
        out.extend(self.whole.iter().filter(|(read, _)| read & rows != 0).map(|(_, k)| k.clone()));
    }

    fn at(&self, key: u128, fields: u64, out: &mut Vec<K>) {
        if let Some(readers) = self.rows.get(&key) {
            out.extend(readers.iter().filter(|(_, read)| read & fields != 0).map(|(k, _)| k.clone()));
        }
        let end = self.ranges.partition_point(|range| range.0 <= key);
        let floor = key.saturating_sub(self.widest);
        for range in self.ranges[..end].iter().rev() {
            if range.0 < floor {
                break;
            }
            if key < range.1 && range.2 & fields != 0 {
                out.push(range.3.clone());
            }
        }
    }
}

#[derive(Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Dependents<K, D> {
    of: HashMap<K, Arc<[Read<D>]>>,
    tables: HashMap<TableId, Readers<K>>,
    derived: HashMap<Option<TableId>, HashMap<D, Vec<K>>>,
}

impl<K, D> Default for Dependents<K, D> {
    fn default() -> Self {
        Self {
            of: HashMap::new(),
            tables: HashMap::new(),
            derived: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, D: Clone + Eq + Hash + Grouped> Dependents<K, D> {
    pub fn reads(&self, key: &K) -> Option<&Arc<[Read<D>]>> {
        self.of.get(key)
    }

    pub fn len(&self) -> usize {
        self.of.len()
    }

    pub fn is_empty(&self) -> bool {
        self.of.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.of.keys()
    }

    pub fn set(&mut self, key: K, reads: Arc<[Read<D>]>) {
        if let Some(held) = self.of.get(&key)
            && (Arc::ptr_eq(held, &reads) || **held == *reads)
        {
            return;
        }
        self.remove(&key);
        for read in reads.iter() {
            match read {
                Read::Row { table, key: row, fields } => {
                    let readers = self.tables.entry(*table).or_default();
                    readers.fields |= fields;
                    readers.rows.entry(*row).or_default().push((key.clone(), *fields));
                }
                Read::Range { table, from, to, fields } => {
                    let readers = self.tables.entry(*table).or_default();
                    readers.fields |= fields;
                    let at = readers.ranges.partition_point(|range| range.0 <= *from);
                    readers.ranges.insert(at, (*from, *to, *fields, key.clone()));
                    readers.widest = readers.widest.max(to.saturating_sub(*from));
                }
                Read::Table { table, fields } => {
                    let readers = self.tables.entry(*table).or_default();
                    readers.fields |= fields;
                    readers.whole.push((*fields, key.clone()));
                }
                Read::Derived(read) => self
                    .derived
                    .entry(read.table())
                    .or_default()
                    .entry(read.clone())
                    .or_default()
                    .push(key.clone()),
            }
        }
        self.of.insert(key, reads);
    }

    pub fn remove(&mut self, key: &K) {
        let Some(reads) = self.of.remove(key) else {
            return;
        };
        for read in reads.iter() {
            match read {
                Read::Row { table, key: row, .. } => {
                    if let Some(readers) = self.tables.get_mut(table)
                        && let Some(held) = readers.rows.get_mut(row)
                    {
                        held.retain(|(k, _)| k != key);
                        if held.is_empty() {
                            readers.rows.remove(row);
                        }
                    }
                }
                Read::Range { table, from, .. } => {
                    if let Some(readers) = self.tables.get_mut(table) {
                        let start = readers.ranges.partition_point(|range| range.0 < *from);
                        if let Some(at) = readers.ranges[start..]
                            .iter()
                            .take_while(|range| range.0 == *from)
                            .position(|range| range.3 == *key)
                        {
                            readers.ranges.remove(start + at);
                        }
                    }
                }
                Read::Table { table, .. } => {
                    if let Some(readers) = self.tables.get_mut(table) {
                        readers.whole.retain(|(_, k)| k != key);
                    }
                }
                Read::Derived(read) => {
                    if let Some(group) = self.derived.get_mut(&read.table())
                        && let Some(held) = group.get_mut(read)
                    {
                        held.retain(|k| k != key);
                        if held.is_empty() {
                            group.remove(read);
                        }
                    }
                }
            }
            if let Some(table) = read.table()
                && self.tables.get(&table).is_some_and(Readers::is_empty)
            {
                self.tables.remove(&table);
            }
        }
    }

    pub fn reached(
        &self,
        changes: &(impl Changes + ?Sized),
        mut derived: impl FnMut(&D) -> bool,
    ) -> Vec<K> {
        let mut out = Vec::new();
        for (&table, readers) in &self.tables {
            let change = changes.table(table);
            let rows = match change {
                Change::Same => continue,
                Change::Fields(fields) if fields & readers.fields == 0 => continue,
                Change::Rows(changed, deleted) => Some((changed, deleted)),
                Change::Fields(_) | Change::Any => changes.rows(table),
            };
            match (rows, change) {
                (Some((changed, deleted)), _) => readers.changed(changed, deleted, &mut out),
                (None, Change::Fields(fields)) => readers.reading(fields, &mut out),
                (None, _) => readers.reading(u64::MAX, &mut out),
            }
        }
        for (table, group) in &self.derived {
            if table.is_some_and(|table| matches!(changes.table(table), Change::Same)) {
                continue;
            }
            for (read, readers) in group {
                if derived(read) {
                    out.extend(readers.iter().cloned());
                }
            }
        }
        let mut seen = HashSet::with_capacity(out.len());
        out.retain(|key| seen.insert(key.clone()));
        out
    }
}
