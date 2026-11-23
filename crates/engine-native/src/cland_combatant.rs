
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a2_36a0);
pub const FLAGS: usize = 0x08;
pub const COMBAT: usize = 0x18;
pub const UNITS: usize = 0x20;
#[cfg(any(test, feature = "test-support"))]
pub const UNIT_COUNT: usize = 0x2c;
pub const COUNTRIES: usize = 0x38;
pub const SECONDARY_COUNTRIES: usize = 0x50;

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
    pub const fn contains_country(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

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
pub const IS_ATTACKER: usize = 0xd8;
pub const COMMITTED: usize = 0xe8;
pub const RESERVES: usize = 0x100;
pub const TACTIC: usize = 0x130;
pub const AIR_SUPPORT: usize = 0x140;
pub const BORDER_MODIFIER: usize = 0x4f8;
pub const BORDER_DIG_IN_FACTOR: usize = 0x560;
pub const BORDER_TERRAIN_FACTOR: usize = 0x568;
pub const ANTI_AIR_ATTACK: usize = 0x158;
pub const AIR_KILLS: usize = 0x160;
pub const AIR_STRENGTH_DAMAGE: usize = 0x168;
pub const ORGANISATION_LOSS_WINDOW: usize = 0x198;
pub const ORGANISATION_LOSS_COUNT: usize = 0x1cc;
pub const STATS_OBSERVER: usize = 0x1d0;
pub const PROGRESS: usize = 0x438;
pub const OUTCOME_FLAGS: usize = 0x4b8;
pub const AIR_FLAGS: u32 = (1 << 14) | (1 << 15);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccountry::synthetic::World;

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
    fn membership_walks_a_side_to_its_last_node() {
        let (mut world, side) = world(5);
        let module = world.module;
        let check = |world: &World, raw| {
            capture_country_membership(&world.memory(), module, world.game_state(), side, raw)
        };
        assert_eq!(check(&world, 5), Ok(CountryMembership::Exact { raw: 5 }));
        assert_eq!(check(&world, 1), Ok(CountryMembership::Exact { raw: 1 }));
        let nodes = crate::read::<u64>(&world.memory(), side + COUNTRIES).unwrap() as usize;
        world.u32(nodes + 4 * 0x20, 0);
        assert_eq!(check(&world, 5), Ok(CountryMembership::Absent));
        world.u64(nodes + 4 * 0x20 + 0x10, nodes as u64);
        assert_eq!(
            check(&world, 5),
            Err("combat country set exceeds the country table")
        );
    }
}
