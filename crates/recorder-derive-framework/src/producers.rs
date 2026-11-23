
use crate::framework::{Columns, Hour};
use replay_data::rows::TableId;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const SLOT: [usize; 1] = [0];

#[derive(Default)]
pub struct Producers {
    links: HashMap<u32, u32>,
    holders: HashMap<u32, u32>,
    ids: HashSet<u32>,
    aliases: HashMap<u32, Vec<u32>>,
}

impl Producers {
    pub fn read(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        self.holders.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            if let (Some(key), Some(canonical)) = (links.key(slot), links.number(SLOT[0], slot)) {
                self.links.insert(key as u32, canonical as u32);
            }
        }
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            let (Some(key), Some(canonical)) = (countries.key(slot), countries.number(SLOT[0], slot)) else { continue };
            let (id, canonical) = (key as u32, canonical as u32);
            if self.links.get(&id) == Some(&canonical) {
                let held = self.holders.entry(canonical).or_insert(id);
                *held = (*held).min(id);
            }
        }
        self.ids.clear();
        self.ids.extend(self.holders.values().copied());
        self.aliases.clear();
        for (&raw, &slot) in &self.links {
            self.aliases.entry(slot).or_default().push(raw);
        }
        for raws in self.aliases.values_mut() {
            raws.sort_unstable();
        }
    }

    pub fn moved(hour: &Hour<'_>) -> bool {
        hour.wrote(TableId::CountryLinks, Columns::These(&SLOT)) || hour.wrote(TableId::Countries, Columns::These(&SLOT))
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        (raw != 0).then(|| self.links.get(&raw).copied()).flatten()
    }

    pub fn of_slot(&self, slot: u32) -> Option<u32> {
        self.holders.get(&slot).copied()
    }

    pub fn of(&self, raw: u32) -> Option<u32> {
        self.of_slot(self.slot(raw)?)
    }

    pub fn holders(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.holders.iter().map(|(&slot, &id)| (slot, id))
    }

    pub fn aliases(&self, slot: u32) -> &[u32] {
        self.aliases.get(&slot).map_or(&[], Vec::as_slice)
    }

    pub fn links(&self) -> BTreeMap<u32, u32> {
        self.links.iter().map(|(&raw, &slot)| (raw, slot)).collect()
    }

    pub fn live(&self, id: u32) -> bool {
        self.ids.contains(&id)
    }
}
