//! Recorded country joins and sparse update keys shared by force censuses.

use crate::{metadata::tagged_countries, products::country_history::Identity};
use replay::{
    frame::{DecodedFrame, TableUpdate, ViewIdentity},
    rows::{TableId, tables},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

type TaggedIdentity = (Vec<u8>, Option<Identity>);

#[derive(Default)]
pub(super) struct Countries {
    from: Option<(ViewIdentity, ViewIdentity)>,
    pub tags: Vec<TaggedIdentity>,
    owners: HashMap<u32, u32>,
}

impl Countries {
    pub fn update(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        let countries = frame.complete::<tables::Countries>()?;
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
        let tagged = tagged_countries(countries, links);
        let present: HashSet<_> = tagged
            .values()
            .filter_map(|lookup| lookup.found().map(|(_, slot)| slot))
            .collect();
        let tags: Vec<_> = tagged
            .into_iter()
            .map(|(tag, lookup)| {
                (
                    tag.to_vec(),
                    lookup.found().map(|(country, slot)| (slot, country.id)),
                )
            })
            .collect();
        let owners: HashMap<_, _> = links
            .iter()
            .filter_map(|link| {
                let slot = link.canonical_slot;
                present.contains(&slot).then_some((link.id, slot))
            })
            .collect();
        let changed = self.tags != tags || self.owners != owners;
        self.tags = tags;
        self.owners = owners;
        self.from = Some((views.0.identity(), views.1.identity()));
        Ok(changed)
    }

    pub fn owner(&self, raw: u32) -> Option<u32> {
        self.owners.get(&raw).copied()
    }
}

/// Only use sparse keys after complete reconstruction. Snapshots and missing
/// evidence require a full rebuild; deltas include removals as well as changes.
pub(super) fn changed_ids(update: &TableUpdate, shift: u32, ids: &mut Vec<u64>) -> bool {
    match update {
        TableUpdate::Unchanged { .. } => true,
        TableUpdate::Delta {
            changed, deleted, ..
        } => {
            let mut changed = changed
                .iter()
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
