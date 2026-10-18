use super::*;

// ===========================================================================
// Scheduled production-rest raw-copy split (ProductionLines, Stockpiles,
// ProductionLicenses, DeploymentConveyors, DeploymentLines,
// DeploymentManpower, DeploymentEquipment, ProductionLicenseRecords).
//
// The acquisition worker copies the country census and per-country raw bytes, and
// the single FIFO decode worker replays the existing typed walks against the
// recorded spans (`crate::raw::Recorded`), so rows, ordinals, arenas,
// truncation and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules).
//
// One shared country census feeds all eight tables (no duplicated censuses);
// the production manager is shared by lines/stockpiles/licences/records and
// the deployment manager by the four deployment tables. Empty
// ProductionLicenseRecords shares the licence walk and moves with it (never
// observed nonempty on the test campaign, but the walk is shared so it is
// recorded, not proven trivial). Licence arenas (produced handles, conveyor
// names, line custom names) are staged decode-side: the log holds the
// underlying string/array bytes and the worker fills the pooled arenas by
// replaying the identical walks.
//
// Fault attribution is conservative: shared census spans carry
// all eight bits, production-manager spans the four production bits, and
// deployment spans all four deployment bits, so any live failure blocks its
// whole shared domain. Per-table leaves use
// single-bit masks. The mirror traverses structurally and records every
// discoverable span, never aborting except on overflow: the replay derives
// every outcome, precedence and error scope from the same recorded spans, so
// later extras are harmless.
// ===========================================================================

/// Fault-attribution bits in family order: [lines, stockpiles, licences,
/// conveyors, lines, manpower, equipment, records]. Shared census spans
/// carry all eight bits; production-manager spans the four production bits;
/// deployment spans all four deployment bits (the walk's shared failure
/// domain).
const M_P_LINES: u32 = 0x01;
const M_P_STOCK: u32 = 0x02;
const M_P_LIC: u32 = 0x04;
const M_P_REC: u32 = 0x80;
const M_P_PROD_MGR: u32 = 0x87;
const M_P_DEPLOY: u32 = 0x78;
const M_P_CENSUS: u32 = 0xFF;

/// One scheduled refresh of raw production-rest payload. Worker pools grow
/// from original request/header demand before publication and recycle storage.
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
        // Optional discovery tracing has its own bounded storage. Exhaustion
        // disables only plan compilation; the ordinary capture still completes.
        log.initialize_read_plan(0, 0, 0);
        let mut job = Self {
            // Reusable storage is advisory; worker copies grow from each
            // original span and the original country header.
            log,
            game_state: 0,
            census: Vec::new(),
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
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Prepare only observed shortage and live census demand off-thread.
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
pub struct ProductionRestCopyReport {
    /// False when staging overflowed or a prepared capture failed. The caller
    /// may use the typed walk only when plan_failed is also false.
    pub raw_ok: bool,
    /// Transcript disagreement or capacity exhaustion after prepared source
    /// observations. This capture must not trigger another live acquisition.
    pub plan_failed: bool,
    pub copy_ticks: [u64; 8],
    pub raw_bytes: [u64; 8],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub struct ProductionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn p_pool_u64(log: &RawLog<8>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn p_pool_u32(log: &RawLog<8>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn pspan(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator. Inline strings ride in the header span; the replay requests
/// exact ranges, so record the payload exactly like a heap one.
fn copy_production_string(log: &mut RawLog<8>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = pspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    // The walk reads length/capacity/pointer as separate 8-byte spans, not
    // just the 32-byte header above; record those exact spans too so
    // exact-match service hits. Extras are harmless.
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

/// Authenticated country census shared by all eight tables. Records the exact
/// read set of `ccountry::Reader::new` plus one full `read_at` per slot.
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
    let vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    if log.worker_growth_enabled()
        && engine_allocation::reserve(census, countries.count.saturating_sub(1) as usize).is_err()
    {
        log.overflowed = true;
        return false;
    }
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(pointer_start) = pspan(log, memory, pointer_addr, 8, M_P_CENSUS) else {
            return false;
        };
        let pointer = match usize::try_from(p_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(vtable_start) = pspan(log, memory, Some(pointer), 8, M_P_CENSUS) else {
            return false;
        };
        let Some(id_start) = pspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            M_P_CENSUS,
        ) else {
            return false;
        };
        if p_pool_u64(log, vtable_start) != vtable {
            return false;
        }
        let id = p_pool_u32(log, id_start);
        let canonical = match copy_production_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return false,
        };
        let canonical_slot = match copy_production_link(log, memory, &links, countries.count, slot)
        {
            Some(Some(canonical_slot)) => canonical_slot,
            _ => return false,
        };
        if canonical_slot != canonical {
            return false;
        }
        let check = match copy_production_link(log, memory, &links, countries.count, canonical) {
            Some(Some(check)) => check,
            _ => return false,
        };
        if check != canonical {
            return false;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(canonical_start) = pspan(log, memory, canonical_addr, 8, M_P_CENSUS) else {
            return false;
        };
        if p_pool_u64(log, canonical_start) != pointer as u64 {
            return false;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return false;
        }
        census.push(ProductionRestCountry { pointer });
    }
    true
}

fn copy_production_link(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = pspan(log, memory, links.data.checked_add(offset), 4, M_P_CENSUS)?;
    let slot = p_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Production manager for one country (shared by lines/stockpiles/licences/
/// records). Returns the manager pointer on success.
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
    let expected = base.checked_add(cproduction_status::VTABLE_RVA)? as u64;
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

/// Deployment manager for one country (shared by the four deployment tables).
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
    let expected = base.checked_add(cdeployment_status::VTABLE_RVA)? as u64;
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

/// Mirror of `walk_lines`' read set: array header, per-line pointers, the
/// eight scattered field spans per line plus variant identities, then each
/// line's maximum-factory and efficiency reads. `defines` holds the six
/// maximum-factory defines once a line of this pass has staged them; the walk
/// reads them once, at the first line that needs one.
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
        // Eight scattered spans exactly like `read_row`'s `copy_scattered`.
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
                    .and_then(|vtable| vtable.checked_sub(base))
                    .and_then(line_kind);
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

/// `read_row`'s line kind by exact primary vtable.
fn line_kind(rva: usize) -> Option<u8> {
    Some(match rva {
        engine_native::cmilitary_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_MILITARY
        }
        engine_native::cnaval_production_line::VTABLE_RVA => cequipment_production_line::KIND_NAVAL,
        engine_native::cship_refit_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_SHIP_REFIT
        }
        engine_native::crailway_gun_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_RAILWAY_GUN
        }
        _ => return None,
    })
}

/// Mirror of `read_row`'s maximum factories and the efficiency entries they
/// select. A failed or unparsable span stops here; the replay fails the same.
#[allow(clippy::too_many_arguments)]
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

/// Mirror of `cequipment_production_line::max_factories`: the variant's type
/// link and classifier, then the type's and its archetype's optionals, then
/// the defines the kind selects.
fn copy_max_factories(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    defines: &mut Option<[i32; 6]>,
) -> Option<i32> {
    use engine_native::cequipment_type;
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
    let expected = base.checked_add(cequipment_type::VTABLE_RVA)? as u64;
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

/// Stages the six maximum-factory defines at the first line that needs one;
/// the walk reads them together once.
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
            let start = pspan(log, memory, base.checked_add(rva), 4, M_P_LINES)?;
            *value = i32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
        }
        *defines = Some(values);
    }
    defines.map(|values| values[index])
}

/// Mirror of `walk_stockpiles`' read set: pool vtable, array header,
/// per-entry words plus variant identities.
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

/// Mirror of `walk_licenses`' read set: array header, per-licence pointers
/// plus variant identities.
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
        // Licence entries hold variant pointers directly; authenticate via identity.
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LIC);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `walk_license_records`' read set: array header, record heads,
/// parent identities, equipment headers plus per-handle pointers
/// and identities.
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

/// Mirror of the shared deployment walk's read set: conveyor array, per
/// conveyor heads plus templates, line arrays and names, per-line heads plus
/// name objects, custom strings, deployment objects with pools and manpower,
/// plus per-entry words and variant identities. All spans carry the combined
/// deployment mask, matching the walk's shared failure domain.
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
            // Template vtable plus reference words.
            let _ = pspan(log, memory, Some(t), 16, M_P_DEPLOY);
        }
        // Conveyor name string.
        if let Some(name) = conveyor.checked_add(cmilitary_deployment_conveyor::NAME) {
            copy_production_string(log, memory, name, M_P_DEPLOY);
        }
        // Line array header as its own exact span: the walk reads it via
        // `Array::read`, which exact-match service does not derive from
        // the head span above.
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
    // Name object: vtable, order, override header, flags; then custom bytes.
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
    // Parent backlink is inside the head span already; deployment object next.
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
    // Equipment pool arrays.
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
            // Allocation entries are counted, not decoded per row here; record
            // the contiguous block as individual entry spans via takes below.
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, 24);
                let _ = pspan(log, memory, addr, 24, M_P_DEPLOY);
            }
        }
    }
    // Scalars.
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
    // Manpower containers.
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

/// Acquire one owned payload and report its per-table costs.
#[cfg(any(test, feature = "test-support"))]
pub fn copy_production_rest_refresh(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
) -> ProductionRestCopyReport {
    copy_production_rest(memory, ctx, job, false)
}

pub fn copy_production_rest_refresh_growing(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
) -> ProductionRestCopyReport {
    copy_production_rest(memory, ctx, job, true)
}

fn copy_production_rest(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
    growing: bool,
) -> ProductionRestCopyReport {
    let started = qpc();
    job.clear();
    if growing {
        job.log.enable_worker_growth();
    }
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
    /// Finish an accepted acquisition failure without retrying live memory.
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

/// Reuse only the discovery program. Every successful replay acquires fresh
/// payload bytes, and its controls must match before a dependent address is
/// read. A changed control resumes the producer with the already-observed
/// ordinal prefix; it never rereads that prefix or a failed source.
fn acquire_production_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ProductionRestJob,
) {
    let _reads = read_accounting::table(TABLES[0]);
    // These are the producer's external inputs. Output-table capacities
    // do not select its reads; the raw producer captures a superset.
    // Census capacity does affect admission, so it belongs to the plan key.
    let key = [base, game_state, job.census.capacity(), 1];
    if !job.log.begin_planned_capture(memory, key) {
        copy_production_rest_pass(memory, base, game_state, job);
    }
    job.log.finish_planned_capture();
}

/// One structural copy pass: shared census plus every country's production
/// and deployment takes. The job is cleared on entry so pooled refreshes
/// never accumulate spans across hours.
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
