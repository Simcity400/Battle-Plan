
use super::engaged::{self, Removal};
use viewer_foundation::tables::rows;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, Tables};
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

pub fn naval_sides<F: Tables + ?Sized>(frame: &F) -> io::Result<engaged::HourSides> {
    use viewer_foundation::fold_tables::available as rows;
    let combats = rows::<tables::NavalCombats, F>(frame)?;
    let mut out = engaged::HourSides {
        sides: BTreeMap::new(),
        complete: combats.is_complete(),
        removal: Removal::BothSides,
    };
    if combats.rows().is_empty() {
        return Ok(out);
    }
    let details: BTreeMap<u64, _> = rows::<tables::NavalCombatDetails, F>(frame)?
        .rows()
        .iter()
        .map(|row| (row.combat, row))
        .collect();
    let participants = rows::<tables::NavalCombatParticipants, F>(frame)?;
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
    let forces = rows::<tables::TaskForces, F>(frame)?.rows();
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
