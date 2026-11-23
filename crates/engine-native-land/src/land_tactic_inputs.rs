
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native_parts::{ccombat_tactic, cnull_combat_tactic};
use engine_native::{ccountry, cland_combat};
use engine_sites::Rva;

pub const LEADER_PREFERRED_TACTIC: usize = 0x10b0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub country_weights: [ccombat_tactic::WeightRow; 2],
    pub country_weight_present: [u8; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextCapture {
    pub outcome: Outcome,
    pub row: ContextRow,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("tactic input unreadable")
}

fn pointer(memory: &impl Memory, parent: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, parent, offset).map(|value| value as usize)
}

fn array(
    memory: &impl Memory,
    parent: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    parent
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .ok_or("tactic input array invalid")
}

fn identity(memory: &impl Memory, base: usize, tactic: usize) -> Result<i32, &'static str> {
    let vtable = field::<u64>(memory, tactic, 0)?;
    if engine_sites::at(base, cnull_combat_tactic::VTABLE_RVA).map(|p| p as u64) == Some(vtable) {
        return Ok(0);
    }
    if engine_sites::at(base, ccombat_tactic::VTABLE_RVA).map(|p| p as u64) != Some(vtable) {
        return Err("tactic vtable mismatch");
    }
    let id = field(memory, tactic, ccombat_tactic::ID)?;
    if id < 1 {
        return Err("tactic id invalid");
    }
    Ok(id)
}

fn authentic(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rvas: &[Rva],
) -> Result<(), &'static str> {
    let vtable = field::<u64>(memory, object, 0)?;
    if engine_sites::which(base, vtable as usize, rvas).is_some() {
        Ok(())
    } else {
        Err("tactic context vtable mismatch")
    }
}

fn parent_commander(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    leader: usize,
) -> Result<usize, &'static str> {
    if leader == 0 {
        return Ok(0);
    }
    let owner = countries.resolve(field(memory, leader, 0x120)?)?;
    let theatres = array(memory, owner.pointer, 0x168, 8)?;
    for ordinal in 0..theatres.count {
        let theatre = pointer(memory, theatres.data, ordinal as usize * 8)?;
        authentic(memory, base, theatre, &[engine_native::ctheatre::VTABLE_RVA])?;
        let groups = array(memory, theatre, 0x80, 8)?;
        for index in 0..groups.count {
            let group = pointer(memory, groups.data, index as usize * 8)?;
            authentic(
                memory,
                base,
                group,
                &[
                    engine_native::corders_group::VTABLE_RVA,
                    engine_native_parts::carmy_group::VTABLE_RVA,
                ],
            )?;
            if pointer(memory, group, engine_native::corders_group::LEADER)? != leader {
                continue;
            }
            let parent = pointer(memory, group, engine_native::corders_group::PARENT)?;
            if parent == 0 {
                continue;
            }
            authentic(memory, base, parent, &[engine_native_parts::carmy_group::VTABLE_RVA])?;
            let result = pointer(memory, parent, engine_native::corders_group::LEADER)?;
            if result != 0 {
                authentic(memory, base, result, &[engine_native::carmy_leader::VTABLE_RVA])?;
            }
            return Ok(result);
        }
    }
    Ok(0)
}

pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    source: cland_combat::Source,
    members: &cland_combat::Members<'_>,
    selections: &mut crate::land_combat_details::Commanders,
) -> ContextCapture {
    capture_context_sharing(
        memory,
        base,
        game,
        source,
        members,
        selections,
        &mut crate::land_combat_details::Leaders::default(),
    )
}

pub fn capture_context_sharing(
    memory: &impl Memory,
    base: usize,
    game: usize,
    source: cland_combat::Source,
    members: &cland_combat::Members<'_>,
    selections: &mut crate::land_combat_details::Commanders,
    leaders: &mut crate::land_combat_details::Leaders,
) -> ContextCapture {
    let mut row = ContextRow::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game)?;
        let commanders =
            crate::land_combat_details::commander_sources(memory, base, source, members, selections)?;
        let sides = [
            pointer(memory, source.pointer, cland_combat::ATTACKER)?,
            pointer(memory, source.pointer, cland_combat::DEFENDER)?,
        ];
        let weight_context = ccombat_tactic::WeightContext::read(memory, base)?;
        for (index, side) in sides.into_iter().enumerate() {
            let first = pointer(memory, side, engine_native::cland_combatant::COUNTRIES)?;
            if first != 0 {
                let tag = field::<u32>(memory, first, 0)?;
                if tag == 0 {
                    return Err("tactic side country scope is null");
                }
                let country = countries.resolve(tag)?;
                let preferred =
                    ccombat_tactic::preferred_identity(memory, base, country.pointer, country.id);
                if preferred.outcome != (Outcome::Rows { len: 1 }) {
                    return Err("tactic country preference unavailable");
                }
                row.preferred_tactic_ids[index][0] = preferred.row.tactic_id;
                if let Some(weight) =
                    ccombat_tactic::weight(memory, country.pointer, &preferred, weight_context)?
                {
                    row.country_weights[index] = weight;
                    row.country_weight_present[index] = 1;
                }
            }
            let leader = commanders[index].pointer;
            let parent = crate::land_combat_details::Leaders::shared(&mut leaders.parents, leader, || {
                parent_commander(memory, base, &countries, leader)
            })?;
            for (position, leader) in [leader, parent].into_iter().enumerate() {
                if leader == 0 {
                    continue;
                }
                row.preferred_tactic_ids[index][position + 1] =
                    crate::land_combat_details::Leaders::shared(&mut leaders.preferred, leader, || {
                        identity(
                            memory,
                            base,
                            pointer(memory, leader, LEADER_PREFERRED_TACTIC)?,
                        )
                    })?;
            }
        }
        Ok(())
    })();
    ContextCapture {
        outcome: match result {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(()) => Outcome::Rows { len: 1 },
        },
        row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_commander_reaches_the_last_native_theatre_and_formation() {
        const COUNT: usize = 65;
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x4000;
        const LEADER: usize = 0x8000;
        const THEATRE: usize = 0x9000;
        const LAST_THEATRE: usize = 0x12000;
        const GROUP: usize = 0x15000;
        const LAST_GROUP: usize = 0x14000;
        const PARENT: usize = 0x11000;
        const PARENT_LEADER: usize = 0xd000;
        fn word(bytes: &mut [u8], at: usize, value: usize) {
            bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn dword(bytes: &mut [u8], at: usize, value: usize) {
            bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
        fn header(bytes: &mut [u8], at: usize, data: usize, count: usize) {
            word(bytes, at, data);
            dword(bytes, at + 8, count);
            dword(bytes, at + 12, count);
        }
        for theatres in [false, true] {
            let mut bytes = vec![0; 0x60000];
            header(&mut bytes, STATE + engine_native::cgame_state::COUNTRIES, 0x3000, 2);
            header(
                &mut bytes,
                STATE + engine_native::cgame_state::COUNTRY_LINKS,
                0x3100,
                2,
            );
            word(&mut bytes, 0x3008, COUNTRY);
            dword(&mut bytes, 0x3104, 1);
            word(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0);
            dword(&mut bytes, COUNTRY + ccountry::ID, 1);
            dword(&mut bytes, LEADER + 0x120, 1);
            for theatre in [THEATRE, LAST_THEATRE] {
                word(&mut bytes, theatre, engine_native::ctheatre::VTABLE_RVA.0);
            }
            for group in [GROUP, LAST_GROUP] {
                word(&mut bytes, group, engine_native::corders_group::VTABLE_RVA.0);
            }
            word(
                &mut bytes,
                LAST_GROUP + engine_native::corders_group::LEADER,
                LEADER,
            );
            word(
                &mut bytes,
                LAST_GROUP + engine_native::corders_group::PARENT,
                PARENT,
            );
            word(&mut bytes, PARENT, engine_native_parts::carmy_group::VTABLE_RVA.0);
            word(
                &mut bytes,
                PARENT + engine_native::corders_group::LEADER,
                PARENT_LEADER,
            );
            word(&mut bytes, PARENT_LEADER, engine_native::carmy_leader::VTABLE_RVA.0);
            if theatres {
                header(&mut bytes, COUNTRY + 0x168, 0x20000, COUNT);
                for index in 0..COUNT {
                    word(
                        &mut bytes,
                        0x20000 + index * 8,
                        if index + 1 == COUNT {
                            LAST_THEATRE
                        } else {
                            THEATRE
                        },
                    );
                }
                header(&mut bytes, THEATRE + 0x80, 0xa000, 1);
                word(&mut bytes, 0xa000, GROUP);
                header(&mut bytes, LAST_THEATRE + 0x80, 0x13000, 1);
                word(&mut bytes, 0x13000, LAST_GROUP);
            } else {
                header(&mut bytes, COUNTRY + 0x168, 0xa000, 1);
                word(&mut bytes, 0xa000, THEATRE);
                header(&mut bytes, THEATRE + 0x80, 0x20000, COUNT);
                for index in 0..COUNT {
                    word(
                        &mut bytes,
                        0x20000 + index * 8,
                        if index + 1 == COUNT {
                            LAST_GROUP
                        } else {
                            GROUP
                        },
                    );
                }
            }
            let source = engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let countries = ccountry::Reader::new(&source, 0, STATE).unwrap();
            assert_eq!(
                parent_commander(&source, 0, &countries, LEADER),
                Ok(PARENT_LEADER)
            );
            word(&mut bytes, PARENT_LEADER, 0);
            let source = engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let countries = ccountry::Reader::new(&source, 0, STATE).unwrap();
            assert_eq!(
                parent_commander(&source, 0, &countries, LEADER),
                Err("tactic context vtable mismatch")
            );
        }
    }
}
