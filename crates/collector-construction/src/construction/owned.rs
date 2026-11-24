use super::*;
use crate::raw::CountrySlots;

impl Captured {
    pub fn decode_construction_rest_payload(
        &mut self,
        base: usize,
        game_state: usize,
        job: &ConstructionRestJob,
        decode_ticks: &mut [u64; 10],
    ) {
        let recorded = match crate::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_construction_rest_capture(reason);
                return;
            }
        };
        let started = qpc();
        self.base_resources.outcome = state_resource_entry::walk_buffer(
            &recorded,
            base,
            game_state,
            engine_native_memory::walk::Buffer::Growing(&mut self.base_resources.rows),
        );
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let queue = cbuilding_production_line::walk_buffer(
            &recorded,
            base,
            game_state,
            cbuilding_production_line::BuffersBuffer {
                buildings: engine_native_memory::walk::Buffer::Growing(&mut self.building_construction.rows),
                railways: engine_native_memory::walk::Buffer::Growing(&mut self.railway_construction.rows),
                route_provinces: engine_native_memory::walk::Buffer::Growing(&mut self.routes.first),
            },
        );
        self.building_construction.outcome = queue.buildings;
        self.railway_construction.outcome = queue.railways;
        self.routes.len = queue.route_len;
        self.routes.required = queue.required_route_len;
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        let settings_error = if job.settings_required > self.settings.rows.len() {
            let additional = job.settings_required - self.settings.rows.len();
            match reserve(&mut self.settings.rows, additional) {
                Ok(()) => {
                    self.settings
                        .rows
                        .resize(job.settings_required, Default::default());
                    None
                }
                Err(_) => Some("occupation settings allocation failed"),
            }
        } else {
            None
        };
        let law_error = if job.law_demand_overflowed {
            Some("occupation law byte demand overflow")
        } else if job.law_bytes_required > self.law_bytes.first.len() {
            let additional = job.law_bytes_required - self.law_bytes.first.len();
            match reserve(&mut self.law_bytes.first, additional) {
                Ok(()) => {
                    self.law_bytes.first.resize(job.law_bytes_required, 0);
                    None
                }
                Err(_) => Some("occupation law byte allocation failed"),
            }
        } else {
            None
        };
        let group_error = if job.group_laws_required > self.group_laws.rows.len() {
            let additional = job.group_laws_required - self.group_laws.rows.len();
            match reserve(&mut self.group_laws.rows, additional) {
                Ok(()) => {
                    self.group_laws
                        .rows
                        .resize(job.group_laws_required, Default::default());
                    None
                }
                Err(_) => Some("occupied-country law allocation failed"),
            }
        } else {
            None
        };
        if job.equipment_required > self.equipment.first.len() {
            let additional = job.equipment_required - self.equipment.first.len();
            if reserve(&mut self.equipment.first, additional).is_ok() {
                self.equipment
                    .first
                    .resize(job.equipment_required, Default::default());
            }
        }
        let occupation = ccountry_occupation_status::walk_with_occupied_buffer(
            &recorded,
            base,
            game_state,
            ccountry_occupation_status::BuffersBuffer {
                settings: engine_native_memory::walk::Buffer::Growing(&mut self.settings.rows),
                laws: engine_native_memory::walk::Buffer::Growing(&mut self.laws.rows),
                group_laws: engine_native_memory::walk::Buffer::Growing(&mut self.group_laws.rows),
                garrisons: engine_native_memory::walk::Buffer::Growing(&mut self.garrisons.rows),
                manpower: engine_native_memory::walk::Buffer::Growing(&mut self.assigned_manpower.rows),
                reserve_manpower: engine_native_memory::walk::Buffer::Growing(&mut self.reserve_manpower.rows),
                names: engine_native_memory::walk::Buffer::Growing(&mut self.law_bytes.first),
                equipment: engine_native_memory::walk::Buffer::Growing(&mut self.equipment.first),
            },
            engine_native_memory::walk::Buffer::Growing(&mut self.occupied.first),
        );
        self.settings.outcome = occupation.settings;
        self.laws.outcome = occupation.laws;
        self.group_laws.outcome = occupation.group_laws;
        self.fallback_law = occupation.fallback_law;
        if let Some(reason) = group_error {
            self.group_laws.outcome = Outcome::Unavailable { reason };
        }
        self.garrisons.outcome = occupation.garrisons;
        self.assigned_manpower.outcome = occupation.manpower;
        self.reserve_manpower.outcome = occupation.reserve_manpower;
        self.law_bytes.len = occupation.bytes_len;
        self.law_bytes.required = occupation.required_bytes;
        self.occupied.len = occupation.occupied_len;
        self.occupied.required = occupation.required_occupied;
        self.equipment.len = occupation.equipment_len;
        self.equipment.required = occupation.required_equipment.max(job.equipment_required);
        if law_error.is_none() {
            self.law_bytes.required = self.law_bytes.required.max(job.law_bytes_required);
        }
        if let Some(reason) = settings_error.or(law_error)
            && !matches!(self.settings.outcome, Outcome::Unavailable { .. })
        {
            self.settings.outcome = Outcome::Unavailable { reason };
        }
        if let Some(reason) = law_error {
            self.group_laws.outcome = Outcome::Unavailable { reason };
            self.fallback_law = None;
        }
        if let Some(reason) = law_error
            && !matches!(self.laws.outcome, Outcome::Unavailable { .. })
        {
            self.laws.outcome = Outcome::Unavailable { reason };
        }
        decode_ticks[3] += elapsed(started);
        self.resistance.outcome = Outcome::Unavailable {
            reason: "state resistance is acquired by the state worker",
        };
        let started = qpc();
        self.collaboration.outcome = ccountry_collaboration_status::walk_buffer(
            &recorded,
            base,
            game_state,
            engine_native_memory::walk::Buffer::Growing(&mut self.collaboration.rows),
        );
        decode_ticks[9] += elapsed(started);
        self.rest_raw_mode = false;
    }
}


const M_C_RES: u32 = 0x001;
const M_C_BLD: u32 = 0x002;
const M_C_RAIL: u32 = 0x004;
const M_C_COLLAB: u32 = 0x200;
const M_C_ALL: u32 = 0x2FF;
const M_C_QUEUE: u32 = 0x006;
pub(super) const M_C_OCC: u32 = 0x0F8;

#[derive(Debug, Default)]
pub struct ConstructionRestJob {
    pub log: RawLog<10>,
    pub game_state: usize,
    pub(super) census: Vec<ConstructionRestCountry>,
    pub(super) settings_required: usize,
    pub(super) group_laws_required: usize,
    pub(super) law_bytes_required: usize,
    pub(super) equipment_required: usize,
    pub(super) law_demand_overflowed: bool,
    pub(super) capacity_charge: crate::raw_memory::Charge,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ConstructionRestCountry {
    pub(super) pointer: usize,
}

impl ConstructionRestJob {
    pub fn new() -> Self {
        let mut job = Self {
            log: RawLog::new(0, 0),
            game_state: 0,
            census: Vec::new(),
            settings_required: 0,
            group_laws_required: 0,
            law_bytes_required: 0,
            equipment_required: 0,
            law_demand_overflowed: false,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
        self.settings_required = 0;
        self.group_laws_required = 0;
        self.law_bytes_required = 0;
        self.equipment_required = 0;
        self.law_demand_overflowed = false;
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
            let _ = crate::allocation::reserve_raw(&mut self.census, target, "construction census");
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
pub struct ConstructionRestCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; 10],
    pub raw_bytes: [u64; 10],
}

#[derive(Debug, Clone, Copy)]
pub struct ConstructionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn c_pool_u64(log: &RawLog<10>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn c_pool_u32(log: &RawLog<10>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn cspan(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

pub(super) fn copy_construction_string(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    string: usize,
    mask: u32,
    required: &mut usize,
    demand_overflowed: &mut bool,
) {
    let Some(header_start) = cspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    let _ = cspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = cspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = cspan(log, memory, Some(string), 8, mask);
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
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
    if let Some(total) = required.checked_add(len) {
        *required = total;
    } else {
        *demand_overflowed = true;
    }
    if len != 0 {
        let _ = cspan(log, memory, Some(source), len, mask);
    }
    let _ = cspan(log, memory, source.checked_add(len), 1, mask);
}

fn copy_header(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    game_state: usize,
    offset: usize,
    stride: usize,
) -> Option<Array> {
    let start = cspan(log, memory, game_state.checked_add(offset), 16, M_C_ALL)?;
    Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), stride)
}

fn copy_country_roots(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<ConstructionRestCountry>,
) -> Option<CountrySlots> {
    let countries = copy_header(log, memory, game_state, cgame_state::COUNTRIES, 8)?;
    let links = copy_header(log, memory, game_state, cgame_state::COUNTRY_LINKS, 4)?;
    let mut slots = CountrySlots::new(countries, links, base)?;
    if engine_allocation::reserve_exact(census, slots.count() as usize - 1).is_err() {
        log.overflowed = true;
        return None;
    }
    for slot in 1..slots.count() {
        match slots.take(log, memory, slot, M_C_ALL) {
            Some(Some(pointer)) if census.len() < census.capacity() => {
                census.push(ConstructionRestCountry { pointer });
            }
            Some(Some(_)) => {
                log.overflowed = true;
                return None;
            }
            Some(None) => {}
            None => break,
        }
    }
    Some(slots)
}

fn copy_states_walk(log: &mut RawLog<10>, memory: &impl Memory, base: usize, states: &Array) {
    let state_vtable = match engine_sites::at(base, cstate::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let template_vtable = match engine_sites::at(base, cstate_template::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let _ = (state_vtable, template_vtable);
    for state in 1..states.count {
        let addr = states.element(state, 8);
        let Some(ptr_start) = cspan(log, memory, addr, 8, M_C_ALL) else {
            continue;
        };
        let pointer = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let _ = cspan(log, memory, Some(pointer), 8, M_C_ALL);
        let _ = cspan(log, memory, pointer.checked_add(cstate::ID), 4, M_C_ALL);
        if let Some(tptr_start) = cspan(
            log,
            memory,
            pointer.checked_add(cstate::TEMPLATE),
            8,
            M_C_RES,
        ) && let Some(template) = usize::try_from(c_pool_u64(log, tptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(template), 8, M_C_RES);
            let _ = cspan(
                log,
                memory,
                template.checked_add(cstate_template::ID),
                4,
                M_C_RES,
            );
            if let Some(rh_start) = cspan(
                log,
                memory,
                template.checked_add(cstate_template::RESOURCES),
                16,
                M_C_RES,
            ) {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[rh_start..rh_start + 16]);
                if let Some(resources) = Array::from_header(raw, state_resource_entry::STRIDE) {
                    for ordinal in 0..resources.count {
                        let addr = resources.element(ordinal, state_resource_entry::STRIDE);
                        let _ = cspan(log, memory, addr, 12, M_C_RES);
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_construction_queue(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[ConstructionRestCountry],
) {
    let states = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STATES),
        16,
        M_C_BLD,
    )
    .and_then(|start| Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), 8));
    let provinces = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::PROVINCES),
        16,
        M_C_BLD,
    )
    .and_then(|start| Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), 8));
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::PRODUCTION_STATUS),
            8,
            M_C_QUEUE,
        ) else {
            continue;
        };
        let manager = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(m) => m,
            None => continue,
        };
        let _ = cspan(log, memory, Some(manager), 8, M_C_QUEUE);
        let _ = cspan(
            log,
            memory,
            manager.checked_add(cproduction_status::COUNTRY),
            8,
            M_C_QUEUE,
        );
        let Some(qh_start) = cspan(
            log,
            memory,
            manager.checked_add(cproduction_status::CONSTRUCTION_LINES),
            16,
            M_C_QUEUE,
        ) else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[qh_start..qh_start + 16]);
        let lines = match Array::from_header(raw, 8) {
            Some(a) => a,
            None => continue,
        };
        for ordinal in 0..lines.count {
            let addr = lines.data.checked_add((ordinal as usize).saturating_mul(8));
            let Some(lptr_start) = cspan(log, memory, addr, 8, M_C_QUEUE) else {
                continue;
            };
            let pointer = match usize::try_from(c_pool_u64(log, lptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                Some(p) => p,
                None => continue,
            };
            let Some(vtable_start) = cspan(log, memory, Some(pointer), 8, M_C_QUEUE) else {
                continue;
            };
            let vtable = c_pool_u64(log, vtable_start);
            let building_vtable =
                engine_sites::at(base, cbuilding_production_line::VTABLE_RVA).map(|v| v as u64);
            let railway_vtable =
                engine_sites::at(base, crailway_production_line::VTABLE_RVA).map(|v| v as u64);
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::REFERENCE_TYPE),
                8,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::ACTIVE_FACTORIES),
                4,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::COST),
                24,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::PRIORITY),
                8,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cgeneral_production_line::CREATED_DATE),
                4,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::PRODUCTION_STATUS),
                8,
                M_C_QUEUE,
            );
            if Some(vtable) == building_vtable {
                if let Some(bptr_start) = cspan(
                    log,
                    memory,
                    pointer.checked_add(cbuilding_production_line::BUILDING),
                    8,
                    M_C_BLD,
                ) && let Some(target) = usize::try_from(c_pool_u64(log, bptr_start))
                    .ok()
                    .filter(|&p| p != 0)
                {
                    let _ = cspan(log, memory, Some(target), 8, M_C_BLD);
                    let _ = cspan(
                        log,
                        memory,
                        target.checked_add(cbuilding::TYPE_ID),
                        4,
                        M_C_BLD,
                    );
                    if let Some(sptr_start) = cspan(
                        log,
                        memory,
                        target.checked_add(cbuilding::BUILDING_STATUS),
                        8,
                        M_C_BLD,
                    ) && let Some(status) = usize::try_from(c_pool_u64(log, sptr_start))
                        .ok()
                        .filter(|&p| p != 0)
                        && let (Some(states), Some(provinces)) = (states, provinces)
                    {
                        copy_construction_target(log, memory, base, status, states, provinces);
                    }
                }
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(cbuilding_production_line::REPAIR_LEVELS_REMAINING),
                    4,
                    M_C_BLD,
                );
            } else if Some(vtable) == railway_vtable {
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::BASE_LEVEL),
                    9,
                    M_C_RAIL,
                );
                if let Some(rh_start) = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::ROUTE),
                    16,
                    M_C_RAIL,
                ) {
                    let mut raw = [0u8; 16];
                    raw.copy_from_slice(&log.bytes[rh_start..rh_start + 16]);
                    if let Some(route) = Array::from_header(raw, 4) {
                        for index in 0..route.count {
                            let addr = route.element(index, 4);
                            let _ = cspan(log, memory, addr, 4, M_C_RAIL);
                        }
                    }
                }
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::CURRENT_SEGMENT),
                    4,
                    M_C_RAIL,
                );
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::REMAINING_COST),
                    8,
                    M_C_RAIL,
                );
            }
            if log.overflowed {
                return;
            }
        }
    }
}

fn copy_construction_target(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    status: usize,
    states: Array,
    provinces: Array,
) {
    let Some(vt) = cspan(log, memory, Some(status), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt))
        != engine_sites::at(base, cbuilding_status::VTABLE_RVA).map(|p| p as u64)
    {
        return;
    }
    let Some(fields) = cspan(
        log,
        memory,
        status.checked_add(cbuilding_status::FLAGS),
        12,
        M_C_BLD,
    ) else {
        return;
    };
    let flags = u32::from_le_bytes(log.bytes[fields..fields + 4].try_into().unwrap());
    let location = u32::from_le_bytes(log.bytes[fields + 4..fields + 8].try_into().unwrap());
    if location == 0 {
        return;
    }
    let province = flags & cbuilding_status::PROVINCE_FLAG != 0;
    let (array, vtable, id_offset, status_offset) = if province {
        (
            provinces,
            cprovince::VTABLE_RVA,
            cprovince::ID,
            cprovince::BUILDING_STATUS,
        )
    } else {
        (
            states,
            cstate::VTABLE_RVA,
            cstate::ID,
            cstate::BUILDING_STATUS,
        )
    };
    let Some(slot) = cspan(log, memory, array.element(location, 8), 8, M_C_BLD) else {
        return;
    };
    let pointer = c_pool_u64(log, slot) as usize;
    if pointer == 0 {
        return;
    }
    let Some(vt) = cspan(log, memory, Some(pointer), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt)) != engine_sites::at(base, vtable).map(|p| p as u64)
        || pointer.checked_add(status_offset) != Some(status)
    {
        return;
    }
    let Some(id) = cspan(log, memory, pointer.checked_add(id_offset), 4, M_C_BLD) else {
        return;
    };
    if u32::from_le_bytes(log.bytes[id..id + 4].try_into().unwrap()) != location || !province {
        return;
    }
    let Some(state) = cspan(
        log,
        memory,
        pointer.checked_add(cprovince::STATE),
        8,
        M_C_BLD,
    ) else {
        return;
    };
    let state = c_pool_u64(log, state) as usize;
    if state == 0 {
        return;
    }
    let Some(vt) = cspan(log, memory, Some(state), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt)) != engine_sites::at(base, cstate::VTABLE_RVA).map(|p| p as u64) {
        return;
    }
    let Some(id) = cspan(log, memory, state.checked_add(cstate::ID), 4, M_C_BLD) else {
        return;
    };
    let id = u32::from_le_bytes(log.bytes[id..id + 4].try_into().unwrap());
    if id != 0 {
        let _ = cspan(log, memory, states.element(id, 8), 8, M_C_BLD);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the occupation walk raises four separate demand counters"
)]
fn copy_occupation_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
    slots: Option<&CountrySlots>,
    groups_required: &mut usize,
    required: &mut usize,
    demand_overflowed: &mut bool,
    equipment_required: &mut usize,
) {
    if let Some(at) = cspan(
        log,
        memory,
        engine_sites::at(base, coccupation_law::DATABASE_RVA),
        8,
        M_C_OCC,
    ) && let Ok(database) = usize::try_from(c_pool_u64(log, at))
        && database != 0
        && let Some(at) = cspan(
            log,
            memory,
            database.checked_add(coccupation_law::DATABASE_FALLBACK),
            8,
            M_C_OCC,
        )
        && let Ok(law) = usize::try_from(c_pool_u64(log, at))
        && law != 0
    {
        let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
        if let Some(name) = law.checked_add(coccupation_law::NAME) {
            copy_construction_string(log, memory, name, M_C_OCC, required, demand_overflowed);
        }
    }
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::OCCUPATION_STATUS),
            8,
            M_C_OCC,
        ) else {
            continue;
        };
        let status = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(s) => s,
            None => continue,
        };
        let _ = cspan(log, memory, Some(status), 8, M_C_OCC);
        let _ = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::SECONDARY_BASE),
            8,
            M_C_OCC,
        );
        if let Some(tptr_start) = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::GARRISON_TEMPLATE),
            8,
            M_C_OCC,
        ) && let Some(template) = usize::try_from(c_pool_u64(log, tptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(template), 8, M_C_OCC);
            let _ = cspan(
                log,
                memory,
                template.checked_add(creferenced_division_template::TYPE),
                8,
                M_C_OCC,
            );
        }
        if let Some(lptr_start) = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::DEFAULT_LAW),
            8,
            M_C_OCC,
        ) && let Some(law) = usize::try_from(c_pool_u64(log, lptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
            if let Some(name) = law.checked_add(coccupation_law::NAME) {
                copy_construction_string(log, memory, name, M_C_OCC, required, demand_overflowed);
            }
        }
        if let Some(container) = status.checked_add(ccountry_occupation_status::ASSIGNED_MANPOWER) {
            let _ = cspan(log, memory, Some(container), 8, M_C_OCC);
            if let Some(data_start) = cspan(
                log,
                memory,
                container.checked_add(carmy_manpower_values::DATA),
                16,
                M_C_OCC,
            ) {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
                if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
                    for ordinal in 0..array.count {
                        let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                        let _ = cspan(log, memory, addr, manpower_value_entry::STRIDE, M_C_OCC);
                    }
                }
            }
        }
        copy_occupation_entries(
            log,
            memory,
            base,
            status,
            slots,
            groups_required,
            required,
            demand_overflowed,
            equipment_required,
        );
        if let Some(header) = status.checked_add(ccountry_occupation_status::OCCUPATION_DATA_ROWS)
            && let Some(start) = cspan(log, memory, Some(header), 16, 0)
        {
            let raw: [u8; 16] = log.bytes[start..start + 16].try_into().unwrap();
            if let Some(array) = Array::from_header(raw, 8) {
                for ordinal in 0..array.count {
                    let Some(start) = cspan(log, memory, array.element(ordinal, 8), 8, 0) else {
                        continue;
                    };
                    let Some(group) = usize::try_from(c_pool_u64(log, start)).ok() else {
                        continue;
                    };
                    if let Some(start) = cspan(log, memory, Some(group), 32, 0) {
                        let country = u64::from_le_bytes(
                            log.bytes[start + 24..start + 32].try_into().unwrap(),
                        );
                        if let Ok(country) = usize::try_from(country) {
                            let _ = cspan(log, memory, Some(country), 12, 0);
                        }
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
    let _ = base;
}

fn copy_pdx_visit(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    table: usize,
    mask: u32,
) -> Option<Vec<(u32, usize)>> {
    let h_start = cspan(log, memory, Some(table), 17, mask)?;
    let mut header = [0u8; 17];
    header.copy_from_slice(&log.bytes[h_start..h_start + 17]);
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap())).ok()?;
    let size = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let capacity = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let max = header[16];
    if size > capacity {
        return None;
    }
    if data == 0 {
        return if size == 0 { Some(Vec::new()) } else { None };
    }
    let span = (capacity as usize)
        .checked_add(max as usize)?
        .checked_add(1)?;
    let mut live = Vec::new();
    reserve_exact(&mut live, size as usize).ok()?;
    for index in 0..span {
        let addr = data.checked_add(index.saturating_mul(pdx_robin_hood_bucket::STRIDE))?;
        let Some(b_start) = cspan(log, memory, Some(addr), 24, mask) else {
            continue;
        };
        let mut bucket = [0u8; 24];
        bucket.copy_from_slice(&log.bytes[b_start..b_start + 24]);
        match bucket[pdx_robin_hood_bucket::DISTANCE_PLUS_ONE] {
            pdx_robin_hood_bucket::EMPTY_DISTANCE => continue,
            pdx_robin_hood_bucket::ITERATOR_SENTINEL_DISTANCE => return None,
            _ => {}
        }
        let key = u32::from_le_bytes(
            bucket[pdx_robin_hood_bucket::KEY..pdx_robin_hood_bucket::KEY + 4]
                .try_into()
                .unwrap(),
        );
        let value = usize::try_from(u64::from_le_bytes(
            bucket[pdx_robin_hood_bucket::VALUE..].try_into().unwrap(),
        ))
        .ok()
        .filter(|&v| v != 0)?;
        if live.len() == size as usize {
            return None;
        }
        live.push((key, value));
        if log.overflowed {
            return None;
        }
    }
    Some(live)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the occupation entries raise four separate demand counters"
)]
fn copy_occupation_entries(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    status: usize,
    slots: Option<&CountrySlots>,
    groups_required: &mut usize,
    required: &mut usize,
    demand_overflowed: &mut bool,
    equipment_required: &mut usize,
) {
    let Some(entries) = status
        .checked_add(ccountry_occupation_status::OCCUPATION_DATA_TABLE)
        .and_then(|table| copy_pdx_visit(log, memory, table, M_C_OCC))
    else {
        return;
    };
    if let Some(total) = groups_required.checked_add(entries.len()) {
        *groups_required = total;
    } else {
        log.overflowed = true;
        return;
    }
    for (_, value) in entries {
        if let Some(at) = cspan(
            log,
            memory,
            value.checked_add(ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER),
            8,
            M_C_OCC,
        ) && let Ok(country) = usize::try_from(c_pool_u64(log, at))
            && country != 0
        {
            let _ = cspan(log, memory, Some(country), 8, M_C_OCC);
            let _ = cspan(log, memory, country.checked_add(ccountry::ID), 4, M_C_OCC);
        }
        if let Some(at) = cspan(
            log,
            memory,
            value.checked_add(ccountry_occupation_data::GROUP_LAW),
            8,
            M_C_OCC,
        ) && let Ok(law) = usize::try_from(c_pool_u64(log, at))
            && law != 0
        {
            let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
            if let Some(name) = law.checked_add(coccupation_law::NAME) {
                copy_construction_string(log, memory, name, M_C_OCC, required, demand_overflowed);
            }
        }
        let Some(id_start) = cspan(log, memory, Some(value), 20, M_C_OCC) else {
            continue;
        };
        let identity: [u8; 20] = log.bytes[id_start..id_start + 20].try_into().unwrap();
        let word = |at: usize| u64::from_le_bytes(identity[at..at + 8].try_into().unwrap());
        if let Some(slots) = slots
            && engine_sites::at(base, ccountry_occupation_data::VTABLE_RVA)
                .map(|vtable| vtable as u64)
                == Some(word(0))
            && word(ccountry_occupation_data::PARENT_STATUS) == status as u64
        {
            let occupied = u32::from_le_bytes(
                identity[ccountry_occupation_data::OCCUPIED_COUNTRY..]
                    .try_into()
                    .unwrap(),
            );
            let _ = slots.resolve(log, memory, occupied, M_C_OCC);
        }
        if let Some(law_table) = value.checked_add(ccountry_occupation_data::LAW_TABLE)
            && let Some(laws) = copy_pdx_visit(log, memory, law_table, M_C_OCC)
        {
            for (_, law) in laws {
                let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
                if let Some(name) = law.checked_add(coccupation_law::NAME) {
                    copy_construction_string(
                        log,
                        memory,
                        name,
                        M_C_OCC,
                        required,
                        demand_overflowed,
                    );
                }
                if log.overflowed {
                    return;
                }
            }
        }
        if let Some(gar_table) = value.checked_add(ccountry_occupation_data::GARRISON_TABLE)
            && let Some(garrisons) = copy_pdx_visit(log, memory, gar_table, M_C_OCC)
        {
            for (gkey, garrison) in garrisons {
                copy_garrison_object(log, memory, base, gkey, garrison, value, equipment_required);
                if log.overflowed {
                    return;
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_garrison_object(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    key: u32,
    garrison: usize,
    parent: usize,
    equipment_required: &mut usize,
) {
    let _ = base;
    let _ = parent;
    let _ = key;
    let _ = cspan(log, memory, Some(garrison), 8, M_C_OCC);
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::STATE),
        16,
        M_C_OCC,
    );
    copy_garrison_equipment(log, memory, garrison, equipment_required);
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::SERIALIZED_FLAG_CANDIDATE),
        1,
        M_C_OCC,
    );
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::REQUIRED_MANPOWER),
        4,
        M_C_OCC,
    );
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::STRENGTH_RATIO),
        16,
        M_C_OCC,
    );
    if let Some(container) = garrison.checked_add(cstate_garrison_data::ASSIGNED_MANPOWER) {
        let _ = cspan(log, memory, Some(container), 8, M_C_OCC);
        if let Some(data_start) = cspan(
            log,
            memory,
            container.checked_add(carmy_manpower_values::DATA),
            16,
            M_C_OCC,
        ) {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
            if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
                for ordinal in 0..array.count {
                    let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                    let _ = cspan(log, memory, addr, manpower_value_entry::STRIDE, M_C_OCC);
                }
            }
        }
    }
}

fn copy_garrison_equipment(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    garrison: usize,
    required: &mut usize,
) {
    use engine_native_parts::{
        allocation_equipment_row as allocation,
        cequipment_type as ty,
    };
    use engine_native::cequipment_variant::pool as pool;
    use engine_occupation::garrison_equipment_pool;
    let Some(requirements) = garrison.checked_add(cstate_garrison_data::REQUIRED_EQUIPMENT) else {
        return;
    };
    let _ = cspan(log, memory, Some(requirements), 8, M_C_OCC);
    let Some(at) = cspan(
        log,
        memory,
        requirements.checked_add(garrison_equipment_pool::DATA),
        16,
        M_C_OCC,
    ) else {
        return;
    };
    let Some(array) = Array::from_header(log.bytes[at..at + 16].try_into().unwrap(), 16) else {
        return;
    };
    let Some(demand) = required.checked_add(array.count as usize) else {
        log.overflowed = true;
        return;
    };
    *required = demand;
    if array.count == 0 {
        return;
    }
    for ordinal in 0..array.count {
        let Some(at) = cspan(log, memory, array.element(ordinal, 16), 16, M_C_OCC) else {
            continue;
        };
        let equipment = c_pool_u64(log, at) as usize;
        for (offset, len) in [(0, 8), (ty::TOKEN, 4), (ty::BUILD_COST_IC, 8)] {
            let _ = cspan(log, memory, equipment.checked_add(offset), len, M_C_OCC);
        }
        if let Some(at) = cspan(
            log,
            memory,
            equipment.checked_add(ty::ARCHETYPE),
            8,
            M_C_OCC,
        ) {
            let archetype = c_pool_u64(log, at) as usize;
            for (offset, len) in [
                (0, 8),
                (ty::TOKEN, 4),
                (ty::ACTIVE, 1),
                (ty::ARCHETYPE_ID, 4),
            ] {
                let _ = cspan(log, memory, archetype.checked_add(offset), len, M_C_OCC);
            }
        }
        if log.overflowed {
            return;
        }
    }
    let Some(allocated) = garrison.checked_add(cstate_garrison_data::LOCAL_EQUIPMENT) else {
        return;
    };
    let _ = cspan(log, memory, Some(allocated), 8, M_C_OCC);
    let Some(at) = cspan(
        log,
        memory,
        allocated.checked_add(pool::ALLOCATIONS),
        16,
        M_C_OCC,
    ) else {
        return;
    };
    let Some(array) = Array::from_header(
        log.bytes[at..at + 16].try_into().unwrap(),
        allocation::STRIDE,
    ) else {
        return;
    };
    for ordinal in 0..array.count {
        if let Some(at) = cspan(
            log,
            memory,
            array.element(ordinal, allocation::STRIDE),
            allocation::STRIDE,
            M_C_OCC,
        ) {
            let archetype = c_pool_u64(log, at) as usize;
            let _ = cspan(log, memory, Some(archetype), 8, M_C_OCC);
            let _ = cspan(
                log,
                memory,
                archetype.checked_add(ty::ARCHETYPE_ID),
                4,
                M_C_OCC,
            );
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_collaboration_link(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    links: Option<Array>,
    raw: u32,
) {
    if raw != 0 {
        let address = links.and_then(|links| links.element(raw, 4));
        let _ = cspan(log, memory, address, 4, M_C_COLLAB);
    }
}

fn copy_collaboration_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
    links: Option<Array>,
) {
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::COLLABORATION_STATUS),
            8,
            M_C_COLLAB,
        ) else {
            continue;
        };
        let status = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(s) => s,
            None => continue,
        };
        let _ = cspan(log, memory, Some(status), 8, M_C_COLLAB);
        let _ = cspan(
            log,
            memory,
            status.checked_add(ccountry_collaboration_status::OWNER),
            8,
            M_C_COLLAB,
        );
        for (offset, mask) in [
            (ccountry_collaboration_status::RELATIONS, M_C_COLLAB),
            (
                ccountry_collaboration_status::INCOMING_COUNTRIES,
                M_C_COLLAB,
            ),
        ] {
            let Some(h_start) = cspan(log, memory, status.checked_add(offset), 16, mask) else {
                continue;
            };
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[h_start..h_start + 16]);
            let stride = if offset == ccountry_collaboration_status::RELATIONS {
                8
            } else {
                4
            };
            if let Some(array) = Array::from_header(raw, stride) {
                for index in 0..array.count {
                    let addr = if stride == 8 {
                        array.data.checked_add((index as usize).saturating_mul(8))
                    } else {
                        array.data.checked_add((index as usize).saturating_mul(4))
                    };
                    if stride == 8 {
                        if let Some(r_start) = cspan(log, memory, addr, 8, mask)
                            && let Some(rel) = usize::try_from(c_pool_u64(log, r_start))
                                .ok()
                                .filter(|&p| p != 0)
                        {
                            if let (Some(target), Some(progress)) = (
                                rel.checked_add(ccountry_collaboration_data::TARGET_COUNTRY),
                                rel.checked_add(ccountry_collaboration_data::PROGRESS),
                            ) {
                                if let Some(([identity, target, _], 3)) = log.take_scattered(
                                    memory,
                                    [rel, target, progress],
                                    [8, 4, 8],
                                    [mask; 3],
                                ) && engine_sites::at(
                                    base,
                                    ccountry_collaboration_data::VTABLE_RVA,
                                ) == usize::try_from(c_pool_u64(log, identity)).ok()
                                {
                                    let raw = c_pool_u32(log, target);
                                    copy_collaboration_link(log, memory, links, raw);
                                }
                            }
                        }
                    } else if let Some(start) = cspan(log, memory, addr, 4, mask) {
                        let raw = c_pool_u32(log, start);
                        copy_collaboration_link(log, memory, links, raw);
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

pub fn copy_construction_rest_refresh(
    memory: &impl Memory,
    ctx: ConstructionRestCopyContext,
    job: &mut ConstructionRestJob,
) -> ConstructionRestCopyReport {
    copy_construction_rest(memory, ctx, job)
}

fn copy_construction_rest(
    memory: &impl Memory,
    ctx: ConstructionRestCopyContext,
    job: &mut ConstructionRestJob,
) -> ConstructionRestCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    copy_construction_rest_pass(memory, ctx.base, ctx.game_state, job);
    let mut ticks = [0; 10];
    ticks[0] = elapsed(started);
    ConstructionRestCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    pub fn fail_construction_rest_capture(&mut self, reason: &'static str) {
        self.base_resources.fail(Outcome::Unavailable { reason });
        self.building_construction
            .fail(Outcome::Unavailable { reason });
        self.railway_construction
            .fail(Outcome::Unavailable { reason });
        self.settings.fail(Outcome::Unavailable { reason });
        self.laws.fail(Outcome::Unavailable { reason });
        self.group_laws.fail(Outcome::Unavailable { reason });
        self.fallback_law = None;
        self.garrisons.fail(Outcome::Unavailable { reason });
        self.assigned_manpower.fail(Outcome::Unavailable { reason });
        self.reserve_manpower.fail(Outcome::Unavailable { reason });
        self.resistance.fail(Outcome::Unavailable { reason });
        self.collaboration.fail(Outcome::Unavailable { reason });
        self.rest_raw_mode = false;
    }
}

fn copy_construction_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ConstructionRestJob,
) {
    let _reads = read_accounting::table(TABLES[2]);
    let states = copy_header(&mut job.log, memory, game_state, cgame_state::STATES, 8);
    let slots = copy_country_roots(&mut job.log, memory, base, game_state, &mut job.census);
    job.settings_required = job.census.len();
    if job.log.overflowed {
        return;
    }
    if let Some(states) = states {
        copy_states_walk(&mut job.log, memory, base, &states);
        if job.log.overflowed {
            return;
        }
    }
    copy_construction_queue(&mut job.log, memory, base, game_state, &job.census);
    if job.log.overflowed {
        return;
    }
    copy_occupation_walk(
        &mut job.log,
        memory,
        base,
        &job.census,
        slots.as_ref(),
        &mut job.group_laws_required,
        &mut job.law_bytes_required,
        &mut job.law_demand_overflowed,
        &mut job.equipment_required,
    );
    if job.log.overflowed {
        return;
    }
    copy_collaboration_walk(
        &mut job.log,
        memory,
        base,
        &job.census,
        slots.as_ref().map(CountrySlots::links),
    );
}
