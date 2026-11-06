//! CTerrainDatabase graphics lookup used by the city predicate.
//! Graphics records and simulation terrain types occupy different arrays.

use engine_sites::Rva;
/// CTerrainDatabase singleton pointer.
pub const SINGLETON_RVA: Rva = Rva(0x033d_d4f0);
/// CTerrainDatabase CTerrainGraphics pointer array.
pub const GRAPHICS: usize = 0x70;
/// Terrain database constructor; initializes separate graphics and simulation null records.
pub const CONSTRUCT_RVA: Rva = Rva(0x00b2_8610);
/// Terrain database pointer array, its null object first: terrain-or-ocean lookup and the terrain
/// predicate pass `TYPES` with name lookup to hashed-name lookup, which returns `array[index + 1]`.
pub const TYPES: usize = 0x40;
/// A terrain's own unit adjuster (`TYPE_ADJUSTER`, initialized during construction) and its attack,
/// added to a stat block's entry.
pub const TYPE_ADJUSTER: usize = 0x168;

/// Every live terrain's database index (its terrain type's) and own adjuster attack, in array order
/// from slot 1, each type and adjuster authenticated. The attack-list witness reads it; nothing
/// published derives from it.
pub fn terrain_attacks(
    memory: &impl crate::Memory,
    base: usize,
    output: &mut Vec<(i32, i64)>,
) -> Result<(), &'static str> {
    output.clear();
    let database = engine_sites::at(base, SINGLETON_RVA)
        .and_then(|address| crate::read::<u64>(memory, address))
        .and_then(|database| usize::try_from(database).ok())
        .filter(|&database| database != 0)
        .ok_or("terrain database unreadable")?;
    let types = database
        .checked_add(TYPES)
        .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
        .ok_or("terrain database types unreadable")?;
    let terrain_vtable = engine_sites::at(base, crate::cterrain_type::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let adjuster_vtable =
        engine_sites::at(base, crate::csub_unit_definition::UNIT_ADJUSTER_VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
    crate::allocation::reserve_exact(output, types.count as usize)
        .map_err(|_| "terrain database allocation failed")?;
    for slot in 1..types.count {
        let pointer = types
            .element(slot, 8)
            .and_then(|address| crate::read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("terrain type pointer unreadable")?;
        let field = |offset: usize| -> Result<[u8; 8], &'static str> {
            pointer
                .checked_add(offset)
                .and_then(|address| crate::read::<[u8; 8]>(memory, address))
                .ok_or("terrain type unreadable")
        };
        if u64::from_le_bytes(field(0)?) != terrain_vtable
            || u64::from_le_bytes(field(TYPE_ADJUSTER)?) != adjuster_vtable
        {
            return Err("terrain type identity mismatch");
        }
        let index = pointer
            .checked_add(crate::cterrain_type::DATABASE_INDEX)
            .and_then(|address| crate::read::<i32>(memory, address))
            .ok_or("terrain type index unreadable")?;
        let attack = i64::from_le_bytes(field(
            TYPE_ADJUSTER + crate::csub_unit_definition::UNIT_ADJUSTER_ATTACK,
        )?);
        output.push((index, attack));
    }
    output.sort_unstable_by_key(|(index, _)| *index);
    Ok(())
}
