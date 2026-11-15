//! A country identity as the writers keyed by one hold it: (canonical slot, raw id of its
//! Countries row), that row's link resolving to its own canonical slot. A raw id names the
//! identity of the slot its link resolves to; a slot no resolving country holds, or two do, names
//! none.

use crate::framework::{Columns, Hour};
use replay_data::rows::TableId;
use std::collections::HashMap;

pub(crate) type Identity = (u32, u32);

/// Countries' and CountryLinks' canonical slot columns.
pub(crate) const COUNTRY_SLOT: [usize; 1] = [0];
pub(crate) const LINK_SLOT: [usize; 1] = [0];

/// The row key of an identity: its slot above its raw id.
pub(crate) fn key((slot, id): Identity) -> u128 {
    u128::from(slot) << 32 | u128::from(id)
}

pub(crate) fn of(key: u128) -> Identity {
    ((key >> 32) as u32, key as u32)
}

#[derive(Default)]
pub(crate) struct Holders {
    links: HashMap<u32, u32>,
    /// Each slot's resolving raw id; none where two resolve to it.
    slots: HashMap<u32, Option<u32>>,
}

impl Holders {
    /// The links and countries as the tables stand.
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

    /// The identity raw country `raw` names: none for the null id, an id no link resolves, or a
    /// slot held by no resolving country or by two.
    pub fn identity(&self, raw: u32) -> Option<Identity> {
        if raw == 0 {
            return None;
        }
        let slot = *self.links.get(&raw)?;
        Some((slot, (*self.slots.get(&slot)?)?))
    }

    /// Whether the hour moved a link's or a country's slot, or a country or link came or went.
    pub fn moved(hour: &Hour<'_>) -> bool {
        hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT)) || hour.wrote(TableId::Countries, Columns::These(&COUNTRY_SLOT))
    }
}
