use crate::{
    deployed::{LINK, invalid, number},
    framework::{Columns, Hour},
};
use replay_data::rows::{TableId, TagHolderRow, fits_key_field};
use std::{collections::HashMap, io};

const NONE: u32 = TagHolderRow::NONE;

#[derive(Default)]
pub struct Holders {
    links: HashMap<u32, u32>,
    holders: HashMap<u32, u32>,
}

impl Holders {
    pub fn read(hour: &Hour<'_>) -> Self {
        let mut found = Self::default();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            if let (Some(key), Some(canonical)) = (links.key(slot), number(&links, LINK[0], slot)) {
                found.links.insert(key as u32, canonical as u32);
            }
        }
        let holders = hour.table(TableId::TagHolders);
        for key in holders.rows().filter_map(|slot| holders.key(slot)) {
            let (number, slot, id) = ((key >> 96) as u32, (key >> 32) as u32, key as u32);
            if number == TagHolderRow::UNKNOWN || slot == NONE {
                continue;
            }
            found.holders.entry(slot).and_modify(|held| *held = (*held).min(id)).or_insert(id);
        }
        found
    }

    pub fn changed(hour: &Hour<'_>) -> bool {
        hour.wrote(TableId::CountryLinks, Columns::These(&LINK)) || hour.changed(TableId::TagHolders)
    }

    pub fn of(&self, raw: u32) -> Option<(u32, u32)> {
        if raw == 0 {
            return None;
        }
        let slot = *self.links.get(&raw)?;
        Some((slot, *self.holders.get(&slot)?))
    }
}

pub fn sides(holders: &Holders, raw: u32) -> io::Result<(u32, u32)> {
    let (slot, id) = holders.of(raw).unwrap_or((NONE, NONE));
    if !fits_key_field(slot) || !fits_key_field(id) {
        return Err(invalid(format!("Country slot {slot} or id {id} is past the 24 bits a derived table's key holds")));
    }
    Ok((slot, id))
}
