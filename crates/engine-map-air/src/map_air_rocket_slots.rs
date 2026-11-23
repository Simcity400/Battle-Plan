
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_air::cair_base;
use engine_native_parts::{cbuilding, cbuilding_template};
use engine_native::{cprovince::buildings as cbuilding_status, cstate};
use engine_sites::Rva;

const MISSILE_SLOT_WEIGHT: usize = 0x310;
const BUILDING_TEMPLATE_VTABLE_RVA: Rva = Rva(0x0286_1988);
const PRIMARY_CONTRIBUTIONS: usize = 0x10;
const SECONDARY_CONTRIBUTIONS: usize = 0x28;
const CONTRIBUTION_LEVEL: usize = 0x48;
const CONTRIBUTION_FLAG: usize = 0x8c;

pub fn state_rocket_slots(
    memory: &impl Memory,
    module_base: usize,
    air_base: usize,
) -> Result<Option<i16>, &'static str> {
    require_vtable(memory, module_base, air_base, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
    let handle = field::<u64>(
        memory,
        air_base,
        engine_native_parts::cair_base_layout::HANDLE,
        "rocket base handle unreadable",
    )?;
    if handle == 0 || handle as u32 != engine_native_parts::cair_base_layout::TYPE {
        return Err("rocket base handle invalid");
    }
    let kind = field::<i32>(
        memory,
        air_base,
        cair_base::KIND,
        "rocket base kind unreadable",
    )?;
    if kind != 1 {
        return Err("rocket slot reader requires kind-one base");
    }
    let state = pointer(
        memory,
        air_base,
        cair_base::STATE,
        "rocket base state unreadable",
    )?;
    if state == 0 {
        let host = field::<u64>(
            memory,
            air_base,
            engine_native_parts::cair_base_layout::HOST_REFERENCE,
            "rocket host reference unreadable",
        )?;
        if host == 0 {
            return Err("stateless rocket base has no host reference");
        }
        return Ok(None);
    }

    require_vtable(memory, module_base, state, cstate::VTABLE_RVA)?;
    let state_id = field::<u32>(memory, state, cstate::ID, "rocket base state id unreadable")?;
    let status = state
        .checked_add(cstate::BUILDING_STATUS)
        .ok_or("rocket state building-status address overflow")?;
    require_vtable(memory, module_base, status, cbuilding_status::VTABLE_RVA)?;
    let status_id = field::<u32>(
        memory,
        status,
        cbuilding_status::ID,
        "rocket building-status id unreadable",
    )?;
    let flags = field::<u32>(
        memory,
        status,
        cbuilding_status::FLAGS,
        "rocket building-status flags unreadable",
    )?;
    if status_id != state_id || flags & cbuilding_status::STATE_FLAG == 0 {
        return Err("rocket building-status state identity mismatch");
    }
    let buildings = bounded_array(
        memory,
        status,
        cbuilding_status::ACTIVE_BUILDINGS,
        "rocket building vector unreadable or invalid",
    )?;
    for index in 0..buildings.count {
        let building = array_pointer(
            memory,
            buildings,
            index,
            "rocket building pointer unreadable",
        )?;
        if building == 0 {
            return Err("rocket building vector contains null");
        }
        require_vtable(memory, module_base, building, cbuilding::VTABLE_RVA)?;
        let backlink = pointer(
            memory,
            building,
            cbuilding::BUILDING_STATUS,
            "rocket building status backlink unreadable",
        )?;
        if backlink != status {
            return Err("rocket building belongs to another status");
        }
        let template = pointer(
            memory,
            building,
            cbuilding::TEMPLATE,
            "rocket building template unreadable",
        )?;
        if template == 0 {
            return Err("rocket building has null template");
        }
        require_vtable(memory, module_base, template, BUILDING_TEMPLATE_VTABLE_RVA)?;
        let building_type = field::<u32>(
            memory,
            building,
            cbuilding::TYPE_ID,
            "rocket building token unreadable",
        )?;
        let template_type = field::<u32>(
            memory,
            template,
            cbuilding_template::TYPE_ID,
            "rocket template token unreadable",
        )?;
        if building_type != template_type {
            return Err("rocket building template token mismatch");
        }
        let weight = field::<i32>(
            memory,
            template,
            MISSILE_SLOT_WEIGHT,
            "rocket missile-slot weight unreadable",
        )?;
        if weight > 0 {
            return aggregate_level(memory, building).map(Some);
        }
    }
    Ok(Some(0))
}

fn aggregate_level(memory: &impl Memory, building: usize) -> Result<i16, &'static str> {
    let primary = bounded_array(
        memory,
        building,
        PRIMARY_CONTRIBUTIONS,
        "rocket primary contributions unreadable or invalid",
    )?;
    let mut flagged_max = 0_i16;
    let mut ordinary_max = 0_i16;
    for index in 0..primary.count {
        let node = array_pointer(
            memory,
            primary,
            index,
            "rocket primary contribution pointer unreadable",
        )?;
        let (flagged, level) = contribution(memory, node)?;
        if flagged {
            flagged_max = flagged_max.max(level);
        } else {
            ordinary_max = ordinary_max.max(level);
        }
    }

    let secondary = bounded_array(
        memory,
        building,
        SECONDARY_CONTRIBUTIONS,
        "rocket secondary contributions unreadable or invalid",
    )?;
    let mut additive = 0_i16;
    for index in 0..secondary.count {
        let node = array_pointer(
            memory,
            secondary,
            index,
            "rocket secondary contribution pointer unreadable",
        )?;
        let (flagged, level) = contribution(memory, node)?;
        if !flagged {
            additive = additive.wrapping_add(level);
        }
    }
    let built = field::<i16>(
        memory,
        building,
        cbuilding::LEVEL,
        "rocket built level unreadable",
    )?;
    Ok(ordinary_max
        .wrapping_add(additive.wrapping_add(built))
        .wrapping_add(flagged_max))
}

fn contribution(memory: &impl Memory, node: usize) -> Result<(bool, i16), &'static str> {
    if node == 0 {
        return Err("rocket contribution vector contains null");
    }
    let flag = field::<u8>(
        memory,
        node,
        CONTRIBUTION_FLAG,
        "rocket contribution flag unreadable",
    )?;
    let level = field::<i16>(
        memory,
        node,
        CONTRIBUTION_LEVEL,
        "rocket contribution level unreadable",
    )?;
    Ok((flag != 0, level))
}

fn bounded_array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    error: &'static str,
) -> Result<Array, &'static str> {
    let at = object.checked_add(offset).ok_or(error)?;
    Array::read(memory, at, 8).ok_or(error)
}

fn array_pointer(
    memory: &impl Memory,
    array: Array,
    index: u32,
    error: &'static str,
) -> Result<usize, &'static str> {
    let at = array.element(index, 8).ok_or(error)?;
    usize::try_from(read::<u64>(memory, at).ok_or(error)?).map_err(|_| error)
}

fn pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    error: &'static str,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset, error)?).map_err(|_| error)
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    error: &'static str,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read::<T>(memory, at))
        .ok_or(error)
}

fn require_vtable(
    memory: &impl Memory,
    module_base: usize,
    object: usize,
    rva: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(module_base, rva).ok_or("rocket vtable address overflow")?;
    if read::<u64>(memory, object) == Some(expected as u64) {
        Ok(())
    } else {
        Err("rocket object vtable unreadable or invalid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const MODULE: usize = 0x1000;
    const BASE: usize = 0x2000;
    const STATE: usize = 0x3000;
    const STATUS: usize = STATE + cstate::BUILDING_STATUS;
    const BUILDINGS: usize = 0x4000;
    const OTHER: usize = 0x5000;
    const ROCKET: usize = 0x6000;
    const OTHER_TEMPLATE: usize = 0x7000;
    const ROCKET_TEMPLATE: usize = 0x8000;
    const PRIMARY: usize = 0x9000;
    const SECONDARY: usize = 0xa000;

    fn put<T: IntoBytes>(bytes: &mut [u8], at: usize, value: T) {
        let value = value.into_bytes();
        bytes[at..at + value.len()].copy_from_slice(&value);
    }

    trait IntoBytes {
        fn into_bytes(self) -> Vec<u8>;
    }
    impl IntoBytes for u64 {
        fn into_bytes(self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
    }
    impl IntoBytes for u32 {
        fn into_bytes(self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
    }
    impl IntoBytes for i32 {
        fn into_bytes(self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
    }
    impl IntoBytes for i16 {
        fn into_bytes(self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
    }
    impl IntoBytes for u8 {
        fn into_bytes(self) -> Vec<u8> {
            vec![self]
        }
    }

    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, data as u64);
        put(bytes, at + 8, count);
        put(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x12000];
        put(&mut bytes, BASE, (MODULE + engine_native_parts::cair_base_layout::VTABLE_RVA.0) as u64);
        put(
            &mut bytes,
            BASE + engine_native_parts::cair_base_layout::HANDLE,
            ((9_u64) << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE),
        );
        put(&mut bytes, BASE + cair_base::KIND, 1_i32);
        put(&mut bytes, BASE + cair_base::STATE, STATE as u64);
        put(&mut bytes, STATE, (MODULE + cstate::VTABLE_RVA.0) as u64);
        put(&mut bytes, STATE + cstate::ID, 25_u32);
        put(
            &mut bytes,
            STATUS,
            (MODULE + cbuilding_status::VTABLE_RVA.0) as u64,
        );
        put(&mut bytes, STATUS + cbuilding_status::ID, 25_u32);
        put(
            &mut bytes,
            STATUS + cbuilding_status::FLAGS,
            cbuilding_status::STATE_FLAG,
        );
        array(
            &mut bytes,
            STATUS + cbuilding_status::ACTIVE_BUILDINGS,
            BUILDINGS,
            2,
        );
        put(&mut bytes, BUILDINGS, OTHER as u64);
        put(&mut bytes, BUILDINGS + 8, ROCKET as u64);
        for (building, template, token, weight) in [
            (OTHER, OTHER_TEMPLATE, 19_u32, 0_i32),
            (ROCKET, ROCKET_TEMPLATE, 29_u32, 1_i32),
        ] {
            put(
                &mut bytes,
                building,
                (MODULE + cbuilding::VTABLE_RVA.0) as u64,
            );
            put(&mut bytes, building + cbuilding::TYPE_ID, token);
            put(
                &mut bytes,
                building + cbuilding::BUILDING_STATUS,
                STATUS as u64,
            );
            put(&mut bytes, building + cbuilding::TEMPLATE, template as u64);
            put(
                &mut bytes,
                template,
                (MODULE + BUILDING_TEMPLATE_VTABLE_RVA.0) as u64,
            );
            put(&mut bytes, template + cbuilding_template::TYPE_ID, token);
            put(&mut bytes, template + MISSILE_SLOT_WEIGHT, weight);
        }
        put(&mut bytes, ROCKET + cbuilding::LEVEL, 2_i16);
        array(&mut bytes, ROCKET + PRIMARY_CONTRIBUTIONS, PRIMARY, 3);
        for (i, node, flag, level) in [
            (0, 0xb000, 1_u8, 4_i16),
            (1, 0xb100, 0_u8, 7_i16),
            (2, 0xb200, 1_u8, 3_i16),
        ] {
            put(&mut bytes, PRIMARY + 8 * i, node as u64);
            put(&mut bytes, node + CONTRIBUTION_FLAG, flag);
            put(&mut bytes, node + CONTRIBUTION_LEVEL, level);
        }
        array(&mut bytes, ROCKET + SECONDARY_CONTRIBUTIONS, SECONDARY, 2);
        for (i, node, flag, level) in [(0, 0xb300, 0_u8, 3_i16), (1, 0xb400, 1_u8, 100_i16)] {
            put(&mut bytes, SECONDARY + 8 * i, node as u64);
            put(&mut bytes, node + CONTRIBUTION_FLAG, flag);
            put(&mut bytes, node + CONTRIBUTION_LEVEL, level);
        }
        bytes
    }

    fn slots(bytes: &[u8]) -> Result<Option<i16>, &'static str> {
        state_rocket_slots(&SliceMemory { base: 0, bytes }, MODULE, BASE)
    }

    #[test]
    fn first_qualifying_state_building_uses_native_aggregate() {
        let bytes = fixture();
        assert_eq!(slots(&bytes), Ok(Some(16)));
    }

    #[test]
    fn aggregate_uses_signed_maxima_and_wrapping_low_word() {
        let mut bytes = fixture();
        put(&mut bytes, ROCKET + cbuilding::LEVEL, i16::MAX);
        put(&mut bytes, 0xb100 + CONTRIBUTION_LEVEL, -4_i16);
        assert_eq!(slots(&bytes), Ok(Some(i16::MIN + 6)));
    }

    #[test]
    fn host_base_has_no_state_operand_and_unqualified_state_is_zero() {
        let mut bytes = fixture();
        put(&mut bytes, BASE + cair_base::STATE, 0_u64);
        put(&mut bytes, BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE, 123_u64);
        assert_eq!(slots(&bytes), Ok(None));
        put(&mut bytes, BASE + cair_base::STATE, STATE as u64);
        put(&mut bytes, ROCKET_TEMPLATE + MISSILE_SLOT_WEIGHT, 0_i32);
        assert_eq!(slots(&bytes), Ok(Some(0)));
    }

    #[test]
    fn invalid_graph_and_short_contribution_fail_closed() {
        let mut bytes = fixture();
        put(&mut bytes, ROCKET + cbuilding::BUILDING_STATUS, 0_u64);
        assert_eq!(
            slots(&bytes),
            Err("rocket building belongs to another status")
        );
        let mut bytes = fixture();
        put(&mut bytes, SECONDARY, 0_u64);
        assert_eq!(
            slots(&bytes),
            Err("rocket contribution vector contains null")
        );
        let mut bytes = fixture();
        array(&mut bytes, ROCKET + SECONDARY_CONTRIBUTIONS, 0x12000, 1);
        assert_eq!(
            slots(&bytes),
            Err("rocket secondary contribution pointer unreadable")
        );
    }
}
