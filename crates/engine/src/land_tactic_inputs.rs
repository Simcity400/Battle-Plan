//! Raw possible-tactics inputs for 141299340 -> 1412A77A0 and popup141C5C1F0.
//!
//! Trigger expressions come from the verified combat-tactics content. This
//! module never invokes a trigger, evaluates a weight or normalizes a chance.
//! Dynamic leaves join the hourly combat, division, template, province and
//! commander rows. Script-variable factor/add branches are explicitly refused.

use crate::{
    Memory, ccombat_tactic, ccountry, cland_combat, cnull_combat_tactic, cpdx_array::Array,
    msvc_string, read, walk::Outcome,
};

pub const CANDIDATE_LIMIT: usize = 1024;
pub const WEIGHT_LIMIT: usize = 4096;
pub const UNLOCK_LIMIT: usize = 4096;
pub const TOKEN_LIMIT: usize = 128;
/// Singleton read by1406B0FC0; no lazy constructor is called.
pub const DATABASE_RVA: usize = 0x0331_88c0;
///140D6A7B0 indexes game+400 doctrine-system array+8 with strideA0.
pub const DOCTRINE_SYSTEM: usize = 0x400;
///1412A77A0 reads the general's preferred tactic pointer directly.
pub const LEADER_PREFERRED_TACTIC: usize = 0x10b0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub ordinal: u32,
    pub tactic_id: i32,
    ///1413B8CF0 unlock comparison, distinct from database id+98.
    pub unlock_id: u32,
    pub only_show_for: u32,
    pub active: u8,
    pub base_weight: i64,
    pub token: msvc_string::Text<TOKEN_LIMIT>,
    pub weight_start: usize,
    pub weight_count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WeightTerm {
    pub tactic_id: i32,
    pub ordinal: u32,
    pub factor: i64,
    pub add: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Registry {
    pub candidate_count: usize,
    pub weight_count: usize,
    pub required_candidates: usize,
    pub required_weights: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryCapture {
    pub outcome: Outcome,
    pub row: Registry,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unlock {
    pub country: u32,
    pub ordinal: u32,
    pub unlock_id: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    /// First linked country on attacker and defender, then viewpoint country.
    pub countries: [u32; 3],
    /// Country, general and field marshal database ids, separately per side.
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub commander_handles: [[u64; 2]; 2],
    /// Country, general and field-marshal preference factor defines.
    pub preference_factors: [i64; 3],
    pub country_weights: [ccombat_tactic::WeightRow; 2],
    pub country_weight_present: [u8; 2],
    pub unlock_start: [usize; 3],
    pub unlock_count: [usize; 3],
    pub required_unlocks: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextCapture {
    pub outcome: Outcome,
    pub row: ContextRow,
}

fn field<T: crate::memory::Raw>(
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
    limit: usize,
) -> Result<Array, &'static str> {
    parent
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .filter(|a| a.count as usize <= limit)
        .ok_or("tactic input array invalid or exceeds safety bound")
}

fn identity(memory: &impl Memory, base: usize, tactic: usize) -> Result<i32, &'static str> {
    let vtable = field::<u64>(memory, tactic, 0)?;
    if base
        .checked_add(cnull_combat_tactic::VTABLE_RVA)
        .map(|p| p as u64)
        == Some(vtable)
    {
        return Ok(0);
    }
    if base
        .checked_add(ccombat_tactic::VTABLE_RVA)
        .map(|p| p as u64)
        != Some(vtable)
    {
        return Err("tactic vtable mismatch");
    }
    let id = field(memory, tactic, ccombat_tactic::ID)?;
    if id < 1 {
        return Err("tactic id invalid");
    }
    Ok(id)
}

fn literal(memory: &impl Memory, variable: usize) -> Result<i64, &'static str> {
    //1405382B0 returns+40 immediately only when+9 is nonzero.
    if field::<u8>(memory, variable, 9)? == 0 {
        return Err("tactic scoped weight uses unsupported dynamic script variable");
    }
    field(memory, variable, 0x40)
}

/// Capture the loaded registry once per combat acquisition pass. All candidates
/// are retained in database order, including inactive and viewpoint-hidden ones.
/// Pressure discards the complete registry; exact required arena sizes survive.
pub fn capture_registry(
    memory: &impl Memory,
    base: usize,
    candidates: &mut [Candidate],
    weights: &mut [WeightTerm],
) -> RegistryCapture {
    let mut row = Registry::default();
    let result = (|| {
        let db = pointer(memory, base, DATABASE_RVA)?;
        if db == 0 {
            return Err("tactic database is not initialized");
        }
        let entries = array(memory, db, 0x10, 8, CANDIDATE_LIMIT + 1)?;
        if entries.count == 0 {
            return Err("tactic database missing null entry");
        }
        row.required_candidates = entries.count as usize - 1;
        for index in 1..entries.count {
            let tactic = pointer(memory, entries.data, index as usize * 8)?;
            let id = identity(memory, base, tactic)?;
            if id != index as i32 {
                return Err("tactic registry id mismatch");
            }
            let terms = array(memory, tactic, 0xc8, 8, WEIGHT_LIMIT)?;
            let start = row.required_weights;
            row.required_weights = start
                .checked_add(terms.count as usize)
                .filter(|n| *n <= WEIGHT_LIMIT)
                .ok_or("tactic weights exceed safety bound")?;
            let token = msvc_string::read_text(
                memory,
                tactic
                    .checked_add(ccombat_tactic::TOKEN)
                    .ok_or("tactic token address overflow")?,
            )
            .ok_or("tactic token invalid or exceeds safety bound")?;
            let value = Candidate {
                ordinal: index,
                tactic_id: id,
                unlock_id: field(memory, tactic, 0x94)?,
                only_show_for: field(memory, tactic, 8)?,
                active: field(memory, tactic, 0x170)?,
                base_weight: field(memory, tactic, 0xc0)?,
                token,
                weight_start: start,
                weight_count: terms.count as usize,
            };
            for ordinal in 0..terms.count {
                let term = pointer(memory, terms.data, ordinal as usize * 8)?;
                let value = WeightTerm {
                    tactic_id: id,
                    ordinal,
                    factor: literal(
                        memory,
                        term.checked_add(0x58)
                            .ok_or("tactic factor address overflow")?,
                    )?,
                    add: literal(
                        memory,
                        term.checked_add(0x128)
                            .ok_or("tactic add address overflow")?,
                    )?,
                };
                if let Some(slot) = weights.get_mut(start + ordinal as usize) {
                    *slot = value;
                }
            }
            if let Some(slot) = candidates.get_mut(index as usize - 1) {
                *slot = value;
            }
        }
        Ok(())
    })();
    let outcome = match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(())
            if row.required_candidates > candidates.len()
                || row.required_weights > weights.len() =>
        {
            Outcome::Truncated {
                len: 0,
                unread_rows: row.required_candidates as u32,
            }
        }
        Ok(()) => {
            row.candidate_count = row.required_candidates;
            row.weight_count = row.required_weights;
            Outcome::Rows {
                len: row.candidate_count,
            }
        }
    };
    RegistryCapture { outcome, row }
}

fn authentic(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rvas: &[usize],
) -> Result<(), &'static str> {
    let vtable = field::<u64>(memory, object, 0)?;
    if rvas
        .iter()
        .any(|rva| base.checked_add(*rva).map(|p| p as u64) == Some(vtable))
    {
        Ok(())
    } else {
        Err("tactic context vtable mismatch")
    }
}

///1412A77A0 finds the first formation in native theatre order with this
/// commander and a parent. This intentionally does not substitute leader+1048.
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
    let theatres = array(memory, owner.pointer, 0x168, 8, 4096)?;
    let mut visited = 0usize;
    for ordinal in 0..theatres.count {
        let theatre = pointer(memory, theatres.data, ordinal as usize * 8)?;
        authentic(memory, base, theatre, &[crate::ctheatre::VTABLE_RVA])?;
        let groups = array(memory, theatre, 0x80, 8, 4096)?;
        visited = visited
            .checked_add(groups.count as usize)
            .filter(|n| *n <= 4096)
            .ok_or("tactic formation census exceeds safety bound")?;
        for index in 0..groups.count {
            let group = pointer(memory, groups.data, index as usize * 8)?;
            authentic(
                memory,
                base,
                group,
                &[
                    crate::corders_group::VTABLE_RVA,
                    crate::carmy_group::VTABLE_RVA,
                ],
            )?;
            if pointer(memory, group, crate::corders_group::LEADER)? != leader {
                continue;
            }
            let parent = pointer(memory, group, crate::corders_group::PARENT)?;
            if parent == 0 {
                continue;
            }
            authentic(memory, base, parent, &[crate::carmy_group::VTABLE_RVA])?;
            let result = pointer(memory, parent, crate::corders_group::LEADER)?;
            if result != 0 {
                authentic(memory, base, result, &[crate::carmy_leader::VTABLE_RVA])?;
            }
            return Ok(result);
        }
    }
    Ok(0)
}

/// Bounded hourly context and unlock arena, preserving native selection order.
/// The combat's commander selections are shared with its other readers
/// (`cland_combat::Commanders`); `members` are the census's participants.
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    source: cland_combat::Source,
    members: &cland_combat::Members<'_>,
    selections: &mut cland_combat::Commanders,
    unlocks: &mut [Unlock],
) -> ContextCapture {
    let mut row = ContextRow::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game)?;
        let commanders =
            cland_combat::commander_sources(memory, base, source, members, selections)?;
        let sides = [
            pointer(memory, source.pointer, cland_combat::ATTACKER)?,
            pointer(memory, source.pointer, cland_combat::DEFENDER)?,
        ];
        for (index, side) in sides.into_iter().enumerate() {
            //141299340 passes the first linked-set node as a CCountryTag*.
            // A side whose last unit has left keeps an empty set until the
            // combat is removed; the null CCountryTag 0 records that no country
            // scope, preference, weight or unlock list exists for that side.
            let first = pointer(memory, side, crate::cland_combatant::COUNTRIES)?;
            if first != 0 {
                row.countries[index] = field(memory, first, 0)?;
                if row.countries[index] == 0 {
                    return Err("tactic side country scope is null");
                }
            }
        }
        let viewpoint = field::<u32>(memory, game, 0x520)?;
        row.countries[2] = if viewpoint as i32 > 0 {
            viewpoint
        } else {
            field(memory, game, 0x524)?
        };
        for (value, rva) in
            row.preference_factors
                .iter_mut()
                .zip([0x0331_d370, 0x0331_d3f0, 0x0331_d478])
        {
            *value = field(memory, base, rva)?;
        }
        let weight_context = ccombat_tactic::WeightContext::read(memory, base)?;
        for (index, commander) in commanders.iter().enumerate() {
            if row.countries[index] != 0 {
                let country = countries.resolve(row.countries[index])?;
                let mut token = [0; TOKEN_LIMIT];
                let preferred = ccombat_tactic::preferred(
                    memory,
                    base,
                    country.pointer,
                    country.id,
                    &mut token,
                );
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
            let leader = commander.pointer;
            let parent = parent_commander(memory, base, &countries, leader)?;
            for (position, leader) in [leader, parent].into_iter().enumerate() {
                if leader == 0 {
                    continue;
                }
                let handle = field::<u64>(memory, leader, crate::cunit_leader::TYPE)?;
                if handle as u32 != crate::cunit_leader::REFERENCE_TYPE || handle >> 32 == 0 {
                    return Err("tactic commander handle invalid");
                }
                row.commander_handles[index][position] = handle;
                row.preferred_tactic_ids[index][position + 1] = identity(
                    memory,
                    base,
                    pointer(memory, leader, LEADER_PREFERRED_TACTIC)?,
                )?;
            }
        }
        let system = pointer(memory, game, DOCTRINE_SYSTEM)?;
        if system == 0 {
            return Err("tactic doctrine system missing");
        }
        let doctrine_rows = array(memory, system, 8, 0xa0, 65536)?;
        for index in 0..3 {
            if row.countries[index] == 0 {
                continue;
            }
            let country = countries.resolve(row.countries[index])?;
            let country_row = doctrine_rows
                .element(country.canonical_slot, 0xa0)
                .ok_or("tactic doctrine country outside table")?;
            let entries = array(memory, country_row, 0x28, 8, UNLOCK_LIMIT)?;
            let start = row.required_unlocks;
            row.required_unlocks = start
                .checked_add(entries.count as usize)
                .filter(|n| *n <= UNLOCK_LIMIT)
                .ok_or("tactic unlocks exceed safety bound")?;
            row.unlock_start[index] = start;
            row.unlock_count[index] = entries.count as usize;
            for ordinal in 0..entries.count {
                let tactic = pointer(memory, entries.data, ordinal as usize * 8)?;
                if identity(memory, base, tactic)? == 0 {
                    return Err("tactic unlock list contains null tactic");
                }
                let value = Unlock {
                    country: row.countries[index],
                    ordinal,
                    unlock_id: field(memory, tactic, 0x94)?,
                };
                if let Some(slot) = unlocks.get_mut(start + ordinal as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    })();
    let outcome = match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if row.required_unlocks > unlocks.len() => Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        },
        Ok(()) => Outcome::Rows { len: 1 },
    };
    ContextCapture { outcome, row }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::Cell;

    struct Fixture {
        bytes: Vec<u8>,
        copies: Cell<usize>,
        copied_bytes: Cell<usize>,
        fault: Option<usize>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.copies.set(self.copies.get() + 1);
            self.copied_bytes
                .set(self.copied_bytes.get() + output.len());
            if self.fault == Some(address) {
                return false;
            }
            if address == DATABASE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x1000_u64.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
        bytes[at..at + N].copy_from_slice(&value);
    }
    fn header(bytes: &mut [u8], at: usize, data: u64, count: u32) {
        put(bytes, at, data.to_le_bytes());
        put(bytes, at + 8, count.to_le_bytes());
        put(bytes, at + 12, count.to_le_bytes());
    }
    fn fixture() -> Fixture {
        let mut bytes = vec![0; 0x5000];
        header(&mut bytes, 0x1010, 0x1800, 2);
        put(&mut bytes, 0x1808, 0x2000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x2000,
            (ccombat_tactic::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x2008, 37_u32.to_le_bytes());
        put(&mut bytes, 0x2010, [0xff, b'x', 0]);
        put(&mut bytes, 0x2020, 2_u64.to_le_bytes());
        put(&mut bytes, 0x2028, 15_u64.to_le_bytes());
        put(&mut bytes, 0x2094, 0xdeadbeef_u32.to_le_bytes());
        put(&mut bytes, 0x2098, 1_i32.to_le_bytes());
        put(&mut bytes, 0x20c0, i64::MIN.to_le_bytes());
        header(&mut bytes, 0x20c8, 0x3000, 1);
        put(&mut bytes, 0x3000, 0x4000_u64.to_le_bytes());
        bytes[0x4061] = 7;
        bytes[0x4131] = 2;
        put(&mut bytes, 0x4098, i64::MAX.to_le_bytes());
        put(&mut bytes, 0x4168, (-123_i64).to_le_bytes());
        Fixture {
            bytes,
            copies: Cell::new(0),
            copied_bytes: Cell::new(0),
            fault: None,
        }
    }

    #[test]
    fn registry_preserves_raw_weights_hidden_inactive_entries_and_bytes() {
        let memory = fixture();
        let mut candidates = [Candidate::default(); 1];
        let mut terms = [WeightTerm::default(); 1];
        let capture = capture_registry(&memory, 0, &mut candidates, &mut terms);
        assert_eq!(capture.outcome, Outcome::Rows { len: 1 });
        assert_eq!(capture.row.required_weights, 1);
        assert_eq!(candidates[0].base_weight, i64::MIN);
        assert_eq!(candidates[0].token.as_bytes(), [0xff, b'x']);
        assert_eq!(
            (
                candidates[0].active,
                candidates[0].only_show_for,
                candidates[0].unlock_id
            ),
            (0, 37, 0xdeadbeef)
        );
        assert_eq!(
            terms[0],
            WeightTerm {
                tactic_id: 1,
                ordinal: 0,
                factor: i64::MAX,
                add: -123
            }
        );
        //One candidate with one literal modifier: guarded reads only, no calls
        //into the engine and no callback-owned heap or country census.
        assert_eq!(memory.copies.get(), 19);
        assert_eq!(memory.copied_bytes.get(), 122);
    }

    #[test]
    fn registry_pressure_and_unsupported_variables_are_distinct() {
        let mut memory = fixture();
        let pressure = capture_registry(&memory, 0, &mut [], &mut []);
        assert_eq!(
            pressure.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            pressure.row,
            Registry {
                required_candidates: 1,
                required_weights: 1,
                ..Registry::default()
            }
        );
        memory.bytes[0x4061] = 0;
        assert_eq!(
            capture_registry(
                &memory,
                0,
                &mut [Candidate::default(); 1],
                &mut [WeightTerm::default(); 1]
            )
            .outcome,
            Outcome::Unavailable {
                reason: "tactic scoped weight uses unsupported dynamic script variable"
            }
        );
        memory.bytes[0x4061] = 1;
        memory.fault = Some(0x2094);
        assert_eq!(
            capture_registry(
                &memory,
                0,
                &mut [Candidate::default(); 1],
                &mut [WeightTerm::default(); 1]
            )
            .outcome,
            Outcome::Unavailable {
                reason: "tactic input unreadable"
            }
        );
    }
}
