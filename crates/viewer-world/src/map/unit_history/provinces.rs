//! Retained province records and their hourly operand joins.
use super::{Entity, ModelInstance, Province, ProvinceTargets, Targets, naval};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

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

    pub fn prepare_inputs(&mut self, targets: &Targets, naval: &naval::History) {
        let _join_span = viewer_foundation::profiling::span("units_prepare_inputs");
        for record in &mut self.records {
            record.targets = targets.0.provinces.get(&record.id).map(Arc::clone);
            record.has_candidates = naval.has_candidates(record.id);
        }
    }

    pub fn get(&self, id: &u32) -> Option<&Province> {
        self.indices.get(id).map(|&at| &self.records[at].province)
    }

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
