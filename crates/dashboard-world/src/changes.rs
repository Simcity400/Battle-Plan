//! The rows an hour changed in each table a lane asked to be told about ([`super::Needs::changes`]):
//! rows written or filled, with their slot and the fields written, rows inserted and keys
//! deleted. Read once an hour for every lane of the walk, so an operator visits, and reads, only
//! what changed. At a restart's first hour these are the hour's own edits to the state the hour
//! before left; at a run's first hour every row is inserted.

use replay::{columns, segments::State};
use replay_data::rows::TableId;
use std::io;

/// What an hour changed of one table.
#[derive(Debug, Default)]
pub struct Changes {
    keys: Vec<u128>,
    written: Vec<(u128, u32, u64)>,
    inserted: Vec<(u128, u32)>,
    deleted: Vec<u128>,
}

impl Changes {
    /// The keys of the rows the hour wrote, filled, inserted or deleted, ascending.
    pub fn keys(&self) -> &[u128] {
        &self.keys
    }

    /// The rows the hour wrote or filled and kept: key, slot and the fields written, by key.
    pub fn written(&self) -> &[(u128, u32, u64)] {
        &self.written
    }

    /// The rows the hour inserted: key and slot, by key.
    pub fn inserted(&self) -> &[(u128, u32)] {
        &self.inserted
    }

    /// The keys the hour deleted, ascending.
    pub fn deleted(&self) -> &[u128] {
        &self.deleted
    }

    /// Whether the hour inserted or deleted rows.
    pub fn rows(&self) -> bool {
        !self.inserted.is_empty() || !self.deleted.is_empty()
    }

    pub(crate) fn read(&mut self, state: &State, table: TableId) -> io::Result<()> {
        self.keys.clear();
        self.written.clear();
        self.inserted.clear();
        self.deleted.clear();
        let held = columns::table(state, table)?;
        let changes = columns::changes(state, table)?;
        engine_allocation::reserve(&mut self.inserted, changes.inserted.len())?;
        for &slot in &changes.inserted {
            if let Some(key) = held.key(slot) {
                self.inserted.push((key, slot));
            }
        }
        self.inserted.sort_unstable();
        engine_allocation::reserve(
            &mut self.written,
            changes.changed.len() + changes.fill_slots.len(),
        )?;
        for (&slot, &mask) in changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        {
            // An inserted row is read whole.
            if let Some(key) = held.key(slot).filter(|key| {
                self.inserted
                    .binary_search_by_key(key, |&(inserted, _)| inserted)
                    .is_err()
            }) {
                self.written.push((key, slot, mask));
            }
        }
        self.written.sort_unstable_by_key(|&(key, ..)| key);
        // A row both written and filled is one, with both masks.
        self.written.dedup_by(|next, kept| {
            let same = next.0 == kept.0;
            if same {
                kept.2 |= next.2;
            }
            same
        });
        engine_allocation::reserve(&mut self.deleted, changes.deleted_keys.len())?;
        self.deleted.extend_from_slice(&changes.deleted_keys);
        self.deleted.sort_unstable();
        engine_allocation::reserve(
            &mut self.keys,
            self.written.len() + self.inserted.len() + self.deleted.len(),
        )?;
        self.keys.extend(self.written.iter().map(|&(key, ..)| key));
        self.keys.extend(self.inserted.iter().map(|&(key, _)| key));
        self.keys.extend_from_slice(&self.deleted);
        self.keys.sort_unstable();
        self.keys.dedup();
        Ok(())
    }
}
