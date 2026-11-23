
use crate::framework::{Columns, Hour};
use replay_data::rows::TableId;
use std::collections::HashMap;

pub type Identity = (u32, u32);

pub const COUNTRY_SLOT: [usize; 1] = [0];
pub const LINK_SLOT: [usize; 1] = [0];

pub fn key((slot, id): Identity) -> u128 {
    u128::from(slot) << 32 | u128::from(id)
}

pub fn of(key: u128) -> Identity {
    ((key >> 32) as u32, key as u32)
}

#[derive(Default)]
pub struct Holders {
    links: HashMap<u32, u32>,
    slots: HashMap<u32, Option<u32>>,
}

impl Holders {
    pub fn read(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        self.slots.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(raw), Some(canonical)) = (links.key(slot), links.number(LINK_SLOT[0], slot)) else { continue };
            self.links.insert(raw as u32, canonical as u32);
        }
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            let (Some(raw), Some(canonical)) = (countries.key(slot), countries.number(COUNTRY_SLOT[0], slot)) else { continue };
            let (raw, canonical) = (raw as u32, canonical as u32);
            if self.links.get(&raw) != Some(&canonical) {
                continue;
            }
            self.slots.entry(canonical).and_modify(|held| *held = None).or_insert(Some(raw));
        }
    }

    pub fn identity(&self, raw: u32) -> Option<Identity> {
        if raw == 0 {
            return None;
        }
        let slot = *self.links.get(&raw)?;
        Some((slot, (*self.slots.get(&slot)?)?))
    }

    pub fn moved(hour: &Hour<'_>) -> bool {
        hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT)) || hour.wrote(TableId::Countries, Columns::These(&COUNTRY_SLOT))
    }
}
