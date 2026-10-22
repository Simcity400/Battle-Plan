//! CEquipmentProductionLine, the offset-zero base of military, naval, refit
//! and railway-gun lines. Reads never invoke game helpers.

use crate::{
    Memory, cequipment_type, cequipment_variant, cmilitary_production_line, cnaval_production_line,
    cpdx_array::Array, cproduction_line, crailway_gun_production_line, cship_refit_production_line,
    read_slice,
};

/// CEquipmentProductionLine CEquipmentVariant pointer.
pub const EQUIPMENT_VARIANT: usize = 0x88;
/// CEquipmentProductionLine industrial organisation CID (type u32 then id u32).
pub const INDUSTRIAL_ORGANISATION_CID: usize = 0x90;
/// Conversion control byte. Military production converts stockpiled
/// equipment into the line's design while it is set and the design is a
/// conversion target; the save writes it as `is_converting`. Line refresh and
/// the player's command change it, never the production advance itself.
pub const CONVERSION: usize = 0xec;
/// CEquipmentProductionLine requested factory count i32.
pub const REQUESTED_FACTORIES: usize = 0xe8;
/// Per-factory efficiency array of signed i64 Q15 values (x32768).
/// Every counted entry is serialized. Refresh grows to maximum factory count
/// and zero-fills new entries without shrinking the array, so the saved length
/// can exceed active factories. The ordinary census records the average;
/// `production_modifiers` retains the complete ordered array.
pub const FACTORY_EFFICIENCIES: usize = 0xc0;

/// Maximum-factory dispatch slot, used to clamp the efficiency count.
/// Military lines select their type's optional limit, then its archetype's,
/// then the military define. Naval and refit lines use that sequence with the
/// naval optional limit and convoy, floating-harbour, capital-ship or default
/// define selected by classifier bits. Railway-gun lines use their define alone.
pub const MAX_FACTORIES_VIRTUAL_SLOT: usize = 0x58;
/// Loaded military maximum factories per line, signed i32.
pub const MAX_MIL_FACTORIES_PER_LINE_RVA: usize = 0x0331_d454;
/// Loaded convoy maximum naval factories per line, signed i32; classifier bit 0.
pub const CONVOY_MAX_NAV_FACTORIES_PER_LINE_RVA: usize = 0x0331_d254;
/// Loaded floating-harbour maximum naval factories per line, signed i32; classifier bit 9.
pub const FLOATING_HARBOR_MAX_NAV_FACTORIES_PER_LINE_RVA: usize = 0x0331_d2fc;
/// Loaded default maximum naval factories per line, signed i32.
pub const DEFAULT_MAX_NAV_FACTORIES_PER_LINE_RVA: usize = 0x0331_d194;
/// Loaded capital-ship maximum naval factories per line, signed i32; classifier bit 6.
pub const CAPITAL_SHIP_MAX_NAV_FACTORIES_PER_LINE_RVA: usize = 0x0331_d3a0;
/// Loaded railway-gun maximum factories per line, signed i32.
pub const RAILWAY_GUN_MAX_MIL_FACTORIES_PER_LINE_RVA: usize = 0x0331_d57c;
/// The six defines above, read together once per walk, in this order.
pub const MAX_FACTORY_DEFINE_RVAS: [usize; 6] = [
    MAX_MIL_FACTORIES_PER_LINE_RVA,
    CONVOY_MAX_NAV_FACTORIES_PER_LINE_RVA,
    FLOATING_HARBOR_MAX_NAV_FACTORIES_PER_LINE_RVA,
    DEFAULT_MAX_NAV_FACTORIES_PER_LINE_RVA,
    CAPITAL_SHIP_MAX_NAV_FACTORIES_PER_LINE_RVA,
    RAILWAY_GUN_MAX_MIL_FACTORIES_PER_LINE_RVA,
];
/// Efficiency entries copied per guarded read.
pub const EFFICIENCY_BATCH: usize = 256;

/// Military production, distinguished by the exact native primary vtable.
pub const KIND_MILITARY: u8 = 0;
/// Naval new-build production, distinguished by the exact native primary vtable.
pub const KIND_NAVAL: u8 = 1;
/// Ship refit production, distinguished by the exact native primary vtable.
pub const KIND_SHIP_REFIT: u8 = 2;
/// Railway-gun production, distinguished by the exact native primary vtable.
pub const KIND_RAILWAY_GUN: u8 = 3;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native CReferenceObject type/id words, including their high bits.
    pub handle: u64,
    pub country: u32,
    pub kind: u8,
    /// Authenticated CEquipmentVariant type/id words, not its process pointer.
    pub variant: u64,
    /// Native industrial organisation CID; no resolver or game helper is called.
    pub mio: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
    pub cost_raw: i64,
    pub speed_raw: i64,
    pub produced_raw: i64,
    pub priority: i32,
    pub amount: i32,
    /// The line's maximum factories (virtual slot 11), the count the
    /// efficiency average is clamped to. See [`MAX_FACTORIES_VIRTUAL_SLOT`].
    pub max_factories: i32,
    /// The current efficiency the production view shows, i64 Q15 (x32768):
    /// [`efficiency_average`] over the leading factories, using the lesser of
    /// the active and maximum factory counts.
    pub efficiency_raw: i64,
}

/// Average of `count` entries with wrapping u64 `sum`. Nonpositive counts
/// return zero. Otherwise both operands are shifted left by 15 bits and
/// divided as signed i64 values, truncating toward zero. The caller uses the
/// minimum of active and maximum factories.
pub fn efficiency_average(sum: u64, count: i32) -> i64 {
    if count <= 0 {
        return 0;
    }
    ((sum << 15) as i64) / (i64::from(count) << 15)
}

/// The six maximum-factory defines, read together once per walk, when the
/// first line needs one. Defines are reloadable, so no walk reuses another's.
#[derive(Debug, Default)]
pub struct Defines(Option<[i32; 6]>);

impl Defines {
    fn get(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: usize,
    ) -> Result<i32, &'static str> {
        if let Some(values) = self.0 {
            return Ok(values[index]);
        }
        let mut bytes = [[0_u8; 4]; 6];
        let addresses = MAX_FACTORY_DEFINE_RVAS.map(|rva| base.checked_add(rva).unwrap_or(0));
        if memory.copy_scattered(addresses, &mut bytes.each_mut().map(|value| &mut value[..])) != 6
        {
            return Err("production line factory defines unreadable");
        }
        let values = bytes.map(i32::from_le_bytes);
        self.0 = Some(values);
        Ok(values[index])
    }
}

/// The line's virtual slot 11 without calling it: the equipment type's
/// optional, then its archetype's, then the kind's define. Both equipment
/// types are authenticated by vtable; the game dereferences them unchecked.
fn max_factories(
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    defines: &mut Defines,
) -> Result<i32, &'static str> {
    let optional = match kind {
        KIND_RAILWAY_GUN => return defines.get(memory, base, 5),
        KIND_MILITARY => cequipment_type::MAX_MIL_FACTORIES,
        _ => cequipment_type::MAX_NAV_FACTORIES,
    } - cequipment_type::MAX_MIL_FACTORIES;
    let expected = base
        .checked_add(cequipment_type::VTABLE_RVA)
        .ok_or("production line equipment type module overflow")? as u64;
    let at = |pointer: usize, offset| pointer.checked_add(offset).unwrap_or(0);
    let mut definition = [0; 8];
    let mut flags = [0; 8];
    if memory.copy_scattered(
        [
            at(variant, cequipment_variant::DEFINITION),
            at(variant, cequipment_variant::CLASSIFIER_FLAGS),
        ],
        &mut [&mut definition, &mut flags],
    ) != 2
    {
        return Err("production line variant type or classifier unreadable");
    }
    let mut equipment = usize::try_from(u64::from_le_bytes(definition))
        .map_err(|_| "production line equipment type pointer overflow")?;
    // The type, then its archetype, which the constructor self-initializes; only the type's read
    // also takes the archetype link.
    for archetype in [false, true] {
        let mut vtable = [0; 8];
        let mut maximum = [0; 16];
        let mut next = [0; 8];
        let (wanted, next_len) = if archetype { (2, 0) } else { (3, 8) };
        if equipment == 0
            || memory.copy_scattered(
                [
                    equipment,
                    at(equipment, cequipment_type::MAX_MIL_FACTORIES),
                    at(equipment, cequipment_type::ARCHETYPE),
                ],
                &mut [&mut vtable, &mut maximum, &mut next[..next_len]],
            ) < wanted
        {
            return Err(if archetype {
                "production line equipment archetype unreadable or null"
            } else {
                "production line equipment type unreadable or null"
            });
        }
        if u64::from_le_bytes(vtable) != expected {
            return Err("production line equipment type vtable mismatch");
        }
        if maximum[optional + 4] != 0 {
            return Ok(i32::from_le_bytes(
                maximum[optional..optional + 4].try_into().unwrap(),
            ));
        }
        equipment = usize::try_from(u64::from_le_bytes(next))
            .map_err(|_| "production line equipment archetype pointer overflow")?;
    }
    if kind == KIND_MILITARY {
        return defines.get(memory, base, 0);
    }
    let flags = u64::from_le_bytes(flags);
    defines.get(
        memory,
        base,
        if flags & cequipment_variant::CONVOY_CLASSIFIER_MASK != 0 {
            1
        } else if flags & cequipment_variant::FLOATING_HARBOR_CLASSIFIER_MASK != 0 {
            2
        } else if flags & cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK != 0 {
            4
        } else {
            3
        },
    )
}

/// Sums the first `count` efficiency entries as the averager does, copying
/// at most [`EFFICIENCY_BATCH`] entries per guarded read. The game reads them
/// without a bound; a count beyond the array's own is refused.
fn efficiency_sum(memory: &impl Memory, header: [u8; 16], count: i32) -> Result<u64, &'static str> {
    let count = u32::try_from(count).unwrap_or(0);
    if count == 0 {
        return Ok(0);
    }
    let array = Array::from_header(header, 8).ok_or("production line efficiency array invalid")?;
    if count > array.count {
        return Err("production line efficiency array shorter than its factories");
    }
    let mut bytes = [0_u8; EFFICIENCY_BATCH * 8];
    let mut sum = 0_u64;
    for first in (0..count).step_by(EFFICIENCY_BATCH) {
        let len = ((count - first) as usize).min(EFFICIENCY_BATCH);
        let source = array
            .element(first, 8)
            .ok_or("production line efficiency array invalid")?;
        if !read_slice(memory, source, &mut bytes[..len * 8]) {
            return Err("production line efficiency entries unreadable");
        }
        for entry in bytes[..len * 8].chunks_exact(8) {
            sum = sum.wrapping_add(u64::from_le_bytes(entry.try_into().unwrap()));
        }
    }
    Ok(sum)
}

/// Copies the recorded fields and the efficiency array header in one guarded
/// scatter, resolves the variant identity, then the maximum factories and the
/// efficiency entries they select. No game helper is called; the cap and
/// growth helpers are modifier calculations and are not recorded.
pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    status: usize,
    country: u32,
    pointer: usize,
    defines: &mut Defines,
) -> Result<Row, &'static str> {
    let mut identity = [0; 16];
    let mut active_factories = [0; 4];
    let mut owner = [0; 8];
    let mut quantities = [0; 24];
    let mut ordering = [0; 8];
    let mut references = [0; 16];
    let mut requested_factories = [0; 4];
    let mut efficiencies = [0; 16];
    let at = |offset| pointer.checked_add(offset).unwrap_or(0);
    let copied = memory.copy_scattered(
        [
            pointer,
            at(cproduction_line::ACTIVE_FACTORIES),
            at(cproduction_line::PRODUCTION_STATUS),
            at(cproduction_line::COST),
            at(cproduction_line::PRIORITY),
            at(EQUIPMENT_VARIANT),
            at(REQUESTED_FACTORIES),
            at(FACTORY_EFFICIENCIES),
        ],
        &mut [
            &mut identity,
            &mut active_factories,
            &mut owner,
            &mut quantities,
            &mut ordering,
            &mut references,
            &mut requested_factories,
            &mut efficiencies,
        ],
    );
    if copied == 0 {
        return Err("production line identity unreadable");
    }
    let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
    let rva = usize::try_from(vtable)
        .ok()
        .and_then(|address| address.checked_sub(base))
        .ok_or("production line vtable mismatch")?;
    let kind = match rva {
        cmilitary_production_line::VTABLE_RVA => KIND_MILITARY,
        cnaval_production_line::VTABLE_RVA => KIND_NAVAL,
        cship_refit_production_line::VTABLE_RVA => KIND_SHIP_REFIT,
        crailway_gun_production_line::VTABLE_RVA => KIND_RAILWAY_GUN,
        _ => return Err("production line vtable mismatch"),
    };
    let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if handle == 0 {
        return Err("production line handle is null");
    }
    if copied < 3 {
        return Err("production line owner unreadable");
    }
    if u64::from_le_bytes(owner) != status as u64 {
        return Err("production line status mismatch");
    }
    if copied < 7 {
        return Err("production line fields unreadable");
    }
    if copied < 8 {
        return Err("production line efficiency header unreadable");
    }
    let variant_pointer = usize::try_from(u64::from_le_bytes(references[..8].try_into().unwrap()))
        .map_err(|_| "production line variant pointer overflow")?;
    let variant = cequipment_variant::identity(memory, base, variant_pointer)?;
    let active_factories = i32::from_le_bytes(active_factories);
    let max_factories = max_factories(memory, base, kind, variant_pointer, defines)?;
    let count = active_factories.min(max_factories);
    let efficiency_raw = efficiency_average(efficiency_sum(memory, efficiencies, count)?, count);
    Ok(Row {
        handle,
        country,
        kind,
        variant,
        mio: u64::from_le_bytes(references[8..].try_into().unwrap()),
        active_factories,
        requested_factories: i32::from_le_bytes(requested_factories),
        cost_raw: i64::from_le_bytes(quantities[..8].try_into().unwrap()),
        speed_raw: i64::from_le_bytes(quantities[8..16].try_into().unwrap()),
        produced_raw: i64::from_le_bytes(quantities[16..].try_into().unwrap()),
        priority: i32::from_le_bytes(ordering[..4].try_into().unwrap()),
        amount: i32::from_le_bytes(ordering[4..].try_into().unwrap()),
        max_factories,
        efficiency_raw,
    })
}

#[cfg(test)]
mod tests {
    use super::efficiency_average;

    #[test]
    fn average_keeps_the_native_wrapping_shift_and_truncating_division() {
        assert_eq!(efficiency_average(u64::MAX, 0), 0);
        assert_eq!(efficiency_average(7, -1), 0);
        assert_eq!(efficiency_average(7, 2), 3);
        // A negative sum truncates toward zero, as IDIV does.
        assert_eq!(efficiency_average(-7_i64 as u64, 2), -3);
        // The shift discards the sum's top 15 bits and moves bit 48 into the
        // sign before the signed division.
        assert_eq!(efficiency_average((1 << 48) + 6, 3), -(1_i64 << 48) / 3 + 2);
        assert_eq!(efficiency_average((1 << 49) + 6, 3), 2);
    }
}
