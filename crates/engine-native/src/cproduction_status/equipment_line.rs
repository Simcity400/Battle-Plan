
use crate::{
    Memory, cequipment_type, cequipment_variant, cmilitary_production_line, cnaval_production_line,
    cpdx_array::Array, cproduction_line, crailway_gun_production_line, cship_refit_production_line,
    read_slice,
};
use engine_sites::Rva;

pub const EQUIPMENT_VARIANT: usize = 0x88;
pub const INDUSTRIAL_ORGANISATION_CID: usize = 0x90;
pub const CONVERSION: usize = 0xec;
pub const REQUESTED_FACTORIES: usize = 0xe8;
pub const FACTORY_EFFICIENCIES: usize = 0xc0;

pub const MAX_FACTORIES_VIRTUAL_SLOT: usize = 0x58;
pub const MAX_MIL_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_06cc);
pub const CONVOY_MAX_NAV_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_04c8);
pub const FLOATING_HARBOR_MAX_NAV_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_0558);
pub const DEFAULT_MAX_NAV_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_0430);
pub const CAPITAL_SHIP_MAX_NAV_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_060c);
pub const RAILWAY_GUN_MAX_MIL_FACTORIES_PER_LINE_RVA: Rva = Rva(0x033e_081c);
pub const MAX_FACTORY_DEFINE_RVAS: [Rva; 6] = [
    MAX_MIL_FACTORIES_PER_LINE_RVA,
    CONVOY_MAX_NAV_FACTORIES_PER_LINE_RVA,
    FLOATING_HARBOR_MAX_NAV_FACTORIES_PER_LINE_RVA,
    DEFAULT_MAX_NAV_FACTORIES_PER_LINE_RVA,
    CAPITAL_SHIP_MAX_NAV_FACTORIES_PER_LINE_RVA,
    RAILWAY_GUN_MAX_MIL_FACTORIES_PER_LINE_RVA,
];
pub const EFFICIENCY_BATCH: usize = 256;

pub use engine_executable::native_values::production_line::KIND_MILITARY;
pub use engine_executable::native_values::production_line::KIND_NAVAL;
pub use engine_executable::native_values::production_line::KIND_SHIP_REFIT;
pub use engine_executable::native_values::production_line::KIND_RAILWAY_GUN;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub handle: u64,
    pub country: u32,
    pub kind: u8,
    pub variant: u64,
    pub mio: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
    pub cost_raw: i64,
    pub speed_raw: i64,
    pub produced_raw: i64,
    pub priority: i32,
    pub amount: i32,
    pub max_factories: i32,
    pub efficiency_raw: i64,
}

pub use game_model::production::efficiency_average;

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
        let addresses = MAX_FACTORY_DEFINE_RVAS.map(|rva| engine_sites::at(base, rva).unwrap_or(0));
        if memory.copy_scattered(addresses, &mut bytes.each_mut().map(|value| &mut value[..])) != 6
        {
            return Err("production line factory defines unreadable");
        }
        let values = bytes.map(i32::from_le_bytes);
        self.0 = Some(values);
        Ok(values[index])
    }
}

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
    let expected = engine_sites::at(base, cequipment_type::VTABLE_RVA)
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
    let address = usize::try_from(vtable).map_err(|_| "production line vtable mismatch")?;
    let kind = match engine_sites::which(
        base,
        address,
        &[
            cmilitary_production_line::VTABLE_RVA,
            cnaval_production_line::VTABLE_RVA,
            cship_refit_production_line::VTABLE_RVA,
            crailway_gun_production_line::VTABLE_RVA,
        ],
    ) {
        Some(cmilitary_production_line::VTABLE_RVA) => KIND_MILITARY,
        Some(cnaval_production_line::VTABLE_RVA) => KIND_NAVAL,
        Some(cship_refit_production_line::VTABLE_RVA) => KIND_SHIP_REFIT,
        Some(crailway_gun_production_line::VTABLE_RVA) => KIND_RAILWAY_GUN,
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
        assert_eq!(efficiency_average(-7_i64 as u64, 2), -3);
        assert_eq!(efficiency_average((1 << 48) + 6, 3), -(1_i64 << 48) / 3 + 2);
        assert_eq!(efficiency_average((1 << 49) + 6, 3), 2);
    }
}
