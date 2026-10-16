//! Direct-war cells used by country relations and the full-visibility war veto, derived
//! at playback from the hour's complete WarRelations with no memory access.
//!
//! Both read the receiver's diplomacy matrix (`country+0xF88`, `diplomacy+0x8`) at the
//! target's canonical slot; the cell is active when its war pointer (`relation
//! status+0x2E8`) is non-null and the pointed relation's retired byte (`relation+0x49`)
//! is zero. Both are functions of the recorded war relations:
//!
//! - The retired byte stays zero. Construction clears `+0x48`/`+0x49` as
//!   one word; loading reads only `+0x48`, and no other store in the image
//!   reaches a relation's `+0x49`.
//! - The war pointer is written only by status construction and reset (both
//!   null), and relation replacement. Only the war-link replacement helper
//!   forms the slot's address; linking installs the war in both endpoints'
//!   cells toward each other, and unlinking nulls both. Declarations, joins
//!   and civil-war transfers create war relations; loading initializes them
//!   unless duplicated. All install through linking. Peace, capitulation,
//!   annexation and resets end or reset war relations through unlinking.
//! - Relation replacement adds the installed war to the cell owner's owned
//!   relations (diplomacy `+0x20`) and removes a replaced one. That vector's
//!   only other writers are the other relation-slot setters, each removing
//!   only its own slot's previous relation, and destruction.
//!
//! So at the hourly boundary every non-null cell holds a war in its owner's owned vector
//! that joins the owner and the cell's target. The WarRelations walk
//! (`engine::cwar_relation`) visits every positive country's owned vector, fails when a
//! war endpoint does not resolve to a positive canonical slot or either endpoint cell
//! does not hold the war, and emits each war once. A complete WarRelations therefore
//! names exactly the active cells between positive countries, and no positive receiver's
//! cell toward slot zero holds a war. The country enemy vector (`diplomacy+0x98`) is not
//! equivalent: the civil-war transfer installs the revolter's wars without rebuilding its
//! enemy vector.

use crate::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, WarRelationRow, tables},
};

type Fact<T> = Result<T, String>;

/// One positive receiver's direct-war cells at one hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiverWars {
    receiver_slot: u32,
    /// Sorted unique canonical targets whose cell holds an active war.
    targets: Vec<u32>,
}

impl ReceiverWars {
    /// From the frame's complete WarRelations and CountryLinks. WarRelations
    /// is hourly, so a complete table was read at the frame's hour.
    pub fn read(frame: &DecodedFrame, receiver_slot: u32) -> Fact<Self> {
        let wars = frame
            .complete::<tables::WarRelations>()
            .map_err(|error| error.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|error| error.to_string())?;
        Self::from_rows(wars, links, receiver_slot)
    }

    /// From recorded rows, `links` in ascending raw-id order. Every raw
    /// reference CountryLinks files under the receiver's slot is the receiver.
    pub fn from_rows(
        wars: &[WarRelationRow],
        links: &[CountryLinkRow],
        receiver_slot: u32,
    ) -> Fact<Self> {
        if receiver_slot == 0 {
            return Err("derived direct wars need a positive receiver".into());
        }
        if !links.windows(2).all(|pair| pair[0].id < pair[1].id) {
            return Err("CountryLinks rows are not in ascending raw-id order".into());
        }
        let slot = |raw: u32| {
            links
                .binary_search_by_key(&raw, |row| row.id)
                .map(|index| links[index].canonical_slot)
                .map_err(|_| format!("WarRelations names country {raw}, which CountryLinks lacks"))
        };
        let receiver: Vec<u32> = links
            .iter()
            .filter(|row| row.canonical_slot == receiver_slot)
            .map(|row| row.id)
            .collect();
        let mut targets = Vec::new();
        for row in wars {
            let other = match (
                receiver.contains(&row.first_country),
                receiver.contains(&row.second_country),
            ) {
                (true, false) => row.second_country,
                (false, true) => row.first_country,
                (false, false) => continue,
                (true, true) => {
                    return Err(format!(
                        "WarRelations joins country slot {receiver_slot} to itself"
                    ));
                }
            };
            // The walk refuses an endpoint off the positive slots, so a
            // complete table never holds one.
            match slot(other)? {
                0 => {
                    return Err(format!(
                        "WarRelations links country {other} to the reserved slot"
                    ));
                }
                target => targets.push(target),
            }
        }
        targets.sort_unstable();
        targets.dedup();
        Ok(Self {
            receiver_slot,
            targets,
        })
    }

    /// The canonical targets whose cell holds an active war.
    pub fn targets(&self) -> &[u32] {
        &self.targets
    }

    /// `+0x2E8` non-null with the pointed `+0x49` zero, without the country-relation
    /// query's self and positive-receiver gates. The receiver's own cell and its cell
    /// toward slot zero never hold a war.
    pub fn pointer_active(&self, target_slot: u32) -> bool {
        self.targets.binary_search(&target_slot).is_ok()
    }
}

#[cfg(test)]
mod tests;
