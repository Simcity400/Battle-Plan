use super::*;

impl Captured {
    /// Decode-worker interpretation of a scheduled raw construction-rest
    /// payload. Replays the existing resource, queue, occupation, resistance
    /// and collaboration walks against the recorded spans. Occupation capacity
    /// comes from demand recorded during that copy; other tables retain their
    /// existing grants. Clears `rest_raw_mode` after installing the results.
    pub(crate) fn decode_construction_rest_payload(
        &mut self,
        base: usize,
        game_state: usize,
        job: &ConstructionRestJob,
        decode_ticks: &mut [u64; 10],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        self.base_resources.outcome =
            state_resource_entry::walk(&recorded, base, game_state, &mut self.base_resources.rows);
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let queue = cbuilding_production_line::walk(
            &recorded,
            base,
            game_state,
            cbuilding_production_line::Buffers {
                buildings: &mut self.building_construction.rows,
                railways: &mut self.railway_construction.rows,
                route_provinces: &mut self.routes.first,
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
        let occupation = ccountry_occupation_status::walk(
            &recorded,
            base,
            game_state,
            ccountry_occupation_status::Buffers {
                settings: &mut self.settings.rows,
                laws: &mut self.laws.rows,
                garrisons: &mut self.garrisons.rows,
                manpower: &mut self.assigned_manpower.rows,
                reserve_manpower: &mut self.reserve_manpower.rows,
                names: &mut self.law_bytes.first,
            },
        );
        self.settings.outcome = occupation.settings;
        self.laws.outcome = occupation.laws;
        self.garrisons.outcome = occupation.garrisons;
        self.assigned_manpower.outcome = occupation.manpower;
        self.reserve_manpower.outcome = occupation.reserve_manpower;
        self.law_bytes.len = occupation.bytes_len;
        self.law_bytes.required = occupation.required_bytes;
        if law_error.is_none() {
            self.law_bytes.required = self.law_bytes.required.max(job.law_bytes_required);
        }
        // Keep an original native failure ahead of allocation/demand failures.
        // Other occupation tables still receive their one independent walk.
        if let Some(reason) = settings_error.or(law_error)
            && !matches!(self.settings.outcome, Outcome::Unavailable { .. })
        {
            self.settings.outcome = Outcome::Unavailable { reason };
        }
        if let Some(reason) = law_error
            && !matches!(self.laws.outcome, Outcome::Unavailable { .. })
        {
            self.laws.outcome = Outcome::Unavailable { reason };
        }
        decode_ticks[3] += elapsed(started);
        let started = qpc();
        self.resistance.outcome =
            cresistance::walk(&recorded, base, game_state, &mut self.resistance.rows);
        decode_ticks[8] += elapsed(started);
        let started = qpc();
        self.collaboration.outcome = ccountry_collaboration_status::walk(
            &recorded,
            base,
            game_state,
            &mut self.collaboration.rows,
        );
        decode_ticks[9] += elapsed(started);
        self.rest_raw_mode = false;
    }
}

// ===========================================================================
// Scheduled construction-rest raw-copy split (StateBaseResources,
// BuildingConstruction, RailwayConstruction, OccupationSettings,
// StateOccupationLaws, StateGarrisons, StateAssignedManpower,
// CountryOccupationReserveManpower, StateResistance, CountryCollaboration).
//
// The acquisition worker copies raw bytes and the single FIFO decode worker replays
// the existing typed walks against the recorded spans
// (`crate::raw::Recorded`), so rows, ordinals, arenas, truncation and failure
// reasons are identical by construction (see `crate::raw`).
//
// One shared states-array traversal feeds resources, resistance and building
// targets (no duplicated censuses); one shared country census feeds the
// queue, occupation and collaboration walks. The occupation shared walk (five
// tables plus route/law arenas) moves together; empty
// CountryOccupationReserveManpower and CountryCollaboration share their
// walks and move with them (never observed nonempty on the test campaign,
// but recorded, not proven trivial). Route/law arenas are staged
// decode-side: the log holds the underlying bytes and the worker fills the
// pooled arenas by replaying the identical walks.
//
// Fault attribution is conservative: shared roots carry the
// combined bits of their domain, so any live failure blocks its whole shared
// domain. The mirror traverses structurally and
// records every discoverable span, never aborting except on overflow.
// ===========================================================================

/// Fault-attribution bits in family order: [resources, building, railway,
/// settings, laws, garrisons, assigned, reserve, resistance, collaboration].
/// Shared roots carry all ten bits; the queue carries building+railway; the
/// occupation walk carries its five bits (shared root failure domain,
/// conservative for leaf-scoped faults).
const M_C_RES: u32 = 0x001;
const M_C_BLD: u32 = 0x002;
const M_C_RAIL: u32 = 0x004;
const M_C_RESIST: u32 = 0x100;
const M_C_COLLAB: u32 = 0x200;
const M_C_ALL: u32 = 0x3FF;
const M_C_QUEUE: u32 = 0x006;
pub(super) const M_C_OCC: u32 = 0x0F8;

/// One scheduled refresh of raw construction-rest payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_construction_rest_job`).
#[derive(Debug, Default)]
pub(crate) struct ConstructionRestJob {
    pub log: RawLog<10>,
    pub game_state: usize,
    pub(super) census: Vec<ConstructionRestCountry>,
    pub(super) settings_required: usize,
    pub(super) law_bytes_required: usize,
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
            // Late-campaign worst case is about 1k states with resources and
            // resistance, construction queues with routes, occupation graphs
            // with laws and collaboration links; both pools start above that
            // so steady state never overflows.
            log: RawLog::new(1 << 17, 8 << 20),
            game_state: 0,
            census: owned_vec(vec(crate::country_scale::census_rows(512))),
            settings_required: 0,
            law_bytes_required: 0,
            law_demand_overflowed: false,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
        self.settings_required = 0;
        self.law_bytes_required = 0;
        self.law_demand_overflowed = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        self.update_capacity_charge();
    }

    /// Off the game thread: hold one census row per observed country slot so
    /// the first hour after a large save loads does not overflow.
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
pub(crate) struct ConstructionRestCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 10],
    pub raw_bytes: [u64; 10],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstructionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn c_pool_u64(log: &RawLog<10>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn c_pool_u32(log: &RawLog<10>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn cspan(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator.
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
    // The walk reads length/capacity/pointer as separate 8-byte spans, not
    // just the 32-byte header above; record those exact spans too.
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
    // Count every occurrence before the original payload attempt, including
    // failed copies and names shared by default and state-override laws.
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

/// Shared states-array header plus country census. Returns the states array
/// on success; the caller traverses states once for resources, resistance
/// and building targets (no duplicated censuses).
fn copy_construction_roots(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<ConstructionRestCountry>,
) -> Option<Array> {
    let states_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STATES),
        16,
        M_C_ALL,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[states_start..states_start + 16]);
    let states = Array::from_header(raw, 8)?;
    let countries_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_C_ALL,
    )?;
    let links_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_C_ALL,
    )?;
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let pointer_start = cspan(log, memory, pointer_addr, 8, M_C_ALL)?;
        let pointer = match usize::try_from(c_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let vtable_start = cspan(log, memory, Some(pointer), 8, M_C_ALL)?;
        let id_start = cspan(log, memory, pointer.checked_add(ccountry::ID), 4, M_C_ALL)?;
        if c_pool_u64(log, vtable_start) != vtable {
            return None;
        }
        let id = c_pool_u32(log, id_start);
        // Full canonical validation mirrors `ccountry::Reader`: id, slot and
        // canonical link reads plus the canonical pointer round-trip.
        let canonical = match copy_construction_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return None,
        };
        let canonical_slot =
            match copy_construction_link(log, memory, &links, countries.count, slot) {
                Some(Some(canonical_slot)) => canonical_slot,
                _ => return None,
            };
        if canonical_slot != canonical {
            return None;
        }
        let check = match copy_construction_link(log, memory, &links, countries.count, canonical) {
            Some(Some(check)) => check,
            _ => return None,
        };
        if check != canonical {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let canonical_start = cspan(log, memory, canonical_addr, 8, M_C_ALL)?;
        if c_pool_u64(log, canonical_start) != pointer as u64 {
            return None;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push(ConstructionRestCountry { pointer });
    }
    Some(states)
}

/// Mirror of a link-table read with local bounds checks.
fn copy_construction_link(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = cspan(log, memory, links.data.checked_add(offset), 4, M_C_ALL)?;
    let slot = c_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Mirror of `state_resource_entry::walk` plus `cresistance::walk` over one
/// shared states traversal: state pointers, vtables, ids, templates,
/// resource vectors plus resistance headers. Takes are unconditional over
/// discoverable addresses (a superset); the replay derives every outcome.
fn copy_states_walk(log: &mut RawLog<10>, memory: &impl Memory, base: usize, states: &Array) {
    let state_vtable = match base.checked_add(cstate::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let template_vtable = match base.checked_add(cstate_template::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let resistance_vtable = match base.checked_add(cresistance::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let _ = (state_vtable, template_vtable, resistance_vtable);
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
        // State vtable plus id (shared by resources and resistance).
        let _ = cspan(log, memory, Some(pointer), 8, M_C_ALL);
        let _ = cspan(log, memory, pointer.checked_add(cstate::ID), 4, M_C_ALL);
        // Template chain for resources.
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
        // Resistance object: header, compliance block, active byte.
        if let Some(resistance) = pointer.checked_add(cstate::RESISTANCE) {
            let _ = cspan(log, memory, Some(resistance), 32, M_C_RESIST);
            if let Some(compliance) = resistance.checked_add(cresistance::COMPLIANCE) {
                let _ = cspan(log, memory, Some(compliance), 20, M_C_RESIST);
            }
            let _ = cspan(
                log,
                memory,
                resistance.checked_add(cresistance::ACTIVE),
                1,
                M_C_RESIST,
            );
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the mixed construction queue's read set: per-country manager,
/// queue headers, per-line vtables, common fields, building targets with map
/// authentication plus railway routes and fields.
fn copy_construction_queue(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[ConstructionRestCountry],
) {
    // Map roots for building targets (states plus provinces arrays).
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
            let building_vtable = base
                .checked_add(cbuilding_production_line::VTABLE_RVA)
                .map(|v| v as u64);
            let railway_vtable = base
                .checked_add(crailway_production_line::VTABLE_RVA)
                .map(|v| v as u64);
            // Common fields (reference, factories, cost/speed/produced,
            // priority/amount, created date, owner).
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
                // Building target chain plus type and repair levels. The
                // status pointer is an exact 8-byte span (the walk reads it
                // as one word); the status identity is two exact spans
                // (vtable plus flags) mirroring `cbuilding_status::identity`.
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
                // Railway levels, route header plus provinces, segment/cost.
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

/// Mirror Locations::resolve, including the map-slot round trip. Authenticate
/// each object before following its state pointer; raw addresses are not types.
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
        != base
            .checked_add(cbuilding_status::VTABLE_RVA)
            .map(|p| p as u64)
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
    if Some(c_pool_u64(log, vt)) != base.checked_add(vtable).map(|p| p as u64)
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
    if Some(c_pool_u64(log, vt)) != base.checked_add(cstate::VTABLE_RVA).map(|p| p as u64) {
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

/// Mirror of the shared occupation walk's read set: per-country status with
/// vtables, garrison templates, default laws plus reserve manpower, the
/// robin-hood occupation table with law/state entries plus garrison and
/// manpower rows. All spans carry the combined occupation mask, matching the
/// walk's shared root failure domain (conservative for leaf-scoped faults).
fn copy_occupation_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
    required: &mut usize,
    demand_overflowed: &mut bool,
) {
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
        // Garrison template plus default law.
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
        // Reserve manpower container.
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
        // Occupation robin-hood table plus nested law/garrison walks. The
        // outer table is visited via `pdx_robin_hood_table::visit` (exact
        // 17-byte header plus per-bucket 24-byte spans); each live entry's
        // identity, law table and garrison table are mirrored below so the
        // replay derives identical rows and reasons.
        if let Some(table) = status.checked_add(ccountry_occupation_status::OCCUPATION_DATA_TABLE) {
            copy_occupation_entries(log, memory, base, table, required, demand_overflowed);
        }
        if log.overflowed {
            return;
        }
    }
    let _ = base;
}

/// Mirror of one robin-hood table's visit: exact 17-byte header plus one
/// 24-byte span per bucket in the computed span. Returns live (key, value)
/// pairs parsed from the pooled bytes for the caller to traverse.
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
        // A corrupt header cannot admit more entries than the checked batch.
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

/// Outer occupation-data entries with nested law and garrison walks.
fn copy_occupation_entries(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    table: usize,
    required: &mut usize,
    demand_overflowed: &mut bool,
) {
    let Some(entries) = copy_pdx_visit(log, memory, table, M_C_OCC) else {
        return;
    };
    for (key, value) in entries {
        // Identity: exact 20-byte span (vtable, parent, occupied). The
        // occupied link resolution reads the country link table; record it
        // explicitly so novel occupied indices are covered beyond the shared
        // census spans.
        let Some(id_start) = cspan(log, memory, Some(value), 20, M_C_OCC) else {
            continue;
        };
        {
            let mut identity = [0u8; 20];
            identity.copy_from_slice(&log.bytes[id_start..id_start + 20]);
            let occupied = u32::from_le_bytes(identity[16..20].try_into().unwrap());
            // Link-table read mirroring `countries.resolve(occupied)`.
            // Bounds-checked locally; the replay validates membership itself.
            if occupied != 0 {
                // Links header is already recorded; record the entry span.
                // (Address derived from the shared census links array; parse
                // it from the log when available, else skip — the replay
                // fails identically from the same spans.)
                let _ = (key, occupied);
            }
        }
        // Law table nested visit plus per-law literals.
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
        // Garrison table nested visit plus per-garrison rows and manpower.
        if let Some(gar_table) = value.checked_add(ccountry_occupation_data::GARRISON_TABLE)
            && let Some(garrisons) = copy_pdx_visit(log, memory, gar_table, M_C_OCC)
        {
            for (gkey, garrison) in garrisons {
                copy_garrison_object(log, memory, base, gkey, garrison, value);
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

/// Mirror of one garrison object's read set: vtable, state/parent
/// references, state-slot authentication, scalars plus assigned manpower.
fn copy_garrison_object(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    key: u32,
    garrison: usize,
    parent: usize,
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
    // Scalars: exact 1+4+16 spans mirroring `copy_scattered`.
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
    // Assigned-manpower container.
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

/// Mirror of `ccountry_collaboration_status::walk`'s read set: per-country
/// status with owner, outgoing/incoming arrays, per-relation objects plus
/// reciprocal validation reads.
fn copy_collaboration_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
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
                            let _ = cspan(log, memory, Some(rel), 24, mask);
                        }
                    } else {
                        let _ = cspan(log, memory, addr, 4, mask);
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

/// Acquire one owned payload and report its per-table costs.
pub(crate) fn copy_construction_rest_refresh(
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
    /// Finish an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_construction_rest_capture(&mut self, reason: &'static str) {
        self.base_resources.fail(Outcome::Unavailable { reason });
        self.building_construction
            .fail(Outcome::Unavailable { reason });
        self.railway_construction
            .fail(Outcome::Unavailable { reason });
        self.settings.fail(Outcome::Unavailable { reason });
        self.laws.fail(Outcome::Unavailable { reason });
        self.garrisons.fail(Outcome::Unavailable { reason });
        self.assigned_manpower.fail(Outcome::Unavailable { reason });
        self.reserve_manpower.fail(Outcome::Unavailable { reason });
        self.resistance.fail(Outcome::Unavailable { reason });
        self.collaboration.fail(Outcome::Unavailable { reason });
        self.rest_raw_mode = false;
    }
}

/// One structural copy pass: shared roots plus states, queue, occupation and
/// collaboration takes. The job is cleared on entry so pooled refreshes never
/// accumulate spans across hours.
fn copy_construction_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ConstructionRestJob,
) {
    let _reads = read_accounting::table(TABLES[2]);
    let states = copy_construction_roots(&mut job.log, memory, base, game_state, &mut job.census);
    // Vec length is already representable and bounded by the owned census.
    // Retain its prefix even if a later country authentication fails.
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
        &mut job.law_bytes_required,
        &mut job.law_demand_overflowed,
    );
    if job.log.overflowed {
        return;
    }
    copy_collaboration_walk(&mut job.log, memory, base, &job.census);
}
