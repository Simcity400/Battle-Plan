//! Naval icon admission and forward-group census, shared by retained history and map bindings.

use crate::{
    engaged::Engaged,
    icon_frames::{self, IconHour, IconInput},
    map::tables::rows,
};
use replay_data::{rows::NavalCombatDetailsRow, rows::NavalCombatShipRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

/// The naval combat icons' hour for the engaged index ([`icon_frames`]): `alive`
/// comes from [`admitted`], `seen` from the combat's naval engaged `flag`, and the
/// frame from the convoy variant over the forward-group census ([`censuses`]).
/// The game writes `bg_btn`'s frame only while engaged and otherwise retains it.
pub fn naval_icon_hour(
    frame: &DecodedFrame,
    flag: &dyn Fn(u64) -> Engaged,
) -> io::Result<IconHour> {
    let combats = rows::<tables::NavalCombats>(frame)?;
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let details: BTreeMap<u64, &NavalCombatDetailsRow> =
        details.rows().iter().map(|row| (row.combat, row)).collect();
    let members = rows::<tables::NavalCombatShips>(frame)?;
    let censuses = members.is_complete().then(|| censuses(members.rows()));
    Ok(IconHour {
        combats: combats
            .rows()
            .iter()
            .map(|combat| {
                let variant = censuses.as_ref().and_then(|censuses| {
                    match censuses.get(&combat.id) {
                        Some(Ok(census)) => Some(*census),
                        Some(Err(_)) => None,
                        // A combat with no forward member has empty sides.
                        None => Some(Default::default()),
                    }
                    .map(|census| u32::from(convoy_variant(census)))
                });
                let input = IconInput {
                    alive: admitted(details.get(&combat.id).copied()),
                    seen: icon_frames::engaged_seen(flag(combat.id)),
                    frame: variant,
                };
                (combat.id, input)
            })
            .collect(),
        complete: combats.is_complete(),
    })
}

/// Drop an active icon when either side's primary country list is empty;
/// an absent detail row cannot decide it.
pub fn admitted(detail: Option<&NavalCombatDetailsRow>) -> Option<bool> {
    detail.map(|detail| {
        detail
            .sides
            .iter()
            .all(|side| !side.summary.countries.is_empty())
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub other: u32,
    pub submarines: u32,
    pub convoys: u32,
}

/// Count live convoys and live ships in each side's current forward groups.
/// Ships are submarines when formation bit 7 is set without capital bit 6 or
/// screen bit 8; all others use the other category. A combat has two sides. A
/// member recording a third leaves its combat's census undecided, with a named cause.
pub fn censuses(members: &[NavalCombatShipRow]) -> BTreeMap<u64, Result<[Census; 2], String>> {
    let mut out: BTreeMap<u64, Result<[Census; 2], String>> = BTreeMap::new();
    for member in members {
        if member.member_source != 0 || member.group_index == u32::MAX {
            continue;
        }
        let census = out.entry(member.combat).or_insert(Ok(Default::default()));
        let Ok(sides) = census else {
            continue;
        };
        let Some(side) = sides.get_mut(usize::from(member.side)) else {
            *census = Err(format!(
                "participant ship {} records side {}",
                member.ship, member.side
            ));
            continue;
        };
        if member.live_convoy != 0 {
            side.convoys += 1;
        }
        if member.live_ship != 0 {
            let flags = member.formation_flags;
            if flags & (1 << 7) != 0 && flags & ((1 << 6) | (1 << 8)) == 0 {
                side.submarines += 1;
            } else {
                side.other += 1;
            }
        }
    }
    out
}

/// the convoy variant of the combat sprite.
pub fn convoy_variant([a, b]: [Census; 2]) -> bool {
    (a.convoys > 0 && (a.other == 0 || (b.other == 0 && b.submarines > 0)))
        || (b.convoys > 0 && (b.other == 0 || (a.other == 0 && a.submarines > 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_convoy_variant_needs_convoys_against_no_surface_ships() {
        let side = |other, submarines, convoys| Census {
            other,
            submarines,
            convoys,
        };
        assert!(convoy_variant([side(0, 0, 3), side(2, 0, 0)]));
        assert!(convoy_variant([side(1, 0, 3), side(0, 2, 0)]));
        assert!(!convoy_variant([side(1, 0, 3), side(1, 0, 0)]));
        assert!(!convoy_variant([side(0, 0, 0), side(0, 2, 0)]));
    }

    #[test]
    fn only_current_forward_members_enter_the_census() {
        let member = |group_index, member_source, live_ship, live_convoy, formation_flags| {
            NavalCombatShipRow {
                combat: 1,
                group_index,
                member_source,
                live_ship,
                live_convoy,
                formation_flags,
                ..Default::default()
            }
        };
        let rows = [
            member(0, 0, 1, 0, 1 << 7),
            member(0, 0, 1, 0, (1 << 7) | (1 << 6)),
            member(0, 0, 0, 1, 0),
            member(u32::MAX, 0, 1, 0, 0),
            member(0, 1, 1, 0, 0),
        ];
        assert_eq!(
            censuses(&rows)[&1].as_ref().unwrap()[0],
            Census {
                other: 1,
                submarines: 1,
                convoys: 1
            }
        );
        // A forward member recording a third side leaves its combat's census undecided, named
        // by ship and side (main's census named the same cause).
        let third = NavalCombatShipRow {
            ship: 44,
            side: 2,
            ..member(0, 0, 1, 0, 0)
        };
        assert_eq!(
            censuses(&[rows[0].clone(), third])[&1],
            Err("participant ship 44 records side 2".to_owned())
        );
    }
}
