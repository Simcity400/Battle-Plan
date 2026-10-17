//! Recorded country joins and sparse update keys shared by force censuses.

use crate::{
    metadata::{CountryKeys, tagged_countries},
    products::country_history::Identity,
};
use replay::{
    frame::{DecodedFrame, TableUpdate, ViewIdentity},
    rows::{DeltaRow, TableId, tables},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
};

type TaggedIdentity = (Vec<u8>, Option<Identity>);

#[derive(Default)]
pub struct Countries {
    from: Option<(ViewIdentity, ViewIdentity)>,
    pub tags: Vec<TaggedIdentity>,
    owners: HashMap<u32, u32>,
    keys: Option<CountryKeys>,
    slots: HashMap<u32, Vec<usize>>,
    pub unreadable_tags: usize,
    tags_valid: bool,
}

impl Countries {
    pub fn update(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        let countries = match frame.complete::<tables::Countries>() {
            Ok(rows) => rows,
            Err(error) => {
                self.from = None;
                self.keys = None;
                self.tags.clear();
                self.owners.clear();
                self.slots.clear();
                self.unreadable_tags = 0;
                self.tags_valid = false;
                return Err(error);
            }
        };
        let links = frame.complete::<tables::CountryLinks>().unwrap_or_default();
        let views = (
            &frame.tables[TableId::Countries.index()],
            &frame.tables[TableId::CountryLinks.index()],
        );
        if self
            .from
            .as_ref()
            .is_some_and(|(countries, links)| countries.is(views.0) && links.is(views.1))
        {
            return Ok(false);
        }
        if self.from.as_ref().is_some_and(|(_, old)| old.is(views.1))
            && self.keys.as_ref().is_some_and(|keys| keys.same(countries))
        {
            self.from = Some((views.0.identity(), views.1.identity()));
            return Ok(false);
        }
        let tagged = tagged_countries(countries, links);
        let present: HashSet<_> = tagged
            .values()
            .filter_map(|lookup| lookup.found().map(|(_, slot)| slot))
            .collect();
        let mut tags: Vec<_> = tagged
            .into_iter()
            .map(|(tag, lookup)| {
                (
                    tag.to_vec(),
                    lookup.found().map(|(country, slot)| (slot, country.id)),
                )
            })
            .collect();
        tags.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let owners: HashMap<_, _> = links
            .iter()
            .filter_map(|link| {
                let slot = link.canonical_slot;
                present.contains(&slot).then_some((link.id, slot))
            })
            .collect();
        let changed = self.tags != tags || self.owners != owners;
        self.slots.clear();
        self.unreadable_tags = 0;
        for (index, (tag, identity)) in tags.iter().enumerate() {
            if let Some(identity) = identity {
                self.slots.entry(identity.0).or_default().push(index);
                self.unreadable_tags += usize::from(std::str::from_utf8(tag).is_err());
            }
        }
        self.tags_valid =
            self.unreadable_tags == 0 && tags.iter().all(|(_, identity)| identity.is_some());
        self.tags = tags;
        self.owners = owners;
        self.from = Some((views.0.identity(), views.1.identity()));
        self.keys = Some(CountryKeys::read(countries));
        Ok(changed)
    }

    pub fn tags_valid(&self) -> bool {
        self.tags_valid
    }
    pub fn contains_slot(&self, slot: u32) -> bool {
        self.slots.contains_key(&slot)
    }

    pub fn in_slots(&self, slots: &BTreeSet<u32>) -> Vec<TaggedIdentity> {
        slots
            .iter()
            .filter_map(|slot| self.slots.get(slot))
            .flatten()
            .map(|&index| self.tags[index].clone())
            .collect()
    }

    pub fn owner(&self, raw: u32) -> Option<u32> {
        self.owners.get(&raw).copied()
    }
}

/// Only use sparse keys after complete reconstruction. Snapshots and missing
/// evidence require a full rebuild; deltas include removals as well as changes.
/// `keep` selects changed fields; it must also keep every newly inserted row.
pub fn changed_ids(
    update: &TableUpdate,
    shift: u32,
    ids: &mut Vec<u64>,
    keep: impl Fn(&DeltaRow) -> bool,
) -> bool {
    match update {
        TableUpdate::Unchanged { .. } => true,
        TableUpdate::Delta {
            changed, deleted, ..
        } => {
            let mut changed = changed
                .iter()
                .filter(|row| keep(row))
                .map(|row| (row.key >> shift) as u64)
                .peekable();
            let mut deleted = deleted.iter().map(|key| (key >> shift) as u64).peekable();
            let mut incoming = Vec::new();
            while changed.peek().is_some() || deleted.peek().is_some() {
                let id = if deleted.peek().is_none()
                    || changed
                        .peek()
                        .zip(deleted.peek())
                        .is_some_and(|(a, b)| a <= b)
                {
                    changed.next().expect("remaining change")
                } else {
                    deleted.next().expect("remaining deletion")
                };
                if incoming.last() != Some(&id) {
                    incoming.push(id);
                }
            }
            let previous = std::mem::take(ids);
            let mut previous = previous.into_iter().peekable();
            let mut incoming = incoming.into_iter().peekable();
            while previous.peek().is_some() || incoming.peek().is_some() {
                let id = if incoming.peek().is_none()
                    || previous
                        .peek()
                        .zip(incoming.peek())
                        .is_some_and(|(a, b)| a <= b)
                {
                    previous.next().expect("remaining previous key")
                } else {
                    incoming.next().expect("remaining new key")
                };
                if ids.last() != Some(&id) {
                    ids.push(id);
                }
            }
            true
        }
        _ => false,
    }
}
