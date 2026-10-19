//! Recorded country joins and sparse update keys shared by force censuses.

use crate::{
    metadata::{CountryKeys, tagged_countries},
    products::country_history::Identity,
};
use replay_data::{rows::DeltaRow, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableUpdate, frame::ViewIdentity};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
};

type TaggedIdentity = (Vec<u8>, Option<Identity>);

#[derive(Default)]
pub struct Countries {
    from: Option<(ViewIdentity, ViewIdentity)>,
    at: Option<(replay_data::BranchId, replay_data::Hour)>,
    rows: usize,
    pub tags: Vec<TaggedIdentity>,
    owners: HashMap<u32, u32>,
    keys: Option<CountryKeys>,
    slots: HashMap<u32, Vec<usize>>,
    pub unreadable_tags: usize,
    tags_valid: bool,
}

impl Countries {
    /// Balance-only deltas preserve every country key. Exact row evidence,
    /// adjacency and unchanged links prove the existing join still applies;
    /// insertions, removals, snapshots and unavailable reads use the ordinary
    /// key comparison instead.
    pub fn update_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<bool> {
        static FIELDS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let fields = *FIELDS.get_or_init(|| {
            crate::prepared::frame_changes::field_mask(
                TableId::Countries,
                &["canonical_slot", "tag"],
            )
        });
        if self.at.is_some_and(|(branch, hour)| {
            branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
        }) && self
            .from
            .as_ref()
            .is_some_and(|(_, links)| links.is(&frame.tables[TableId::CountryLinks.index()]))
            && frame
                .complete::<tables::Countries>()
                .is_ok_and(|rows| rows.len() == self.rows)
            && changes
                .and_then(|changes| changes.rows(TableId::Countries))
                .is_some_and(|(changed, deleted)| {
                    deleted.is_empty() && changed.iter().all(|row| row.mask & fields == 0)
                })
        {
            self.from = Some((
                frame.tables[TableId::Countries.index()].identity(),
                frame.tables[TableId::CountryLinks.index()].identity(),
            ));
            self.at = Some((frame.branch, frame.hour));
            return Ok(false);
        }
        self.update(frame)
    }

    pub fn update(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        self.at = Some((frame.branch, frame.hour));
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
        self.rows = countries.len();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared::frame_changes::FrameChangeTracker, test_fixtures::scale_frame};
    use replay_data::{format::FormatVersion, rows::TableRows};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder, frame::TableView};
    use std::sync::Arc;

    #[test]
    fn balance_deltas_skip_country_key_walks_and_other_boundaries_keep_the_join() {
        let count = 5000;
        let mut input = scale_frame(1, 100, count);
        let mut encoder = FrameEncoder::default();
        let mut decoder = FrameDecoder::default();
        let mut tracker = FrameChangeTracker::new();
        let (mut sparse, mut whole) = (Countries::default(), Countries::default());
        for step in 0..10 {
            input.hour = 100 + step;
            if step == 6 {
                input.hour += 1;
            }
            if step >= 7 {
                input.hour += 1;
                input.branch = 2;
            }
            if step == 9 {
                input = scale_frame(2, 110, count);
            }
            if step == 8 {
                *input.tables[TableId::Countries.index()] = TableView::Unavailable {
                    reason: "synthetic missing census".into(),
                };
            } else if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            {
                for row in rows.iter_mut() {
                    row.political_power_raw = Some(i64::from(step));
                }
                match step {
                    2 => rows[0].tag = b"mod new tag".to_vec(),
                    3 => rows[0].canonical_slot = count,
                    4 => {
                        rows.pop();
                    }
                    _ => {}
                }
            }
            let raw = Arc::new(
                encoder
                    .encode(input.clone(), [0, 5, 7].contains(&step))
                    .unwrap(),
            );
            let frame = decoder.advance(&raw).unwrap();
            let changes = tracker.observe_updates(frame, Arc::clone(&raw), FormatVersion::CURRENT);
            let held_keys = sparse.keys.clone();
            let before = held_keys.as_ref().map_or(0, CountryKeys::row_visits);
            let a = sparse.update_changed(frame, Some(&changes));
            let b = whole.update(frame);
            assert_eq!(a.as_ref().ok(), b.as_ref().ok(), "step {step}");
            assert_eq!(a.is_err(), b.is_err());
            assert_eq!(sparse.tags, whole.tags);
            assert_eq!(sparse.owners, whole.owners);
            assert_eq!(sparse.slots, whole.slots);
            assert_eq!(sparse.tags_valid, whole.tags_valid);
            if step == 1 {
                assert_eq!(
                    held_keys.as_ref().unwrap().row_visits(),
                    before,
                    "balance-only deltas walk no keys"
                );
                assert!(whole.keys.as_ref().unwrap().row_visits() >= count as usize);
            }
        }
    }
}
