//! CTerrainDatabase graphics lookup used by the city predicate.
//! Graphics records and simulation terrain types occupy different arrays.

/// CTerrainDatabase singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_6cd8;
/// CTerrainDatabase CTerrainGraphics pointer array.
pub const GRAPHICS: usize = 0x70;
/// Signed graphics count; an invalid index selects graphics slot zero.
pub const GRAPHICS_COUNT: usize = 0x7c;
/// CTerrainDatabase sampled-byte-to-graphics-index array of i32.
pub const GRAPHICS_INDEX: usize = 0x88;
/// CTerrainDatabase signed graphics-index count.
pub const GRAPHICS_INDEX_COUNT: usize = 0x94;
/// Terrain database constructor; initializes separate graphics and simulation null records.
pub const CONSTRUCT_RVA: usize = 0x00ac_6860;
/// Terrain database pointer array, its null object first: terrain-or-ocean
/// lookup and the terrain predicate pass `+0x40` with name lookup `+0x28` to
/// hashed-name lookup, which returns `array[index + 1]`.
pub const TYPES: usize = 0x40;
/// A terrain's own unit adjuster (`CTerrainType+0x168`, initialized during
/// construction) and its attack, added to a stat block's entry.
pub const TYPE_ADJUSTER: usize = 0x168;

/// Every live terrain's database index (`CTerrainType+0x88`) and own adjuster
/// attack, in array order from slot 1, each type and adjuster authenticated.
/// The attack-list witness reads it; nothing published derives from it.
pub fn terrain_attacks(
    memory: &impl crate::Memory,
    base: usize,
    output: &mut Vec<(i32, i64)>,
) -> Result<(), &'static str> {
    output.clear();
    let database = base
        .checked_add(SINGLETON_RVA)
        .and_then(|address| crate::read::<u64>(memory, address))
        .and_then(|database| usize::try_from(database).ok())
        .filter(|&database| database != 0)
        .ok_or("terrain database unreadable")?;
    let types = database
        .checked_add(TYPES)
        .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
        .ok_or("terrain database types unreadable")?;
    let terrain_vtable = base
        .checked_add(crate::cterrain_type::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let adjuster_vtable = base
        .checked_add(crate::csub_unit_definition::UNIT_ADJUSTER_VTABLE_RVA)
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
