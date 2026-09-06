use super::*;
use compact_contract::division_routes::MAX_ROUTE_PROVINCES;

const ORDINARY_ROUTE: usize = 0x1f0;
const ALTERNATE_ROUTE: usize = 0x210;

#[derive(PartialEq, Eq)]
struct RouteRead {
    identity: [u64; 4],
    header: [u64; 3],
    provinces: Vec<u32>,
}

fn division_identity<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<[u64; 4]> {
    let identity = [
        mem.u64(body)?,
        mem.u32(body.checked_add(DIVISION_OFF_ID_TYPE)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_ID_ID)?)? as u64,
        mem.u32(body.checked_add(DIVISION_OFF_LOCATION)?)? as u64,
    ];
    if identity[0] != module.checked_add(DIVISION_VTABLE_RVA)? as u64
        || !is_compact_division_body_type(identity[1] as u32)
        || identity[2] != id as u64
    {
        return None;
    }
    Some(identity)
}

#[derive(PartialEq, Eq)]
struct TemplateRead {
    identity: [u64; 4],
    override_pointer: usize,
    selected_pointer: usize,
    template: Option<u32>,
}

fn template_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<TemplateRead> {
    let identity = division_identity(mem, module, body, id)?;
    // The collector body is CDivision + 0x10; the current template is CDivision + 0x3c8.
    let override_pointer = mem.u64(body.checked_add(0x3b8)?)? as usize;
    let selected_pointer = if override_pointer == 0 {
        mem.u64(body.checked_add(DIVISION_OFF_TEMPLATE)?)? as usize
    } else {
        override_pointer
    };
    let template = if selected_pointer == 0 {
        None
    } else {
        if !is_map_control_heap_pointer(selected_pointer)
            || mem.u64(selected_pointer)?
                != module.checked_add(DIVISION_TEMPLATE_VTABLE_RVA)? as u64
            || mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_TYPE)?)?
                != DIVISION_TEMPLATE_TYPE
        {
            return None;
        }
        let template_id = mem.u32(selected_pointer.checked_add(DIVISION_TEMPLATE_OFF_ID)?)?;
        if template_id == 0 || template_id > MAX_DIVISION_TEMPLATE_ID {
            return None;
        }
        Some(template_id)
    };
    Some(TemplateRead {
        identity,
        override_pointer,
        selected_pointer,
        template,
    })
}

#[cfg(test)]
fn stable_template<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<Option<u32>> {
    let first = template_once(mem, module, body, id)?;
    between();
    let second = template_once(mem, module, body, id)?;
    (first == second).then_some(first.template)
}

#[derive(PartialEq, Eq)]
struct PreferenceRead {
    string_custody: [usize; 3],
    exclusions: Option<ExclusionRead>,
    preference: compact_contract::division_routes::TemplateModelPreference,
}

#[derive(PartialEq, Eq)]
struct ExclusionRead {
    header: [u64; 3],
    pointers: Vec<u64>,
    ids: Vec<Option<u32>>,
}

fn exclusions_once<M: DecodeMemory + ?Sized>(mem: &M, module: usize, template: usize) -> Option<ExclusionRead> {
    let at = template.checked_add(0x140)?;
    let data = mem.u64(at)?;
    let capacity = mem.u32(at.checked_add(8)?)?;
    let count = mem.u32(at.checked_add(12)?)?;
    if count > capacity || count as usize > compact_contract::division_routes::MAX_TEMPLATE_EXCLUSIONS {
        return None;
    }
    let mut pointers = Vec::with_capacity(count as usize);
    let mut ids = Vec::with_capacity(count as usize);
    if count != 0 {
        if !is_map_control_heap_pointer(data as usize) { return None; }
        let bytes = mem.bytes(data as usize, (count as usize).checked_mul(8)?)?;
        for entry in bytes.chunks_exact(8) {
            let pointer = u64::from_le_bytes(entry.try_into().ok()?);
            pointers.push(pointer);
            if pointer == 0 { ids.push(None); continue; }
            let pointer = pointer as usize;
            if !is_map_control_heap_pointer(pointer)
                || mem.u64(pointer)? != module.checked_add(DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA)? as u64
                || mem.u32(pointer.checked_add(DIVISION_EQUIPMENT_VARIANT_OFF_TYPE)?)? != DIVISION_EQUIPMENT_VARIANT_TYPE {
                return None;
            }
            let id = mem.u32(pointer.checked_add(DIVISION_EQUIPMENT_VARIANT_OFF_ID)?)?;
            if id == 0 || id > MAX_DIVISION_EQUIPMENT_VARIANT_ID { return None; }
            ids.push(Some(id));
        }
    }
    Some(ExclusionRead { header: [data, capacity as u64, count as u64], pointers, ids })
}

fn preference_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    selected: &TemplateRead,
) -> Option<PreferenceRead> {
    let template_id = selected.template?;
    let pointer = selected.selected_pointer;
    let entity = read_msvc_string_snapshot_once_in(
        mem,
        pointer.checked_add(0x210)?,
        compact_contract::division_routes::MAX_MODEL_ENTITY_BYTES,
    )?;
    let present = mem.scalar::<1>(pointer.checked_add(0x208)?)?[0];
    let category = if present == 0 {
        None
    } else {
        Some(mem.u64(pointer.checked_add(0x200)?)?)
    };
    Some(PreferenceRead {
        string_custody: [entity.size, entity.cap, entity.chars_addr],
        exclusions: exclusions_once(mem, module, pointer),
        preference: compact_contract::division_routes::TemplateModelPreference {
            template_id,
            entity: String::from_utf8(entity.bytes).ok()?,
            category,
            excluded_variants: None,
        },
    })
}

fn stable_template_inputs<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> (
    Option<Option<u32>>,
    Option<compact_contract::division_routes::TemplateModelPreference>,
) {
    let Some(first) = template_once(mem, module, body, id) else {
        return (None, None);
    };
    let preference = preference_once(mem, module, &first);
    between();
    let Some(second) = template_once(mem, module, body, id) else {
        return (None, None);
    };
    if first != second {
        return (None, None);
    }
    let after = preference_once(mem, module, &second);
    let preference = match (preference, after) {
        (Some(mut before), Some(after)) if before.string_custody == after.string_custody
            && before.preference == after.preference => {
                if before.exclusions == after.exclusions {
                    before.preference.excluded_variants = before.exclusions.map(|value| value.ids);
                }
                Some(before.preference)
            }
        _ => None,
    };
    (Some(first.template), preference)
}

pub(super) fn current_template_inputs(
    module: usize,
    body: usize,
    id: u32,
) -> (
    Option<Option<u32>>,
    Option<compact_contract::division_routes::TemplateModelPreference>,
) {
    stable_template_inputs(&LIVE_MEMORY, module, body, id, || {})
}

#[derive(PartialEq, Eq)]
struct HeadRead {
    identity: [u64; 4],
    pointer: usize,
    province: Option<u32>,
}

fn head_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<HeadRead> {
    let identity = division_identity(mem, module, body, id)?;
    let pointer = mem.u64(body.checked_add(0x1e0)?)? as usize;
    let province = if pointer == 0 {
        None
    } else {
        if !is_map_control_heap_pointer(pointer)
            || mem.u64(pointer)? != module.checked_add(CPROVINCE_VTABLE_RVA)? as u64
        {
            return None;
        }
        let province = mem.u32(pointer.checked_add(CPROVINCE_OFF_ID)?)?;
        if !valid_division_province_id(province) {
            return None;
        }
        Some(province)
    };
    Some(HeadRead {
        identity,
        pointer,
        province,
    })
}

#[cfg(test)]
fn stable_head<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<Option<u32>> {
    let first = head_once(mem, module, body, id)?;
    between();
    let second = head_once(mem, module, body, id)?;
    (first == second).then_some(first.province)
}

fn route_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    offset: usize,
) -> Option<RouteRead> {
    let identity = division_identity(mem, module, body, id)?;
    let at = body.checked_add(offset)?;
    let data = mem.u64(at)?;
    let capacity = mem.u32(at.checked_add(8)?)?;
    let count = mem.u32(at.checked_add(12)?)?;
    if count > capacity || count as usize > MAX_ROUTE_PROVINCES {
        return None;
    }
    let mut provinces = Vec::new();
    if count != 0 {
        if !is_map_control_heap_pointer(data as usize) {
            return None;
        }
        let bytes = mem.bytes(data as usize, (count as usize).checked_mul(4)?)?;
        for bytes in bytes.chunks_exact(4) {
            let province = u32::from_le_bytes(bytes.try_into().ok()?);
            if !valid_division_province_id(province) {
                return None;
            }
            provinces.push(province);
        }
    }
    Some(RouteRead {
        identity,
        header: [data, capacity as u64, count as u64],
        provinces,
    })
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct MotionInputs {
    pub ordinary_route: Option<Vec<u32>>,
    pub alternate_route: Option<Vec<u32>>,
    pub route_head: Option<Option<u32>>,
    pub scalars: compact_contract::division_routes::MovementScalars,
}

#[derive(PartialEq, Eq)]
struct CombatRead {
    header: [u64; 3],
    pointer: u64,
    identity: Option<compact_contract::division_routes::CombatIdentity>,
    vtables: [u64; 2],
}

fn first_combat_once<M: DecodeMemory + ?Sized>(mem: &M, module: usize, body: usize) -> Option<CombatRead> {
    use compact_contract::division_routes::{CombatClass, CombatIdentity};
    let data = mem.u64(body.checked_add(0x198)?)?;
    let capacity = mem.u32(body.checked_add(0x1a0)?)?;
    let count = mem.u32(body.checked_add(DIVISION_OFF_COMBAT_REF_COUNT)?)?;
    if count > capacity || count > MAX_DIVISION_COMBAT_REF_COUNT { return None; }
    let header = [data, capacity as u64, count as u64];
    if count == 0 { return Some(CombatRead { header, pointer: 0, identity: None, vtables: [0; 2] }); }
    if !is_map_control_heap_pointer(data as usize) { return None; }
    let pointer = mem.u64(data as usize)?;
    if !is_map_control_heap_pointer(pointer as usize) { return None; }
    let primary = pointer as usize;
    let combat = primary.checked_add(LAND_COMBAT_MANAGER_BODY_DELTA)?;
    let vtables = [mem.u64(primary)?, mem.u64(combat)?];
    let class = if vtables == [(module + LAND_COMBAT_PRIMARY_VTABLE_RVA) as u64, (module + LAND_COMBAT_BODY_VTABLE_RVA) as u64] {
        CombatClass::Land
    } else if vtables == [(module + LAND_BORDER_WAR_COMBAT_PRIMARY_VTABLE_RVA) as u64, (module + LAND_BORDER_WAR_COMBAT_BODY_VTABLE_RVA) as u64] {
        CombatClass::BorderWar
    } else { return None };
    let kind = mem.u32(combat.checked_add(LAND_COMBAT_OFF_TYPE)?)?;
    let id = mem.u32(combat.checked_add(LAND_COMBAT_OFF_ID)?)?;
    if id == 0 || id > i32::MAX as u32 || (class == CombatClass::Land && kind != LAND_COMBAT_TYPE) { return None; }
    Some(CombatRead { header, pointer, identity: Some(CombatIdentity { class, kind, id }), vtables })
}

#[derive(PartialEq, Eq)]
struct MotionRead {
    identity: [u64; 4],
    ordinary: Option<RouteRead>,
    alternate: Option<RouteRead>,
    head: Option<HeadRead>,
    numerator: Option<i64>,
    transfer: Option<u64>,
    target: Option<u64>,
    retreat: Option<u8>,
    combat: Option<CombatRead>,
}

fn motion_once<M: DecodeMemory + ?Sized>(mem: &M, module: usize, body: usize, id: u32,
    location: u32, combat_count: u32) -> Option<MotionRead> {
    let identity = division_identity(mem, module, body, id)?;
    if identity[3] != location as u64 || mem.u32(body.checked_add(DIVISION_OFF_COMBAT_REF_COUNT)?)? != combat_count { return None; }
    let value = MotionRead {
        identity,
        ordinary: route_once(mem, module, body, id, ORDINARY_ROUTE),
        alternate: route_once(mem, module, body, id, ALTERNATE_ROUTE),
        head: head_once(mem, module, body, id),
        numerator: mem.scalar::<8>(body.checked_add(0x230)?).map(i64::from_le_bytes),
        transfer: mem.u64(body.checked_add(0x2e8)?),
        target: mem.u64(body.checked_add(0x1d8)?),
        retreat: mem.scalar::<1>(body.checked_add(0x23c)?).map(|bytes| bytes[0]),
        combat: first_combat_once(mem, module, body),
    };
    if division_identity(mem, module, body, id) != Some(identity)
        || mem.u32(body.checked_add(DIVISION_OFF_COMBAT_REF_COUNT)?)? != combat_count
        || value.combat.as_ref().is_some_and(|read| read.header[2] != combat_count as u64) {
        return None;
    }
    Some(value)
}

fn stable_motion_inputs<M: DecodeMemory + ?Sized>(mem: &M, module: usize, body: usize, id: u32,
    location: u32, combat_count: u32, between: impl FnOnce()) -> MotionInputs {
    let Some(first) = motion_once(mem, module, body, id, location, combat_count) else { return MotionInputs::default() };
    between();
    let Some(second) = motion_once(mem, module, body, id, location, combat_count) else { return MotionInputs::default() };
    if first.identity != second.identity { return MotionInputs::default(); }
    let stable = |before, after| if before == after { before } else { None };
    MotionInputs {
        ordinary_route: (first.ordinary == second.ordinary).then(|| first.ordinary.map(|value| value.provinces)).flatten(),
        alternate_route: (first.alternate == second.alternate).then(|| first.alternate.map(|value| value.provinces)).flatten(),
        route_head: (first.head == second.head).then(|| first.head.map(|value| value.province)).flatten(),
        scalars: compact_contract::division_routes::MovementScalars {
            numerator_raw: if first.numerator == second.numerator { first.numerator } else { None },
            naval_transfer_membership: stable(first.transfer, second.transfer).map(|pointer| pointer != 0),
            movement_target_present: stable(first.target, second.target).map(|pointer| pointer != 0),
            retreat_raw: if first.retreat == second.retreat { first.retreat } else { None },
            first_combat: (first.combat == second.combat).then(|| first.combat.map(|value| value.identity)).flatten(),
        },
    }
}

pub(super) fn current_motion_inputs(module: usize, body: usize, id: u32, location: u32, combat_count: u32) -> MotionInputs {
    let Some(hour) = read_research_game_hour_raw(module) else { return MotionInputs::default() };
    let value = stable_motion_inputs(&LIVE_MEMORY, module, body, id, location, combat_count, || {});
    if read_research_game_hour_raw(module) != Some(hour) { return MotionInputs::default(); }
    value
}

#[derive(PartialEq, Eq)]
struct AcclimatizationRead {
    identity: [u64; 4],
    custody: Vec<u64>,
    value: compact_contract::division_routes::Acclimatization,
}

fn acclimatization_once<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
) -> Option<AcclimatizationRead> {
    use compact_contract::division_routes::{
        Acclimatization, AcclimatizationCategory, MAX_ACCLIMATIZATION_CATEGORIES,
        MAX_MODEL_ENTITY_BYTES,
    };
    let identity = division_identity(mem, module, body, id)?;
    let at = body.checked_add(0x4a8)?;
    if mem.u64(at)? != module.checked_add(0x27c7b30)? as u64 {
        return None;
    }
    let registry = mem.u64(module.checked_add(0x3318080)?)? as usize;
    if !is_map_control_heap_pointer(registry) {
        return None;
    }
    let registered = mem.u64(registry)? as usize;
    let registered_capacity = mem.u32(registry.checked_add(8)?)?;
    let registered_count = mem.u32(registry.checked_add(12)?)?;
    let data = mem.u64(at.checked_add(8)?)? as usize;
    let capacity = mem.u32(at.checked_add(0x10)?)?;
    let count = mem.u32(at.checked_add(0x14)?)?;
    if count != registered_count
        || count > capacity
        || registered_count > registered_capacity
        || count as usize > MAX_ACCLIMATIZATION_CATEGORIES
    {
        return None;
    }
    let maximum_raw = mem.u64(at.checked_add(0x38)?)? as i64;
    let mut custody = vec![
        registry as u64,
        registered as u64,
        registered_capacity as u64,
        registered_count as u64,
        data as u64,
        capacity as u64,
        count as u64,
    ];
    let mut categories = Vec::with_capacity(count as usize);
    if count != 0 {
        if !is_map_control_heap_pointer(registered) || !is_map_control_heap_pointer(data) {
            return None;
        }
        let pointers = mem.bytes(registered, (count as usize).checked_mul(8)?)?;
        let entries = mem.bytes(data, (count as usize).checked_mul(16)?)?;
        let mut seen = BTreeSet::new();
        for (registered, entry) in pointers.chunks_exact(8).zip(entries.chunks_exact(16)) {
            let pointer = u64::from_le_bytes(entry[..8].try_into().ok()?) as usize;
            if pointer as u64 != u64::from_le_bytes(registered.try_into().ok()?)
                || !is_map_control_heap_pointer(pointer)
            {
                return None;
            }
            let name = read_msvc_string_snapshot_once_in(mem, pointer, MAX_MODEL_ENTITY_BYTES)?;
            let token = mem.u32(pointer.checked_add(0x20)?)?;
            let category = String::from_utf8(name.bytes).ok()?;
            if category.is_empty() || !seen.insert(category.clone()) {
                return None;
            }
            custody.extend([
                pointer as u64,
                name.size as u64,
                name.cap as u64,
                name.chars_addr as u64,
                token as u64,
            ]);
            categories.push(AcclimatizationCategory {
                category,
                weight_raw: i64::from_le_bytes(entry[8..].try_into().ok()?),
            });
        }
    }
    Some(AcclimatizationRead {
        identity,
        custody,
        value: Acclimatization {
            maximum_raw,
            categories,
        },
    })
}

fn stable_acclimatization<M: DecodeMemory + ?Sized>(
    mem: &M,
    module: usize,
    body: usize,
    id: u32,
    between: impl FnOnce(),
) -> Option<compact_contract::division_routes::Acclimatization> {
    let first = acclimatization_once(mem, module, body, id)?;
    between();
    let second = acclimatization_once(mem, module, body, id)?;
    (first == second).then_some(first.value)
}

pub(super) fn acclimatization(
    module: usize,
    body: usize,
    id: u32,
) -> Option<compact_contract::division_routes::Acclimatization> {
    stable_acclimatization(&LIVE_MEMORY, module, body, id, || {})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<usize, u8>>);
    impl Memory {
        fn put(&self, at: usize, bytes: &[u8]) {
            self.0
                .borrow_mut()
                .extend(bytes.iter().enumerate().map(|(i, b)| (at + i, *b)));
        }
        fn put32(&self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn put64(&self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn vector(&self, body: usize, offset: usize, data: usize, provinces: &[u32]) {
            self.put64(body + offset, data as u64);
            self.put32(body + offset + 8, provinces.len() as u32);
            self.put32(body + offset + 12, provinces.len() as u32);
            for (index, province) in provinces.iter().enumerate() {
                self.put32(data + index * 4, *province);
            }
        }
    }
    impl DecodeMemory for Memory {
        fn scalar<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
            self.bytes(at, N)?.try_into().ok()
        }
        fn bytes_with_reason(&self, at: usize, len: usize) -> Result<Vec<u8>, GuardedReadRefusal> {
            (at..at + len)
                .map(|key| {
                    self.0
                        .borrow()
                        .get(&key)
                        .copied()
                        .ok_or(GuardedReadRefusal::OutsideConfirmedRanges)
                })
                .collect()
        }
        fn fill(&self, at: usize, buf: &mut [u8]) -> bool {
            let Some(bytes) = self.bytes(at, buf.len()) else {
                return false;
            };
            buf.copy_from_slice(&bytes);
            true
        }
    }
    fn fixture() -> (Memory, usize, usize) {
        let mem = Memory::default();
        let module = 0x140000000;
        let body = HEAP_LO + 0x10000;
        mem.put64(body, (module + DIVISION_VTABLE_RVA) as u64);
        mem.put32(body + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        mem.put32(body + DIVISION_OFF_ID_ID, 7);
        mem.put32(body + DIVISION_OFF_LOCATION, 12);
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[19, 17, 19]);
        mem.vector(body, ALTERNATE_ROUTE, 0, &[]);
        mem.put64(body + 0x198, 0); mem.put32(body + 0x1a0, 0); mem.put32(body + 0x1a4, 0);
        mem.put64(body + 0x230, 0); mem.put64(body + 0x2e8, 0); mem.put64(body + 0x1d8, 0);
        mem.put(body + 0x23c, &[0]);
        (mem, module, body)
    }
    #[test]
    fn route_head_uses_province_pointer_and_preserves_unknown_null_and_changed() {
        let (mem, module, body) = fixture();
        let province = body + 0x50000;
        assert_eq!(stable_head(&mem, module, body, 7, || {}), None);
        mem.put64(body + 0x1e0, 0);
        assert_eq!(stable_head(&mem, module, body, 7, || {}), Some(None));
        mem.put64(body + 0x1e0, province as u64);
        mem.put64(province, (module + CPROVINCE_VTABLE_RVA) as u64);
        mem.put32(province + CPROVINCE_OFF_ID, 23);
        mem.put32(body + DIVISION_OFF_LOCATION, 12);
        assert_eq!(stable_head(&mem, module, body, 7, || {}), Some(Some(23)));
        let input = stable_motion_inputs(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(input.ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(input.route_head, Some(Some(23)));
        let changed = stable_motion_inputs(&mem, module, body, 7, 12, 0, || mem.put32(body + 0x10000, 18));
        assert_eq!(changed.ordinary_route, None);
        assert_eq!(changed.route_head, Some(Some(23)));
        mem.put32(body + 0x10000, 19);
        assert_eq!(
            stable_head(&mem, module, body, 7, || mem
                .put32(province + CPROVINCE_OFF_ID, 24)),
            None
        );
        mem.put64(province, 0);
        assert_eq!(stable_head(&mem, module, body, 7, || {}), None);
        mem.put64(province, (module + CPROVINCE_VTABLE_RVA) as u64);
        mem.put32(province + CPROVINCE_OFF_ID, 0);
        assert_eq!(stable_head(&mem, module, body, 7, || {}), None);
        mem.put32(province + CPROVINCE_OFF_ID, 23);
        assert_eq!(
            stable_head(&mem, module, body, 7, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)),
            None
        );
    }

    #[test]
    fn template_preferences_bind_current_identity_and_preserve_empty_optional_and_torn_values() {
        let (mem, module, body) = fixture();
        let template = body + 0x30000;
        mem.put64(body + 0x3b8, template as u64);
        mem.put64(template, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
        mem.put32(
            template + DIVISION_TEMPLATE_OFF_TYPE,
            DIVISION_TEMPLATE_TYPE,
        );
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        mem.put(template + 0x210, b"tank_entity\0");
        mem.put64(template + 0x220, 11);
        mem.put64(template + 0x228, 15);
        mem.put(template + 0x208, &[0]);
        let (id, preference) = stable_template_inputs(&mem, module, body, 7, || {});
        assert_eq!(id, Some(Some(73)));
        let preference = preference.unwrap();
        assert_eq!(preference.template_id, 73);
        assert_eq!(preference.entity, "tank_entity");
        assert_eq!(preference.category, None);
        mem.put(template + 0x208, &[1]);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {}),
            (Some(Some(73)), None)
        );
        mem.put64(template + 0x200, 0);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {})
                .1
                .unwrap()
                .category,
            Some(0)
        );
        mem.put(template + 0x210, b"\0");
        mem.put64(template + 0x220, 0);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || {})
                .1
                .unwrap()
                .entity,
            ""
        );
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem.put64(template + 0x200, 9)),
            (Some(Some(73)), None)
        );
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem
                .put32(template + DIVISION_TEMPLATE_OFF_ID, 74)),
            (None, None)
        );
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        assert_eq!(
            stable_template_inputs(&mem, module, body, 7, || mem.put64(body + 0x3b8, 0)),
            (None, None)
        );
    }

    #[test]
    fn template_exclusions_preserve_unknown_empty_null_order_and_torn_variant_identity() {
        let (mem, module, body) = fixture();
        let template = body + 0x30000;
        let list = body + 0x40000;
        let variant = body + 0x50000;
        mem.put64(body + 0x3b8, template as u64);
        mem.put64(template, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
        mem.put32(template + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
        mem.put32(template + DIVISION_TEMPLATE_OFF_ID, 73);
        mem.put(template + 0x210, &[0]);
        mem.put64(template + 0x220, 0);
        mem.put64(template + 0x228, 15);
        mem.put(template + 0x208, &[1]);
        mem.put64(template + 0x200, u64::MAX);
        let read = || stable_template_inputs(&mem, module, body, 7, || {}).1.unwrap();
        assert_eq!(read().excluded_variants, None);
        mem.put64(template + 0x140, 0);
        mem.put32(template + 0x148, 0);
        mem.put32(template + 0x14c, 0);
        assert_eq!(read().excluded_variants, Some(vec![]));
        mem.put64(template + 0x140, list as u64);
        mem.put32(template + 0x148, 3);
        mem.put32(template + 0x14c, 3);
        mem.put64(list, variant as u64);
        mem.put64(list + 8, 0);
        mem.put64(list + 16, variant as u64);
        mem.put64(variant, (module + DIVISION_EQUIPMENT_VARIANT_VTABLE_RVA) as u64);
        mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_TYPE, DIVISION_EQUIPMENT_VARIANT_TYPE);
        mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_ID, 11);
        assert_eq!(read().excluded_variants, Some(vec![Some(11), None, Some(11)]));
        let torn = stable_template_inputs(&mem, module, body, 7, || mem.put32(variant + DIVISION_EQUIPMENT_VARIANT_OFF_ID, 12)).1.unwrap();
        assert_eq!(torn.excluded_variants, None);
        assert_eq!(torn.category, Some(u64::MAX));
        assert!(torn.entity.is_empty());
        mem.put32(template + 0x14c, 4);
        assert_eq!(read().excluded_variants, None);
        mem.put32(template + 0x14c, 3);
        mem.put64(variant, 0);
        assert_eq!(read().excluded_variants, None);
    }

    #[test]
    fn acclimatization_census_preserves_registered_order_raw_values_empty_and_torn_refusal() {
        let (mem, module, body) = fixture();
        let at = body + 0x4a8;
        let registry = body + 0x60000;
        let registered = body + 0x70000;
        let data = body + 0x80000;
        let cold = body + 0x90000;
        let hot = body + 0xa0000;
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put64(at, (module + 0x27c7b30) as u64);
        mem.put64(module + 0x3318080, registry as u64);
        mem.put64(registry, 0);
        mem.put32(registry + 8, 0);
        mem.put32(registry + 12, 0);
        mem.put64(at + 8, 0);
        mem.put32(at + 0x10, 0);
        mem.put32(at + 0x14, 0);
        mem.put64(at + 0x38, 10_000_000);
        let empty = stable_acclimatization(&mem, module, body, 7, || {}).unwrap();
        assert_eq!(empty.maximum_raw, 10_000_000);
        assert!(empty.categories.is_empty());
        mem.put64(registry, registered as u64);
        mem.put32(registry + 8, 2);
        mem.put32(registry + 12, 2);
        mem.put64(at + 8, data as u64);
        mem.put32(at + 0x10, 2);
        mem.put32(at + 0x14, 2);
        for (index, (pointer, name, weight)) in [(cold, "cold", -1i64), (hot, "hot", 123_456i64)]
            .into_iter()
            .enumerate()
        {
            mem.put64(registered + index * 8, pointer as u64);
            mem.put64(data + index * 16, pointer as u64);
            mem.put64(data + index * 16 + 8, weight as u64);
            mem.put(pointer, format!("{name}\0").as_bytes());
            mem.put64(pointer + 0x10, name.len() as u64);
            mem.put64(pointer + 0x18, 15);
            mem.put32(pointer + 0x20, index as u32 + 100);
        }
        let value = stable_acclimatization(&mem, module, body, 7, || {}).unwrap();
        assert_eq!(
            value
                .categories
                .iter()
                .map(|v| (v.category.as_str(), v.weight_raw))
                .collect::<Vec<_>>(),
            vec![("cold", -1), ("hot", 123_456)]
        );
        assert_eq!(
            stable_acclimatization(&mem, module, body, 7, || mem.put64(data + 8, 0)),
            None
        );
        assert_eq!(
            stable_acclimatization(&mem, module, body, 7, || mem.put(cold, b"warm\0")),
            None
        );
        mem.put64(data, hot as u64);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put64(data, cold as u64);
        mem.put32(at + 0x14, 1);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
        mem.put32(at + 0x14, 2);
        mem.put64(at, 0);
        assert_eq!(stable_acclimatization(&mem, module, body, 7, || {}), None);
    }

    #[test]
    fn current_template_uses_only_witnessed_null_fallback_and_preserves_absence() {
        let (mem, module, body) = fixture();
        let standard = body + 0x30000;
        let current = body + 0x40000;
        for (at, id) in [(standard, 61), (current, 72)] {
            mem.put64(at, (module + DIVISION_TEMPLATE_VTABLE_RVA) as u64);
            mem.put32(at + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
            mem.put32(at + DIVISION_TEMPLATE_OFF_ID, id);
        }
        mem.put64(body + DIVISION_OFF_TEMPLATE, standard as u64);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put64(body + 0x3b8, 0);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(61))
        );
        mem.put64(body + 0x3b8, current as u64);
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.0.borrow_mut().remove(&(body + DIVISION_OFF_TEMPLATE));
        assert_eq!(
            stable_template(&mem, module, body, 7, || {}),
            Some(Some(72))
        );
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, 70);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), None);
        mem.put32(current + DIVISION_TEMPLATE_OFF_TYPE, DIVISION_TEMPLATE_TYPE);
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(current + DIVISION_TEMPLATE_OFF_ID, 73)),
            None
        );
        mem.put64(body + 0x3b8, 0);
        mem.put64(body + DIVISION_OFF_TEMPLATE, 0);
        assert_eq!(stable_template(&mem, module, body, 7, || {}), Some(None));
        assert_eq!(
            stable_template(&mem, module, body, 7, || mem
                .put32(body + DIVISION_OFF_ID_ID, 8)),
            None
        );
    }

    #[test]
    fn both_routes_preserve_native_order_and_independent_availability() {
        let (mem, module, body) = fixture();
        let read = || stable_motion_inputs(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(read().ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(read().alternate_route, Some(vec![]));
        mem.vector(body, ALTERNATE_ROUTE, body + 0x20000, &[22, 21]);
        assert_eq!(read().ordinary_route, Some(vec![19, 17, 19]));
        assert_eq!(read().alternate_route, Some(vec![22, 21]));
        mem.0.borrow_mut().remove(&(body + ORDINARY_ROUTE));
        assert_eq!(read().ordinary_route, None);
        assert_eq!(read().alternate_route, Some(vec![22, 21]));
        mem.0.borrow_mut().remove(&(body + ALTERNATE_ROUTE + 12));
        assert_eq!(read().alternate_route, None);
    }

    #[test]
    fn grouped_motion_preserves_zero_signed_width_and_nullable_pointer_changes() {
        let (mem, module, body) = fixture();
        let read = || stable_motion_inputs(&mem, module, body, 7, 12, 0, || {});
        assert_eq!(read().scalars.numerator_raw, Some(0));
        assert_eq!(read().scalars.naval_transfer_membership, Some(false));
        assert_eq!(read().scalars.movement_target_present, Some(false));
        assert_eq!(read().scalars.retreat_raw, Some(0));
        assert_eq!(read().scalars.first_combat, Some(None));
        for raw in [i64::MIN, i64::MAX, -1, 0x1_0000_0000] {
            mem.put64(body + 0x230, raw as u64);
            assert_eq!(read().scalars.numerator_raw, Some(raw));
        }
        mem.put(body + 0x23c, &[255]);
        assert_eq!(read().scalars.retreat_raw, Some(255));
        for offset in [0x2e8, 0x1d8] {
            mem.put64(body + offset, (body + 0x90000) as u64);
        }
        assert_eq!(read().scalars.naval_transfer_membership, Some(true));
        assert_eq!(read().scalars.movement_target_present, Some(true));
        let torn = stable_motion_inputs(&mem, module, body, 7, 12, 0, || {
            mem.put64(body + 0x2e8, (body + 0xa0000) as u64);
            mem.put64(body + 0x1d8, (body + 0xb0000) as u64);
            mem.put64(body + 0x230, 123);
        });
        assert_eq!(torn.scalars.naval_transfer_membership, None);
        assert_eq!(torn.scalars.movement_target_present, None);
        assert_eq!(torn.scalars.numerator_raw, None);
        assert_eq!(torn.scalars.retreat_raw, Some(255));
        mem.0.borrow_mut().remove(&(body + 0x230 + 7));
        assert_eq!(read().scalars.numerator_raw, None);
        let changed = stable_motion_inputs(&mem, module, body, 7, 12, 0, || mem.put32(body + DIVISION_OFF_ID_ID, 8));
        assert_eq!(changed, MotionInputs::default());
    }

    #[test]
    fn first_combat_preserves_native_first_identity_and_refuses_reorder_or_reuse() {
        use compact_contract::division_routes::{CombatClass, CombatIdentity};
        let (mem, module, body) = fixture();
        let data = body + 0x60000;
        let first = body + 0x70000;
        let second = body + 0x80000;
        mem.put64(body + 0x198, data as u64); mem.put32(body + 0x1a0, 2); mem.put32(body + 0x1a4, 2);
        mem.put64(data, first as u64); mem.put64(data + 8, second as u64);
        for (pointer, id) in [(first, 91), (second, 92)] {
            mem.put64(pointer, (module + LAND_COMBAT_PRIMARY_VTABLE_RVA) as u64);
            mem.put64(pointer + 0x10, (module + LAND_COMBAT_BODY_VTABLE_RVA) as u64);
            mem.put32(pointer + 0x18, LAND_COMBAT_TYPE); mem.put32(pointer + 0x1c, id);
        }
        let expected = Some(Some(CombatIdentity { class: CombatClass::Land, kind: 62, id: 91 }));
        assert_eq!(stable_motion_inputs(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat, expected);
        let changed = stable_motion_inputs(&mem, module, body, 7, 12, 2, || {
            mem.put64(data, second as u64); mem.put64(data + 8, first as u64);
        });
        assert_eq!(changed.scalars.first_combat, None);
        assert_eq!(stable_motion_inputs(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat.unwrap().unwrap().id, 92);
        let changed = stable_motion_inputs(&mem, module, body, 7, 12, 2, || mem.put32(second + 0x1c, 93));
        assert_eq!(changed.scalars.first_combat, None);
        mem.put64(second, 0);
        assert_eq!(stable_motion_inputs(&mem, module, body, 7, 12, 2, || {}).scalars.first_combat, None);
    }

    #[test]
    fn both_route_pools_reject_invalid_or_changed_bytes_without_dropping_other_scalars() {
        let (mem, module, body) = fixture();
        let changed = stable_motion_inputs(&mem, module, body, 7, 12, 0, || mem.put32(body + 0x10000, 20));
        assert_eq!(changed.ordinary_route, None);
        assert_eq!(changed.scalars.numerator_raw, Some(0));
        mem.vector(body, ORDINARY_ROUTE, body + 0x10000, &[0]);
        assert_eq!(stable_motion_inputs(&mem, module, body, 7, 12, 0, || {}).ordinary_route, None);
        mem.put32(body + ALTERNATE_ROUTE + 12, MAX_ROUTE_PROVINCES as u32 + 1);
        assert_eq!(stable_motion_inputs(&mem, module, body, 7, 12, 0, || {}).alternate_route, None);
    }
}
