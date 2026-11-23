use super::*;

pub const MODULE_PREDICATE: usize = 0x801;

pub fn air_capacity_kind(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    formation_flags: u64,
) -> Result<u32, &'static str> {
    if formation_flags & 0x80 != 0 {
        return Ok(3);
    }
    let at = pointer
        .checked_add(MODULE_PREDICATE)
        .ok_or("carrier capacity address overflow")?;
    let engagement = at
        .checked_add(1)
        .ok_or("carrier capacity address overflow")?;
    let valid =
        read::<u8>(memory, engagement).ok_or("carrier capacity cache engagement unreadable")?;
    let special = if valid != 0 {
        read::<u8>(memory, at).ok_or("carrier capacity cache unreadable")?
    } else {
        u8::from(has_air_capacity_module(memory, base, pointer)?)
    };
    if special > 1 {
        return Err("carrier capacity cache value unsupported");
    }
    Ok(u32::from(special) + 1)
}

fn has_air_capacity_module(
    memory: &impl Memory,
    base: usize,
    host: usize,
) -> Result<bool, &'static str> {
    let pool = host
        .checked_add(EQUIPMENT_POOL)
        .ok_or("carrier capacity pool address overflow")?;
    let variants = cequipment_variant_pool::entries(memory, base, pool)?;
    let mut found = false;
    for index in 0..variants.count {
        let entry = variants
            .element(index, 16)
            .ok_or("carrier capacity variant address overflow")?;
        let variant = read::<u64>(memory, entry)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p != 0)
            .ok_or("carrier capacity variant unreadable or null")?;
        engine_native_parts::equipment_variant_identity::identity(memory, base, variant)?;
        let modules_at = variant
            .checked_add(cequipment_variant::MODULES + 8)
            .ok_or("carrier capacity modules address overflow")?;
        let modules = engine_native_memory::cpdx_array::Array::read(memory, modules_at, 16)
            .ok_or("carrier capacity modules unreadable or invalid")?;
        for index in 0..modules.count {
            let entry = modules
                .element(index, 16)
                .ok_or("carrier capacity module address overflow")?;
            let module = read::<u64>(memory, entry + 8)
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or("carrier capacity module unreadable or null")?;
            if read::<u64>(memory, module)
                != engine_sites::at(base, engine_native_parts::cequipment_module::VTABLE_RVA).map(|p| p as u64)
            {
                return Err("carrier capacity module identity mismatch");
            }
            let flag = module
                .checked_add(engine_native_parts::cequipment_module::CARRIER_CACHE_FLAG_CANDIDATE)
                .and_then(|at| read::<u8>(memory, at))
                .ok_or("carrier capacity module flag unreadable")?;
            if flag != 0 {
                found = true;
                break;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    const HOST: usize = 0x1000;
    const ENTRIES: usize = 0x2000;
    const VARIANT: usize = 0x3000;
    const MODULES: usize = 0x4000;
    const MODULE: usize = 0x5000;
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        put(
            &mut bytes,
            HOST + EQUIPMENT_POOL,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
        put(
            &mut bytes,
            HOST + EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            ENTRIES as u64,
        );
        put(
            &mut bytes,
            HOST + EQUIPMENT_POOL + cequipment_variant_pool::CAPACITY,
            1 | (1 << 32),
        );
        put(&mut bytes, ENTRIES, VARIANT as u64);
        put(&mut bytes, VARIANT, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            VARIANT + cequipment_variant::TYPE,
            70 | (3 << 32),
        );
        put(
            &mut bytes,
            VARIANT + cequipment_variant::MODULES + 8,
            MODULES as u64,
        );
        put(
            &mut bytes,
            VARIANT + cequipment_variant::MODULES + 16,
            1 | (1 << 32),
        );
        put(&mut bytes, MODULES + 8, MODULE as u64);
        put(
            &mut bytes,
            MODULE,
            engine_native_parts::cequipment_module::VTABLE_RVA.0 as u64,
        );
        bytes[MODULE + engine_native_parts::cequipment_module::CARRIER_CACHE_FLAG_CANDIDATE] = 1;
        bytes
    }
    fn kind(bytes: &[u8], flags: u64) -> Result<u32, &'static str> {
        air_capacity_kind(&SliceMemory { base: 0, bytes }, 0, HOST, flags)
    }
    #[test]
    fn invalid_cache_uses_zero_quantity_modules_and_valid_cache_can_stay_stale() {
        let mut bytes = fixture();
        assert_eq!(kind(&bytes, 0), Ok(2));
        bytes[HOST + MODULE_PREDICATE + 1] = 1;
        assert_eq!(kind(&bytes, 0), Ok(1));
        bytes[HOST + MODULE_PREDICATE] = 1;
        bytes[MODULE + engine_native_parts::cequipment_module::CARRIER_CACHE_FLAG_CANDIDATE] = 0;
        assert_eq!(kind(&bytes, 0), Ok(2));
        bytes[HOST + MODULE_PREDICATE + 1] = 0;
        assert_eq!(kind(&bytes, 0), Ok(1));
    }
    #[test]
    fn mega_capacity_demands_no_cache_and_bad_variants_refuse_only_the_predicate() {
        assert_eq!(kind(&[], 0x80), Ok(3));
        let mut bytes = fixture();
        put(&mut bytes, VARIANT, 0);
        assert!(kind(&bytes, 0).is_err());
        bytes[HOST + MODULE_PREDICATE + 1] = 1;
        bytes[HOST + MODULE_PREDICATE] = 2;
        assert_eq!(
            kind(&bytes, 0),
            Err("carrier capacity cache value unsupported")
        );
    }

    #[test]
    fn a_positive_module_does_not_skip_later_stored_variants() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            HOST + EQUIPMENT_POOL + cequipment_variant_pool::CAPACITY,
            2 | (2 << 32),
        );
        put(&mut bytes, ENTRIES + 16, 0x3500);
        assert!(kind(&bytes, 0).is_err());
    }
}
