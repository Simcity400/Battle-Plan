use super::*;
use crate::raw::CountrySlots;


const M_P_LINES: u32 = 0x01;
const M_P_STOCK: u32 = 0x02;
const M_P_LIC: u32 = 0x04;
const M_P_REC: u32 = 0x80;
const M_P_PROD_MGR: u32 = 0x87;
const M_P_DEPLOY: u32 = 0x78;
const M_P_CENSUS: u32 = 0xFF;

#[derive(Debug, Default)]
pub struct ProductionRestJob {
    pub log: RawLog<8>,
    pub game_state: usize,
    pub(super) census: Vec<ProductionRestCountry>,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProductionRestCountry {
    pointer: usize,
}

impl ProductionRestJob {
    pub fn new() -> Self {
        let mut log = RawLog::new(0, 0);
        log.initialize_read_plan(0, 0, 0);
        let mut job = Self {
            log,
            game_state: 0,
            census: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        engine_allocation::advisory(|| {
            self.log.grow();
            let target = self
                .census
                .len()
                .saturating_add(1)
                .max(crate::country_scale::census_rows(0));
            let _ = crate::allocation::reserve_raw(&mut self.census, target, "production census");
        });
        self.update_capacity_charge();
    }

    pub fn presize_countries(&mut self) {
        if crate::country_scale::reserve_census(&mut self.census) {
            self.update_capacity_charge();
        }
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionRestCopyReport {
    pub raw_ok: bool,
    pub plan_failed: bool,
    pub copy_ticks: [u64; 8],
    pub raw_bytes: [u64; 8],
}

#[derive(Debug, Clone, Copy)]
pub struct ProductionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn p_pool_u64(log: &RawLog<8>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn pspan(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn copy_production_string(log: &mut RawLog<8>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = pspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = pspan(log, memory, Some(string), 8, mask);
    let len = match usize::try_from(p_pool_u64(log, header_start + msvc_string::LENGTH)) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = p_pool_u64(log, header_start + msvc_string::CAPACITY);
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(p_pool_u64(log, header_start))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(s) => s,
            None => return,
        }
    };
    if source.checked_add(len).is_none() {
        return;
    }
    if len != 0 {
        let _ = pspan(log, memory, Some(source), len, mask);
    }
    let _ = pspan(log, memory, source.checked_add(len), 1, mask);
}

fn copy_production_census(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<ProductionRestCountry>,
) -> bool {
    let Some(countries_start) = pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_P_CENSUS,
    ) else {
        return false;
    };
    let Some(links_start) = pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_P_CENSUS,
    ) else {
        return false;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return false,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(a) => a,
        None => return false,
    };
    if countries.count < 2 || links.count == 0 {
        return false;
    }
    let Some(mut slots) = CountrySlots::new(countries, links, base) else {
        return false;
    };
    if engine_allocation::reserve(census, countries.count.saturating_sub(1) as usize).is_err() {
        log.overflowed = true;
        return false;
    }
    for slot in 1..slots.count() {
        match slots.take(log, memory, slot, M_P_CENSUS) {
            Some(Some(pointer)) => {
                if census.len() == census.capacity() {
                    log.overflowed = true;
                    return false;
                }
                census.push(ProductionRestCountry { pointer });
            }
            Some(None) => {}
            None => return false,
        }
    }
    true
}

fn copy_production_manager(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    country: &ProductionRestCountry,
) -> Option<usize> {
    let ptr_start = pspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::PRODUCTION_STATUS),
        8,
        M_P_PROD_MGR,
    )?;
    let pointer = usize::try_from(p_pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)?;
    let expected = engine_sites::at(base, cproduction_status::VTABLE_RVA)? as u64;
    let vtable_start = pspan(log, memory, Some(pointer), 8, M_P_PROD_MGR)?;
    let owner_start = pspan(
        log,
        memory,
        pointer.checked_add(cproduction_status::COUNTRY),
        8,
        M_P_PROD_MGR,
    )?;
    if p_pool_u64(log, vtable_start) != expected {
        return None;
    }
    if p_pool_u64(log, owner_start) != country.pointer as u64 {
        return None;
    }
    Some(pointer)
}

fn copy_deployment_manager(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    country: &ProductionRestCountry,
) -> Option<usize> {
    let ptr_start = pspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::DEPLOYMENT_MANAGER),
        8,
        M_P_DEPLOY,
    )?;
    let pointer = usize::try_from(p_pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)?;
    let expected = engine_sites::at(base, cdeployment_status::VTABLE_RVA)? as u64;
    let vtable_start = pspan(log, memory, Some(pointer), 8, M_P_DEPLOY)?;
    let owner_start = pspan(
        log,
        memory,
        pointer.checked_add(cdeployment_status::OWNER),
        8,
        M_P_DEPLOY,
    )?;
    if p_pool_u64(log, vtable_start) != expected {
        return None;
    }
    if p_pool_u64(log, owner_start) != country.pointer as u64 {
        return None;
    }
    Some(pointer)
}

fn copy_variant_identity(log: &mut RawLog<8>, memory: &impl Memory, pointer: usize, mask: u32) {
    let _ = pspan(log, memory, Some(pointer), 16, mask);
}

fn copy_production_lines(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    manager: usize,
    defines: &mut Option<[i32; 6]>,
) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LINES),
        16,
        M_P_LINES,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for index in 0..array.count {
        let addr = array.data.checked_add((index as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_LINES) else {
            continue;
        };
        let pointer = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let takes = [
            (pointer, 16),
            (
                pointer
                    .checked_add(cproduction_line::ACTIVE_FACTORIES)
                    .unwrap_or(0),
                4,
            ),
            (
                pointer
                    .checked_add(cproduction_line::PRODUCTION_STATUS)
                    .unwrap_or(0),
                8,
            ),
            (pointer.checked_add(cproduction_line::COST).unwrap_or(0), 24),
            (
                pointer.checked_add(cproduction_line::PRIORITY).unwrap_or(0),
                8,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::EQUIPMENT_VARIANT)
                    .unwrap_or(0),
                16,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::REQUESTED_FACTORIES)
                    .unwrap_or(0),
                4,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::FACTORY_EFFICIENCIES)
                    .unwrap_or(0),
                16,
            ),
        ];
        let mut starts = [None; 8];
        for (start, (addr, len)) in starts.iter_mut().zip(takes) {
            *start = pspan(log, memory, Some(addr), len, M_P_LINES);
        }
        let variant = starts[5].and_then(|start| {
            usize::try_from(p_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        });
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LINES);
            if let [Some(identity), Some(active), .., Some(header)] = starts {
                let kind = usize::try_from(p_pool_u64(log, identity))
                    .ok()
                    .and_then(|vtable| line_kind(base, vtable));
                let active = i32::from_le_bytes(log.bytes[active..active + 4].try_into().unwrap());
                let header = log.bytes[header..header + 16].try_into().unwrap();
                if let Some(kind) = kind {
                    copy_line_efficiency(log, memory, base, kind, v, active, header, defines);
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn line_kind(base: usize, vtable: usize) -> Option<u8> {
    let kinds = [
        engine_native_parts::cmilitary_production_line::VTABLE_RVA,
        engine_native_parts::cnaval_production_line::VTABLE_RVA,
        engine_native_parts::cship_refit_production_line::VTABLE_RVA,
        engine_native_parts::crailway_gun_production_line::VTABLE_RVA,
    ];
    Some(match engine_sites::which(base, vtable, &kinds)? {
        engine_native_parts::cmilitary_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_MILITARY
        }
        engine_native_parts::cnaval_production_line::VTABLE_RVA => cequipment_production_line::KIND_NAVAL,
        engine_native_parts::cship_refit_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_SHIP_REFIT
        }
        engine_native_parts::crailway_gun_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_RAILWAY_GUN
        }
        _ => return None,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "one line-efficiency copy names its kind, variant, activity, header and defines"
)]
fn copy_line_efficiency(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    active: i32,
    header: [u8; 16],
    defines: &mut Option<[i32; 6]>,
) {
    let Some(maximum) = copy_max_factories(log, memory, base, kind, variant, defines) else {
        return;
    };
    let count = u32::try_from(active.min(maximum)).unwrap_or(0);
    if count == 0 {
        return;
    }
    let Some(array) = Array::from_header(header, 8).filter(|array| count <= array.count) else {
        return;
    };
    let batch = cequipment_production_line::EFFICIENCY_BATCH as u32;
    for first in (0..count).step_by(batch as usize) {
        let len = (count - first).min(batch) as usize;
        if pspan(log, memory, array.element(first, 8), len * 8, M_P_LINES).is_none() {
            return;
        }
    }
}

fn copy_max_factories(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    defines: &mut Option<[i32; 6]>,
) -> Option<i32> {
    use engine_native_parts::cequipment_type;
    let optional = match kind {
        cequipment_production_line::KIND_RAILWAY_GUN => {
            return copy_factory_define(log, memory, base, defines, 5);
        }
        cequipment_production_line::KIND_MILITARY => 0,
        _ => cequipment_type::MAX_NAV_FACTORIES - cequipment_type::MAX_MIL_FACTORIES,
    };
    let definition = pspan(
        log,
        memory,
        variant.checked_add(cequipment_variant::DEFINITION),
        8,
        M_P_LINES,
    )?;
    let flags = pspan(
        log,
        memory,
        variant.checked_add(cequipment_variant::CLASSIFIER_FLAGS),
        8,
        M_P_LINES,
    )?;
    let expected = engine_sites::at(base, cequipment_type::VTABLE_RVA)? as u64;
    let mut equipment = usize::try_from(p_pool_u64(log, definition)).ok()?;
    for archetype in [false, true] {
        if equipment == 0 {
            return None;
        }
        let vtable = pspan(log, memory, Some(equipment), 8, M_P_LINES)?;
        let maximum = pspan(
            log,
            memory,
            equipment.checked_add(cequipment_type::MAX_MIL_FACTORIES),
            16,
            M_P_LINES,
        )?;
        let next = if archetype {
            None
        } else {
            Some(pspan(
                log,
                memory,
                equipment.checked_add(cequipment_type::ARCHETYPE),
                8,
                M_P_LINES,
            )?)
        };
        if p_pool_u64(log, vtable) != expected {
            return None;
        }
        let value = maximum + optional;
        if log.bytes[value + 4] != 0 {
            return Some(i32::from_le_bytes(
                log.bytes[value..value + 4].try_into().unwrap(),
            ));
        }
        if let Some(next) = next {
            equipment = usize::try_from(p_pool_u64(log, next)).ok()?;
        }
    }
    let flags = p_pool_u64(log, flags);
    let index = if kind == cequipment_production_line::KIND_MILITARY {
        0
    } else if flags & cequipment_variant::CONVOY_CLASSIFIER_MASK != 0 {
        1
    } else if flags & cequipment_variant::FLOATING_HARBOR_CLASSIFIER_MASK != 0 {
        2
    } else if flags & cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK != 0 {
        4
    } else {
        3
    };
    copy_factory_define(log, memory, base, defines, index)
}

fn copy_factory_define(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    defines: &mut Option<[i32; 6]>,
    index: usize,
) -> Option<i32> {
    if defines.is_none() {
        let mut values = [0; 6];
        for (value, rva) in values
            .iter_mut()
            .zip(cequipment_production_line::MAX_FACTORY_DEFINE_RVAS)
        {
            let start = pspan(log, memory, engine_sites::at(base, rva), 4, M_P_LINES)?;
            *value = i32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
        }
        *defines = Some(values);
    }
    defines.map(|values| values[index])
}

fn copy_production_stockpiles(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let pool = match manager.checked_add(cproduction_status::STOCKPILE_POOL) {
        Some(p) => p,
        None => return,
    };
    let _ = pspan(log, memory, Some(pool), 8, M_P_STOCK);
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::STOCKPILE),
        16,
        M_P_STOCK,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, equipment_entry::STRIDE) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.element(ordinal, equipment_entry::STRIDE);
        let Some(entry_start) = pspan(log, memory, addr, equipment_entry::STRIDE, M_P_STOCK) else {
            continue;
        };
        let variant = usize::try_from(p_pool_u64(log, entry_start))
            .ok()
            .filter(|&p| p != 0);
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_STOCK);
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_production_licenses(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LICENSES),
        16,
        M_P_LIC,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.data.checked_add((ordinal as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_LIC) else {
            continue;
        };
        let variant = usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0);
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LIC);
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_production_records(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LICENSE_RECORDS),
        16,
        M_P_REC,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.data.checked_add((ordinal as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_REC) else {
            continue;
        };
        let pointer = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(head_start) = pspan(log, memory, Some(pointer), 0x58, M_P_REC) else {
            continue;
        };
        let parent = usize::try_from(p_pool_u64(
            log,
            head_start + cequipment_production_license::PARENT,
        ))
        .ok()
        .filter(|&p| p != 0);
        if let Some(p) = parent {
            copy_variant_identity(log, memory, p, M_P_REC);
        }
        let equipment = Array::from_header(
            log.bytes[head_start + cequipment_production_license::EQUIPMENT
                ..head_start + cequipment_production_license::EQUIPMENT + 16]
                .try_into()
                .unwrap(),
            8,
        );
        if let Some(equipment) = equipment {
            for index in 0..equipment.count {
                let addr = equipment
                    .data
                    .checked_add((index as usize).saturating_mul(8));
                let Some(h_start) = pspan(log, memory, addr, 8, M_P_REC) else {
                    continue;
                };
                let variant = usize::try_from(p_pool_u64(log, h_start))
                    .ok()
                    .filter(|&p| p != 0);
                if let Some(v) = variant {
                    copy_variant_identity(log, memory, v, M_P_REC);
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_deployment_queue(log: &mut RawLog<8>, memory: &impl Memory, base: usize, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cdeployment_status::CONVEYORS),
        16,
        M_P_DEPLOY,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let queue = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for index in 0..queue.count {
        let addr = queue.data.checked_add((index as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_DEPLOY) else {
            continue;
        };
        let conveyor = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(head_start) = pspan(log, memory, Some(conveyor), 0x9c, M_P_DEPLOY) else {
            continue;
        };
        let template = usize::try_from(p_pool_u64(
            log,
            head_start + cmilitary_deployment_conveyor::TEMPLATE,
        ))
        .ok()
        .filter(|&p| p != 0);
        if let Some(t) = template {
            let _ = pspan(log, memory, Some(t), 16, M_P_DEPLOY);
        }
        if let Some(name) = conveyor.checked_add(cmilitary_deployment_conveyor::NAME) {
            copy_production_string(log, memory, name, M_P_DEPLOY);
        }
        let _ = pspan(
            log,
            memory,
            conveyor.checked_add(cmilitary_deployment_conveyor::LINES),
            16,
            M_P_DEPLOY,
        );
        let lines = Array::from_header(
            log.bytes[head_start + cmilitary_deployment_conveyor::LINES
                ..head_start + cmilitary_deployment_conveyor::LINES + 16]
                .try_into()
                .unwrap(),
            8,
        );
        let Some(lines) = lines else {
            continue;
        };
        for ordinal in 0..lines.count {
            let addr = lines.data.checked_add((ordinal as usize).saturating_mul(8));
            let Some(lptr_start) = pspan(log, memory, addr, 8, M_P_DEPLOY) else {
                continue;
            };
            let line = match usize::try_from(p_pool_u64(log, lptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                Some(p) => p,
                None => continue,
            };
            copy_deployment_line(log, memory, base, conveyor, line);
            if log.overflowed {
                return;
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_deployment_line(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    conveyor: usize,
    line: usize,
) {
    let _ = base;
    let Some(head_start) = pspan(log, memory, Some(line), 0x40, M_P_DEPLOY) else {
        return;
    };
    let name = usize::try_from(p_pool_u64(
        log,
        head_start + cmilitary_deployment_line::NAME,
    ))
    .ok()
    .filter(|&p| p != 0);
    let deployment = usize::try_from(p_pool_u64(
        log,
        head_start + cmilitary_deployment_line::DEPLOYMENT,
    ))
    .ok()
    .filter(|&p| p != 0);
    let (Some(name), Some(deployment)) = (name, deployment) else {
        return;
    };
    let _ = pspan(log, memory, Some(name), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::ORDER),
        4,
        M_P_DEPLOY,
    );
    let string = match name.checked_add(cname_group_member::OVERRIDE_NAME) {
        Some(s) => s,
        None => return,
    };
    let Some(chead_start) = pspan(log, memory, Some(string), 32, M_P_DEPLOY) else {
        return;
    };
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(log, memory, Some(string), 8, M_P_DEPLOY);
    let len = match usize::try_from(p_pool_u64(log, chead_start + msvc_string::LENGTH)) {
        Ok(l) => l,
        Err(_) => return,
    };
    let capacity = p_pool_u64(log, chead_start + msvc_string::CAPACITY);
    if len as u64 <= capacity && capacity >= msvc_string::INLINE_CAPACITY {
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            string
        } else {
            match usize::try_from(p_pool_u64(log, chead_start))
                .ok()
                .filter(|&s| s != 0)
            {
                Some(s) => s,
                None => string,
            }
        };
        if source.checked_add(len).is_some() {
            if len != 0 {
                let _ = pspan(log, memory, Some(source), len, M_P_DEPLOY);
            }
            let _ = pspan(log, memory, source.checked_add(len), 1, M_P_DEPLOY);
        }
    }
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::FLAG_A),
        1,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::FLAG_B),
        1,
        M_P_DEPLOY,
    );
    let _ = conveyor;
    copy_deployment_object(log, memory, base, deployment, line);
}

fn copy_deployment_object(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    deployment: usize,
    line: usize,
) {
    let _ = base;
    let _ = line;
    let _ = pspan(log, memory, Some(deployment), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::SECONDARY_VTABLE),
        8,
        M_P_DEPLOY,
    );
    let pool = match deployment.checked_add(cmilitary_deployment::EQUIPMENT_POOL) {
        Some(p) => p,
        None => return,
    };
    let _ = pspan(log, memory, Some(pool), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ALLOW_ZERO),
        1,
        M_P_DEPLOY,
    );
    if let Some(h) = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ITEMS),
        16,
        M_P_DEPLOY,
    ) {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[h..h + 16]);
        if let Some(array) = Array::from_header(raw, equipment_entry::STRIDE) {
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, equipment_entry::STRIDE);
                if let Some(e_start) = pspan(log, memory, addr, equipment_entry::STRIDE, M_P_DEPLOY)
                {
                    let variant = usize::try_from(p_pool_u64(log, e_start))
                        .ok()
                        .filter(|&p| p != 0);
                    if let Some(v) = variant {
                        copy_variant_identity(log, memory, v, M_P_DEPLOY);
                    }
                }
            }
        }
    }
    if let Some(h) = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ALLOCATIONS),
        16,
        M_P_DEPLOY,
    ) {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[h..h + 16]);
        if let Some(array) = Array::from_header(raw, 24) {
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, 24);
                let _ = pspan(log, memory, addr, 24, M_P_DEPLOY);
            }
        }
    }
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::TRAINING),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::MAX_TRAINING),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::MAX_MANPOWER),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::OWNING_LINE),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::DISTRIBUTABLE_PRIORITY),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::HALTING_REASON),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::HALTING_REASON_PRESENT),
        1,
        M_P_DEPLOY,
    );
    for offset in [
        cmilitary_deployment::MANPOWER_HAVE,
        cmilitary_deployment::MANPOWER_NEED,
    ] {
        let container = match deployment.checked_add(offset) {
            Some(c) => c,
            None => continue,
        };
        let _ = pspan(log, memory, Some(container), 8, M_P_DEPLOY);
        let Some(data_start) = pspan(
            log,
            memory,
            container.checked_add(carmy_manpower_values::DATA),
            16,
            M_P_DEPLOY,
        ) else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
        if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                let _ = pspan(log, memory, addr, manpower_value_entry::STRIDE, M_P_DEPLOY);
            }
        }
    }
}

pub fn copy_production_rest_refresh(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
) -> ProductionRestCopyReport {
    copy_production_rest(memory, ctx, job)
}

fn copy_production_rest(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
) -> ProductionRestCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    acquire_production_rest_pass(memory, ctx.base, ctx.game_state, job);
    let mut ticks = [0; 8];
    ticks[0] = elapsed(started);
    let plan_failed = job.log.planned_capture_requires_no_retry();
    ProductionRestCopyReport {
        raw_ok: !plan_failed && !job.log.overflowed,
        plan_failed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    pub fn fail_production_rest_capture(&mut self, reason: &'static str) {
        self.lines.outcome = Outcome::Unavailable { reason };
        self.stockpiles.outcome = Outcome::Unavailable { reason };
        self.licenses.outcome = Outcome::Unavailable { reason };
        self.conveyors.outcome = Outcome::Unavailable { reason };
        self.deployment_lines.outcome = Outcome::Unavailable { reason };
        self.deployment_manpower.outcome = Outcome::Unavailable { reason };
        self.deployment_equipment.outcome = Outcome::Unavailable { reason };
        self.license_records.outcome = Outcome::Unavailable { reason };
        self.rest_raw_mode = false;
    }
}

fn acquire_production_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ProductionRestJob,
) {
    let _reads = read_accounting::table(TABLES[0]);
    let key = [base, game_state, job.census.capacity(), 1];
    if !job.log.begin_planned_capture(memory, key) {
        copy_production_rest_pass(memory, base, game_state, job);
    }
    job.log.finish_planned_capture();
}

fn copy_production_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ProductionRestJob,
) {
    if !copy_production_census(&mut job.log, memory, base, game_state, &mut job.census) {
        return;
    }
    let mut defines = None;
    for index in 0..job.census.len() {
        let country = job.census[index];
        if let Some(manager) = copy_production_manager(&mut job.log, memory, base, &country) {
            copy_production_lines(&mut job.log, memory, base, manager, &mut defines);
            if job.log.overflowed {
                return;
            }
            copy_production_stockpiles(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
            copy_production_licenses(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
            copy_production_records(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
        }
        if let Some(manager) = copy_deployment_manager(&mut job.log, memory, base, &country) {
            copy_deployment_queue(&mut job.log, memory, base, manager);
            if job.log.overflowed {
                return;
            }
        }
    }
}
