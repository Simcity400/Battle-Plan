//! Naval combat sides for the engaged flag, fed to the
//! shared derivation ([`super::engaged`]) through the engaged index.
//!
//! A naval side's flag is rebuilt (`engagement refresh`) by:
//!
//! - `unit joining` when a task force joins, which appends
//!   its owners to the lists: the side's recorded task forces
//!   gain a handle, and the recorded lists are tested;
//! - `transfer joining` when a transfer's
//!   convoys join, which creates their members with fresh combat-wide battle
//!   slots and refreshes: a convoy member's slot the
//!   previous hour lacked proves a rebuild of the recorded lists, and an hour
//!   whose participant census is incomplete may hide one
//!   ([`engaged::Added::Unavailable`]);
//! - `combat unit removal`, the only naval removal, on both
//!   sides: each side's `side unit removal` rebuilds its lists from its
//!   remaining task forces' owners alone (logical owners into the primary list,
//!   positive raw roster owners into the roster-owner list), which are the lists that
//!   rebuild tests;
//! - a load, a player change and the local player's controller change.
//!
//! Known misses, beyond the shared ones in [`super::engaged`]: the convoy and
//! transfer owners a removal drops re-enter the lists afterwards without a
//! refresh, so a later hour's recorded lists can hold owners the
//! held flag never tested, until the next rebuild; a task force re-added
//! while present and a transfer joining without a new convoy member rebuild
//! without a recorded trace.

use super::engaged::{self, Removal};
use crate::map::tables::rows;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub struct NavalSides;

impl engaged::Sides for NavalSides {
    fn tables(&self) -> &'static [TableId] {
        &[
            TableId::NavalCombats,
            TableId::NavalCombatDetails,
            TableId::NavalCombatParticipants,
            TableId::TaskForces,
        ]
    }

    fn combats(&self, frame: &DecodedFrame) -> io::Result<Vec<u64>> {
        let mut combats: Vec<u64> = rows::<tables::NavalCombats>(frame)?
            .rows()
            .iter()
            .map(|combat| combat.id)
            .collect();
        combats.sort_unstable();
        combats.dedup();
        Ok(combats)
    }

    fn sides(&self, frame: &DecodedFrame) -> io::Result<engaged::HourSides> {
        naval_sides(frame)
    }
}

fn naval_sides(frame: &DecodedFrame) -> io::Result<engaged::HourSides> {
    let combats = rows::<tables::NavalCombats>(frame)?;
    let mut out = engaged::HourSides {
        sides: BTreeMap::new(),
        complete: combats.is_complete(),
        removal: Removal::BothSides,
    };
    // Most hours hold no naval combat: nothing below is read for them.
    if combats.rows().is_empty() {
        return Ok(out);
    }
    let details: BTreeMap<u64, _> = rows::<tables::NavalCombatDetails>(frame)?
        .rows()
        .iter()
        .map(|row| (row.combat, row))
        .collect();
    let participants = rows::<tables::NavalCombatParticipants>(frame)?;
    // Convoy members (no ship) by side, their battle slots sorted; only a
    // complete census can show a slot the previous hour lacked.
    let mut convoys: BTreeMap<(u64, u8), Vec<u64>> = BTreeMap::new();
    if participants.is_complete() {
        for member in participants.rows().iter().filter(|member| member.ship == 0) {
            convoys
                .entry((member.combat, member.side))
                .or_default()
                .push(u64::from(member.battle_slot));
        }
        for slots in convoys.values_mut() {
            slots.sort_unstable();
        }
    }
    // Only the task forces the sides name are read, each found in the census,
    // which is key-sorted by id.
    let forces = rows::<tables::TaskForces>(frame)?.rows();
    // lists from the remaining task forces alone; `None` when one
    // of them is not in the census.
    let unit_lists = |units: &[u64]| {
        let mut lists = (Vec::new(), Vec::new());
        for &unit in units {
            let force = &forces[forces.binary_search_by_key(&unit, |force| force.id).ok()?];
            lists.0.push(force.logical_owner);
            if (force.roster_owner as i32) > 0 {
                lists.1.push(force.roster_owner);
            }
        }
        Some(lists)
    };
    for combat in combats.rows() {
        for side in 0..2_u8 {
            let key = (combat.id, side);
            let recorded = details
                .get(&combat.id)
                .map(|detail| &detail.sides[usize::from(side)]);
            let units = recorded.map(|recorded| {
                let mut units = recorded.units.clone();
                units.sort_unstable();
                units
            });
            out.sides.insert(
                key,
                engaged::Side {
                    unit_lists: recorded.and_then(|recorded| unit_lists(&recorded.units)),
                    units,
                    added: if participants.is_complete() {
                        engaged::Added::Observed(convoys.get(&key).cloned().unwrap_or_default())
                    } else {
                        engaged::Added::Unavailable
                    },
                    lists: recorded.map(|recorded| {
                        (
                            recorded.summary.countries.clone(),
                            recorded.summary.secondary_countries.clone(),
                        )
                    }),
                },
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "naval_sides/tests.rs"]
mod tests;
