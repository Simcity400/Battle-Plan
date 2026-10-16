//! CLandCombatant layout. Cached field lifetime and consistency at the
//! hourly boundary remain unproven. Reserve rebuild can rewrite contents
//! without changing the final header; rereading a header does not establish
//! an exhaustive writer or thread lifetime.

/// CLandCombatant primary vtable.
pub const VTABLE_RVA: usize = 0x0299_13c0;
/// CLandCombatant u32 indicator flags.
pub const FLAGS: usize = 0x08;
/// CLandCombatant parent CLandCombat pointer.
pub const COMBAT: usize = 0x18;
/// CLandCombatant full unit pointer array.
pub const UNITS: usize = 0x20;
/// CLandCombatant full unit-array u32 capacity.
pub const UNIT_CAPACITY: usize = 0x28;
/// CLandCombatant full unit-array u32 count.
pub const UNIT_COUNT: usize = 0x2c;
/// Primary country-tag linked set: head, tail and signed count at +0x10.
/// Nodes hold tag, previous +8 and next +0x10. Used for membership and shared
/// vision, separately from the unit list.
pub const COUNTRIES: usize = 0x38;
/// Secondary country-tag set, with the same layout. It selects the
/// viewer directly or shared vision for defender markers.
pub const SECONDARY_COUNTRIES: usize = 0x50;

/// Owned terminal witness for one country-membership query. A match
/// consumes only through its matching tag and does not establish readability
/// of later nodes or next pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountryMembership {
    Absent,
    Exact {
        raw: u32,
    },
    Alias {
        raw: u32,
        member: u32,
        canonical_slot: u32,
    },
}

impl CountryMembership {
    /// Project the acquired membership without live country resolution.
    pub const fn contains_country(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// Acquire membership lazily: exact or alias decisions select each next
/// node, so retain those decisions without copying the whole set. Walk to a
/// null next pointer, bounded by live country count plus one for tag zero.
/// Insertion excludes equal or equivalent tags. Unknown pointers or longer
/// cyclic lists fail instead of implying defender membership.
pub fn capture_country_membership(
    memory: &impl crate::Memory,
    base: usize,
    state: usize,
    side: usize,
    raw: u32,
) -> Result<CountryMembership, &'static str> {
    use crate::{ccountry, read};
    let countries = ccountry::Reader::new(memory, base, state)?;
    let canonical = if raw == 0 {
        None
    } else {
        Some(countries.resolve(raw)?.canonical_slot)
    };
    let mut node = side
        .checked_add(COUNTRIES)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("combat country set unreadable")? as usize;
    for _ in 0..=countries.count() {
        if node == 0 {
            return Ok(CountryMembership::Absent);
        }
        let tag = read::<u32>(memory, node).ok_or("combat country tag unreadable")?;
        if tag == raw {
            return Ok(CountryMembership::Exact { raw });
        }
        if tag != 0
            && let Some(slot) = canonical
            && countries
                .resolve(tag)
                .is_ok_and(|c| c.canonical_slot == slot)
        {
            return Ok(CountryMembership::Alias {
                raw,
                member: tag,
                canonical_slot: slot,
            });
        }
        // Do not hide a failed alias lookup in the inequality branch.
        if tag != 0 {
            countries.resolve(tag)?;
        }
        node = node
            .checked_add(0x10)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("combat country next unreadable")? as usize;
    }
    Err("combat country set exceeds the country table")
}

/// Direct-reader entry point sharing the branch-lazy owned acquisition.
#[cfg(test)]
pub fn contains_country(
    memory: &impl crate::Memory,
    base: usize,
    state: usize,
    side: usize,
    raw: u32,
) -> Result<bool, &'static str> {
    Ok(capture_country_membership(memory, base, state, side, raw)?.contains_country())
}
/// CLandCombatant attacker discriminator byte.
pub const IS_ATTACKER: usize = 0xb8;
/// CLandCombatant committed unit pointer array.
pub const COMMITTED: usize = 0xc8;
/// CLandCombatant committed-array u32 capacity.
pub const COMMITTED_CAPACITY: usize = 0xd0;
/// CLandCombatant committed-array u32 count.
pub const COMMITTED_COUNT: usize = 0xd4;
/// CLandCombatant reserve unit pointer array.
pub const RESERVES: usize = 0xe0;
/// CLandCombatant reserve-array u32 capacity.
pub const RESERVE_CAPACITY: usize = 0xe8;
/// CLandCombatant reserve-array u32 count.
pub const RESERVE_COUNT: usize = 0xec;
/// CLandCombatant CCombatTactic pointer.
pub const TACTIC: usize = 0x110;
/// Air-support pointer array; display sums each entry signed i32 at +0x14.
pub const AIR_SUPPORT: usize = 0x120;
/// Signed fixed5 border modifier; the tooltip tests it for presence.
pub const BORDER_MODIFIER: usize = 0x4d8;
/// Border-conflict dig-in factor.
pub const BORDER_DIG_IN_FACTOR: usize = 0x540;
/// Border-conflict terrain factor.
pub const BORDER_TERRAIN_FACTOR: usize = 0x548;
/// Participant-vector rebuild latch.
pub const REBUILD_LATCH: usize = 0x118;
/// CLandCombatant anti-air attack i64 /100000.
pub const ANTI_AIR_ATTACK: usize = 0x138;
/// CLandCombatant air-kill u32 count.
pub const AIR_KILLS: usize = 0x140;
/// CLandCombatant air strength damage i64 /100000.
pub const AIR_STRENGTH_DAMAGE: usize = 0x148;
/// CLandCombatant air organisation damage i64 /100000.
pub const AIR_ORGANISATION_DAMAGE: usize = 0x150;
/// CLandCombatant ground strength damage i64 /100000.
pub const GROUND_STRENGTH_DAMAGE: usize = 0x158;
/// CLandCombatant ground organisation damage i64 /100000.
pub const GROUND_ORGANISATION_DAMAGE: usize = 0x160;
/// CLandCombatant prevented strength damage i64 /100000.
pub const PREVENTED_STRENGTH_DAMAGE: usize = 0x168;
/// CLandCombatant prevented organisation damage i64 /100000.
pub const PREVENTED_ORGANISATION_DAMAGE: usize = 0x170;
/// CLandCombatant pointer to i64 /100000 organisation-loss samples.
pub const ORGANISATION_LOSS_WINDOW: usize = 0x178;
/// Engine land-indicator comparison averages 24 loss samples.
pub const ORGANISATION_LOSS_WINDOW_LENGTH: usize = 24;
/// CLandCombatant organisation-loss u32 summary index.
pub const ORGANISATION_LOSS_INDEX: usize = 0x1a8;
/// CLandCombatant organisation-loss u32 sample count.
pub const ORGANISATION_LOSS_COUNT: usize = 0x1ac;
/// Embedded combat statistics observer.
pub const STATS_OBSERVER: usize = 0x1b0;
/// Signed fixed5 observer progress, saved, loaded and compared as a qword.
pub const PROGRESS: usize = 0x418;
/// Embedded observer outcome u8 flags. The setter adds bit 1;
/// load and save retain it, and finalization reads the same flags.
pub const OUTCOME_FLAGS: usize = 0x498;
/// CLandCombatant unit vectors contain eight-byte CArmy pointers.
pub const UNIT_POINTER_STRIDE: usize = 8;
/// CLandCombatant round vtable slot.
pub const ROUND_VTABLE_SLOT: usize = 0x98;
/// Combat-side round entry.
pub const ROUND_RVA: usize = 0x012a_3220;
/// Exact combat-side round entry bytes.
pub const ROUND_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54,
];
/// Combat-side pair resolver.
pub const PAIR_RESOLVER_RVA: usize = 0x0129_e240;
/// Combat-side damage application.
pub const DAMAGE_APPLY_RVA: usize = 0x012a_4be0;
/// Reserve and committed participant rebuild.
pub const REBUILD_UNITS_RVA: usize = 0x0129_7390;
/// First direct participant-rebuild caller.
pub const REBUILD_CALLER_RVA: usize = 0x012a_3ef0;
/// Second direct participant-rebuild caller.
pub const REBUILD_SECOND_CALLER_RVA: usize = 0x012a_4bb0;
/// Combat-side organisation-sum getter.
pub const ORGANISATION_SUM_RVA: usize = 0x012a_02a0;
/// CLandCombatant organisation-sum vtable slot.
pub const ORGANISATION_SUM_VTABLE_SLOT: usize = 0x60;
/// CLandCombatant comparison getter vtable slot.
pub const COMPARISON_VTABLE_SLOT: usize = 0x68;
/// Combat-side comparison getter selects by loss-sample count.
pub const COMPARISON_RVA: usize = 0x012a_2aa0;
/// Committed-unit strength model.
pub const STRENGTH_RVA: usize = 0x012a_26b0;
/// Parent-combat getter returns the stored backlink.
pub const COMBAT_GETTER_RVA: usize = 0x001f_6a80;
/// CLandCombatant flag bits 11/12 read for bombardment.
pub const BOMBARDMENT_FLAGS: u32 = (1 << 11) | (1 << 12);
/// CLandCombatant flag bits 14/15 read for air frames.
pub const AIR_FLAGS: u32 = (1 << 14) | (1 << 15);
/// Combat participant lifecycle entry.
pub const PARTICIPANT_LIFECYCLE_RVA: usize = 0x0129_9850;
/// CLandCombatant lifecycle pointer slot, vtable entry 33.
pub const PARTICIPANT_LIFECYCLE_SLOT_RVA: usize = 0x0299_14c8;
/// CLandCombatant authenticated vtable entry count.
pub const VTABLE_ENTRY_COUNT: usize = 34;
/// CLandCombatant participant lifecycle vtable entry index.
pub const PARTICIPANT_LIFECYCLE_SLOT_INDEX: usize = 33;
/// Participant lifecycle unwind metadata.
pub const PARTICIPANT_LIFECYCLE_UNWIND_RVA: usize = 0x0367_0e14;
/// Exact participant-lifecycle entry bytes.
pub const PARTICIPANT_LIFECYCLE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Combat-width producer returning signed i64.
pub const WIDTH_PRODUCER_RVA: usize = 0x0129_9610;
/// Exact combat-width producer entry bytes.
pub const WIDTH_PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// Combat-side leader resolver; panel-time acquisition is excluded.
pub const LEADER_RESOLVER_RVA: usize = 0x0129_e190;
/// Exact leader-resolver entry bytes.
pub const LEADER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x33, 0xff,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccountry::synthetic::World;

    /// A side whose country list holds every country 1..=n in order.
    fn world(countries: u32) -> (World, usize) {
        let mut world = World::new(0x4000_0000, countries, 0x20);
        let side = world.alloc(0x100);
        let nodes = world.alloc(countries as usize * 0x20);
        world.u64(side + COUNTRIES, nodes as u64);
        for index in 0..countries as usize {
            let node = nodes + index * 0x20;
            world.u32(node, index as u32 + 1);
            if index + 1 < countries as usize {
                world.u64(node + 0x10, (node + 0x20) as u64);
            }
        }
        (world, side)
    }

    #[test]
    fn membership_walks_a_5000_country_side_to_its_last_node() {
        let (mut world, side) = world(5_000);
        let module = world.module;
        let check = |world: &World, raw| {
            capture_country_membership(&world.memory(), module, world.game_state(), side, raw)
        };
        assert_eq!(
            check(&world, 5_000),
            Ok(CountryMembership::Exact { raw: 5_000 })
        );
        assert_eq!(check(&world, 1), Ok(CountryMembership::Exact { raw: 1 }));
        // Every node resolves; an unlisted tag reports absence after all 5,000.
        let nodes = crate::read::<u64>(&world.memory(), side + COUNTRIES).unwrap() as usize;
        world.u32(nodes + 4_999 * 0x20, 0);
        assert_eq!(check(&world, 5_000), Ok(CountryMembership::Absent));
        // A cycle is still refused, bounded by the live country table.
        world.u64(nodes + 4_999 * 0x20 + 0x10, nodes as u64);
        assert_eq!(
            check(&world, 5_000),
            Err("combat country set exceeds the country table")
        );
    }
}
