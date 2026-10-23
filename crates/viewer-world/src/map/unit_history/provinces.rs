//! Retained province records and their hourly operand joins.
use super::{Entity, ModelInstance, Province, ProvinceTargets, Targets, naval};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[cfg(test)]
mod tests;

#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
pub(super) struct Record {
    pub id: u32,
    pub province: Province,
    pub targets: Option<Arc<ProvinceTargets>>,
    pub has_candidates: bool,
}

#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
pub(super) struct Provinces {
    pub records: Vec<Record>,
    pub indices: BTreeMap<u32, usize>,
    pub populated: BTreeSet<usize>,
    /// Select the original point-lookup join without changing process environment.
    #[cfg(test)]
    pub lookup_join: Option<bool>,
    /// The targets and candidate provinces the last sparse join installed ([`Self::join_sparse`]).
    joined: Option<(Targets, Vec<u32>)>,
    /// Select the original full-walk install without changing process environment.
    #[cfg(test)]
    pub sparse_install: Option<bool>,
}

impl Provinces {
    /// Add only new keys, then restore native province order. Sparse IDs never size storage.
    pub fn ensure(&mut self, ids: impl IntoIterator<Item = u32>) -> bool {
        let before = self.records.len();
        let mut added: Vec<_> = ids
            .into_iter()
            .filter(|id| !self.indices.contains_key(id))
            .collect();
        added.sort_unstable();
        added.dedup();
        if added.is_empty() {
            return false;
        }
        self.records.extend(added.into_iter().map(|id| Record {
            id,
            ..Default::default()
        }));
        self.records.sort_unstable_by_key(|record| record.id);
        self.indices.clear();
        self.populated.clear();
        for (index, record) in self.records.iter().enumerate() {
            self.indices.insert(record.id, index);
            if !record.province.entities.is_empty() {
                self.populated.insert(index);
            }
        }
        self.records.len() != before
    }

    pub fn install_targets(&mut self, from: &Targets, to: &Targets) -> bool {
        #[cfg(test)]
        if self.sparse_install == Some(false) {
            return self.install_full(from, to);
        }
        let changed = self.ensure_ascending(to.0.provinces.keys().copied());
        self.install_lands(from, to);
        changed
    }

    #[cfg(test)]
    fn install_full(&mut self, from: &Targets, to: &Targets) -> bool {
        for id in from.0.provinces.keys() {
            if !to.0.provinces.contains_key(id) {
                self.get_mut(id).unwrap().lands = Arc::default();
            }
        }
        let changed = self.ensure(to.0.provinces.keys().copied());
        for (&id, prepared) in &to.0.provinces {
            self.get_mut(&id).unwrap().lands = Arc::clone(&prepared.lands);
        }
        changed
    }

    /// The first record at or after `from` whose id is not below `id`.
    fn locate(&self, from: usize, id: u32) -> usize {
        from + self.records[from..].partition_point(|record| record.id < id)
    }

    /// [`Self::ensure`], finding the missing ids of an ascending list by one forward walk.
    pub fn ensure_ascending(&mut self, ids: impl IntoIterator<Item = u32>) -> bool {
        let (mut at, mut last) = (0, None);
        let missing: Vec<_> = ids
            .into_iter()
            .filter(|&id| {
                if last.is_some_and(|last| id < last) {
                    at = 0;
                }
                last = Some(id);
                at = self.locate(at, id);
                self.records.get(at).is_none_or(|record| record.id != id)
            })
            .collect();
        !missing.is_empty() && self.ensure(missing)
    }

    /// Only the two hours' union can gain or lose a land list.
    pub fn install_lands(&mut self, from: &Targets, to: &Targets) {
        let mut old = from.0.provinces.keys().copied().peekable();
        let mut new = to.0.provinces.iter().peekable();
        let mut at = 0;
        loop {
            let id = match (old.peek(), new.peek()) {
                (None, None) => break,
                (Some(&id), None) | (None, Some(&(&id, _))) => id,
                (Some(&a), Some(&(&b, _))) => a.min(b),
            };
            let was = old.next_if_eq(&id).is_some();
            let lands = new
                .next_if(|&(&key, _)| key == id)
                .map(|(_, prepared)| Arc::clone(&prepared.lands));
            at = self.locate(at, id);
            // The full walk's lookup refuses a province without a record the same way.
            let index = if self.records.get(at).is_some_and(|record| record.id == id) {
                at
            } else {
                self.indices[&id]
            };
            let province = &mut self.records[index].province;
            match lands {
                Some(lands) => province.lands = lands,
                None if was => province.lands = Arc::default(),
                None => {}
            }
        }
    }

    pub fn prepare_inputs(&mut self, targets: &Targets, naval: &naval::History) {
        let _join_span = viewer_foundation::profiling::span("units_prepare_inputs");
        #[cfg(test)]
        if self.sparse_install == Some(false) {
            self.joined = None;
            if self.lookup_join == Some(true) {
                self.join_by_lookup(targets, naval);
            } else {
                self.join_by_merge(targets, naval);
            }
            return;
        }
        self.join_sparse(targets, naval);
    }

    /// All inputs ascend by province ID, so each cursor only moves forward.
    fn join_by_merge(&mut self, targets: &Targets, naval: &naval::History) {
        let mut prepared = targets.0.provinces.iter().peekable();
        let mut candidates = naval.candidate_provinces().peekable();
        for record in &mut self.records {
            let id = record.id;
            while prepared.next_if(|&(&key, _)| key < id).is_some() {}
            record.targets = prepared
                .next_if(|&(&key, _)| key == id)
                .map(|(_, joined)| Arc::clone(joined));
            while candidates.next_if(|&key| key < id).is_some() {}
            record.has_candidates = candidates.next_if_eq(&id).is_some();
        }
    }

    /// Only the last and current joins' union can differ; missing records require a full walk.
    fn join_sparse(&mut self, targets: &Targets, naval: &naval::History) {
        let candidates: Vec<u32> = naval.candidate_provinces().collect();
        let Some((joined, joined_candidates)) = self.joined.take() else {
            self.join_by_merge(targets, naval);
            self.joined = Some((targets.clone(), candidates));
            return;
        };
        let mut ids: Vec<u32> = joined
            .0
            .provinces
            .keys()
            .copied()
            .chain(joined_candidates)
            .chain(targets.0.provinces.keys().copied())
            .chain(candidates.iter().copied())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut prepared = targets.0.provinces.iter().peekable();
        let mut current = candidates.iter().copied().peekable();
        let (mut at, mut complete) = (0, true);
        for id in ids {
            at = self.locate(at, id);
            let Some(record) = self.records.get_mut(at).filter(|record| record.id == id) else {
                complete = false;
                break;
            };
            while prepared.next_if(|&(&key, _)| key < id).is_some() {}
            record.targets = prepared
                .next_if(|&(&key, _)| key == id)
                .map(|(_, joined)| Arc::clone(joined));
            while current.next_if(|&key| key < id).is_some() {}
            record.has_candidates = current.next_if_eq(&id).is_some();
        }
        if !complete {
            self.join_by_merge(targets, naval);
        }
        self.joined = Some((targets.clone(), candidates));
    }

    #[cfg(test)]
    fn join_by_lookup(&mut self, targets: &Targets, naval: &naval::History) {
        for record in &mut self.records {
            record.targets = targets.0.provinces.get(&record.id).map(Arc::clone);
            record.has_candidates = naval.has_candidates(record.id);
        }
    }

    pub fn get(&self, id: &u32) -> Option<&Province> {
        self.indices.get(id).map(|&at| &self.records[at].province)
    }

    #[cfg(test)]
    pub fn get_mut(&mut self, id: &u32) -> Option<&mut Province> {
        self.indices
            .get(id)
            .map(|&at| &mut self.records[at].province)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &Province)> {
        self.records
            .iter()
            .map(|record| (&record.id, &record.province))
    }

    pub fn keys(&self) -> impl Iterator<Item = &u32> {
        self.iter().map(|(id, _)| id)
    }

    pub fn reset(&mut self, selected: Option<&BTreeSet<u32>>) {
        for (at, record) in self.records.iter_mut().enumerate() {
            if selected.is_none_or(|ids| ids.contains(&record.id)) {
                record.province = Province {
                    land_cursor: record.province.land_cursor,
                    lands: std::mem::take(&mut record.province.lands),
                    ..Default::default()
                };
                self.populated.remove(&at);
            }
        }
    }

    pub fn placed(&self) -> Vec<(usize, ModelInstance)> {
        self.populated
            .iter()
            .flat_map(|&at| {
                self.records[at]
                    .province
                    .entities
                    .values()
                    .flat_map(Entity::placed)
            })
            .collect()
    }

    #[cfg(test)]
    pub fn populated_ids(&self) -> BTreeSet<u32> {
        self.populated
            .iter()
            .map(|&at| self.records[at].id)
            .collect()
    }
}

impl std::ops::Index<&u32> for Provinces {
    type Output = Province;
    fn index(&self, id: &u32) -> &Self::Output {
        self.get(id).unwrap()
    }
}

#[cfg(test)]
impl PartialEq for Provinces {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
