//! Domain 60 live extraction: country capitulation state + war pool.
//!
//! Shape mirrors the domain 53 / naval 49-50 discovery-cache pattern:
//! - discovery (cached, background-refreshed by the collector, generation
//!   guarded): ONE heap pass finds the two war date-object vtables and keeps
//!   only fully validated war-row addresses (tag pair in the live country set,
//!   echo pair, casualty + start-date range checks — the proven p06 filter).
//! - per frame: country roots are re-resolved, every cached war-row address is
//!   REVALIDATED (vtable + full row re-read, values never cached), and the
//!   capitulation block is read fresh off every live CCountry.
//! - fail closed: torn reads skip; any skip fails the whole frame closed
//!   (collector emits an empty PARTIAL record). A stale MAJORITY of cached war
//!   rows degrades discovery (`CAPWAR_DISCOVERY_DEGRADED`) so the collector
//!   rescans synchronously instead of emitting a silently shrunken war set.

use crate::compact::{
    date_raw_valid_nullable, war_start_date_raw_valid, CapitulationWarSnapshot,
    CountryCapitulationRow, HostilityReason, WarResiduals, WarRow, MAX_CASUALTIES,
    MAX_EXILE_ARMY_LEADERS, MAX_LEGITIMACY_RAW, MAX_TAG_DB_ID,
};
use compact_contract::build_profile as profile;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    MemoryRead { address: u64, len: usize },
    InvalidRoot(&'static str),
    InvalidCount { name: &'static str, value: u32 },
    Validation(String),
}

pub const CAPWAR_DISCOVERY_DEGRADED: &str = "capwar_discovery_degraded";

const RTTI_CCOUNTRY: &str = "CCountry";
const CAPWAR_RTTI_MAX_NAME_LEN: usize = 200;
const CAPWAR_MODULE_SPAN_MAX: u64 = 0x5000_0000;
/// Legacy bytes read while the H-B5 live source gate remains disabled.
const WAR_ROW_READ_LEN: usize = 0x70;
/// Capitulation block span: `D+CAP_BLOCK_BASE .. D+CAP_BLOCK_BASE+CAP_BLOCK_LEN`
/// covers legitimacy (+0x1B0) through capitulated_date (+0x2F0..+0x2F4).
const CAP_BLOCK_BASE: usize = profile::DOMAIN60_DIPLOMACY_LEGITIMACY;
const CAP_BLOCK_LEN: usize =
    profile::DOMAIN60_DIPLOMACY_CAPITULATED_DATE + 4 - profile::DOMAIN60_DIPLOMACY_LEGITIMACY;

pub trait CapWarMemory {
    fn module_base(&self) -> u64;
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError>;

    fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
        Vec::new()
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("u32 bytes")))
    }

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().expect("u64 bytes")))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapWarDiscovery {
    /// Validated war-row base addresses (dedup by address, sorted).
    pub war_row_addrs: Vec<u64>,
    pub scan_range_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedCapWarFrame {
    pub countries: Vec<CountryCapitulationRow>,
    pub wars: Vec<WarRow>,
    pub observed_rows: u32,
    pub skipped_rows: u32,
    pub stale_dropped: u32,
    pub country_roots: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountryRoot {
    tag_db_id: u32,
    country_ptr: u64,
}

struct FrameRoots {
    module_base: u64,
    countries: Vec<CountryRoot>,
}

fn add_addr(base: u64, offset: usize) -> Result<u64, ExtractError> {
    base.checked_add(offset as u64)
        .ok_or(ExtractError::InvalidRoot("capwar_address_overflow"))
}

fn is_heap_ptr(value: u64) -> bool {
    value >= profile::DOMAIN53_HEAP_LO as u64 && value < profile::DOMAIN53_HEAP_HI as u64
}

fn in_module(module_base: u64, value: u64) -> bool {
    (module_base..module_base.saturating_add(CAPWAR_MODULE_SPAN_MAX)).contains(&value)
}

fn valid_tag_db_id(value: u32) -> bool {
    // R2 gate finding 2: callers include SILENT skips (root-roster `continue`
    // arms), so an over-ceiling tag must record its bound trip — a >1024-tag
    // overhaul would otherwise silently drop whole country roots.
    value > 0
        && compact_contract::bound_trips::gate_u32(
            value,
            MAX_TAG_DB_ID,
            compact_contract::bound_trips::BOUND_TRIP_TAG_DB_ID_1024,
        )
        .is_some()
}

fn war_date_vtables(module_base: u64) -> Result<[u64; 2], ExtractError> {
    Ok([
        add_addr(module_base, profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA)?,
        add_addr(module_base, profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA)?,
    ])
}

// --- build identity (RTTI) --------------------------------------------------

fn verify_build_identity<M: CapWarMemory>(mem: &M, module_base: u64) -> Result<(), ExtractError> {
    verify_vtable_rtti_name(
        mem,
        module_base,
        profile::DOMAIN53_COUNTRY_VTABLE_RVA,
        RTTI_CCOUNTRY,
    )
}

fn verify_vtable_rtti_name<M: CapWarMemory>(
    mem: &M,
    module_base: u64,
    vtable_rva: usize,
    expected_undecorated: &str,
) -> Result<(), ExtractError> {
    let vtable = add_addr(module_base, vtable_rva)?;
    let decorated = rtti_type_name_for_vtable(mem, module_base, vtable)?;
    let Some(undecorated) = undecorate_msvc_class_name(&decorated) else {
        return Err(ExtractError::Validation(format!(
            "capwar_rtti_name_not_class:{decorated}"
        )));
    };
    if undecorated != expected_undecorated {
        return Err(ExtractError::Validation(format!(
            "capwar_rtti_name_mismatch:{undecorated}!={expected_undecorated}"
        )));
    }
    Ok(())
}

fn rtti_type_name_for_vtable<M: CapWarMemory>(
    mem: &M,
    module_base: u64,
    vtable: u64,
) -> Result<String, ExtractError> {
    if !in_module(module_base, vtable) {
        return Err(ExtractError::InvalidRoot(
            "capwar_rtti_vtable_out_of_module",
        ));
    }
    let col_slot = vtable
        .checked_sub(8)
        .ok_or(ExtractError::InvalidRoot("capwar_rtti_vtable_underflow"))?;
    let col = mem.read_u64(col_slot)?;
    if !in_module(module_base, col) {
        return Err(ExtractError::InvalidRoot("capwar_rtti_col_out_of_module"));
    }
    let col_bytes = mem.read_bytes(col, 24)?;
    let signature = u32::from_le_bytes(col_bytes[0..4].try_into().expect("u32 bytes"));
    let type_descriptor_rva = u32::from_le_bytes(col_bytes[0x0C..0x10].try_into().expect("u32"));
    let class_descriptor_rva = u32::from_le_bytes(col_bytes[0x10..0x14].try_into().expect("u32"));
    let self_rva = u32::from_le_bytes(col_bytes[0x14..0x18].try_into().expect("u32"));
    if signature > 1 {
        return Err(ExtractError::InvalidRoot("capwar_rtti_col_signature"));
    }
    if signature == 1 && module_base.saturating_add(u64::from(self_rva)) != col {
        return Err(ExtractError::InvalidRoot("capwar_rtti_col_self_mismatch"));
    }
    if type_descriptor_rva == 0 || class_descriptor_rva == 0 {
        return Err(ExtractError::InvalidRoot("capwar_rtti_col_missing_refs"));
    }
    let type_descriptor = module_base
        .checked_add(u64::from(type_descriptor_rva))
        .ok_or(ExtractError::InvalidRoot(
            "capwar_rtti_type_descriptor_overflow",
        ))?;
    if !in_module(module_base, type_descriptor) {
        return Err(ExtractError::InvalidRoot(
            "capwar_rtti_type_descriptor_out_of_module",
        ));
    }
    let bytes = mem.read_bytes(add_addr(type_descriptor, 0x10)?, CAPWAR_RTTI_MAX_NAME_LEN)?;
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(ExtractError::InvalidRoot("capwar_rtti_name_not_terminated"));
    };
    let value = std::str::from_utf8(&bytes[..end])
        .map_err(|_| ExtractError::InvalidRoot("capwar_rtti_name_invalid_utf8"))?;
    if !value.starts_with(".?A") || value.chars().any(char::is_control) {
        return Err(ExtractError::InvalidRoot("capwar_rtti_name_invalid"));
    }
    Ok(value.to_string())
}

fn undecorate_msvc_class_name(name: &str) -> Option<&str> {
    name.strip_prefix(".?AV")?.strip_suffix("@@")
}

// --- roots -------------------------------------------------------------------

fn resolve_frame_roots<M: CapWarMemory>(mem: &M) -> Result<FrameRoots, ExtractError> {
    let module_base = mem.module_base();
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }
    let game_state = mem.read_u64(add_addr(module_base, profile::CG_STATE_DATE_OFFSET)?)?;
    let country_database =
        mem.read_u64(add_addr(module_base, profile::COUNTRY_DATABASE_OFFSET)?)?;
    if !is_heap_ptr(game_state) || !is_heap_ptr(country_database) {
        return Err(ExtractError::InvalidRoot("capwar_global_roots_unavailable"));
    }
    verify_build_identity(mem, module_base)?;
    let countries = resolve_country_roots(mem, module_base, game_state)?;
    if countries.is_empty() {
        return Err(ExtractError::InvalidRoot(
            "capwar_country_roots_empty_implausible",
        ));
    }
    Ok(FrameRoots {
        module_base,
        countries,
    })
}

fn resolve_country_roots<M: CapWarMemory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
) -> Result<Vec<CountryRoot>, ExtractError> {
    let country_table = mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE,
    )?)?;
    let country_count = mem.read_u32(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT,
    )?)?;
    if country_count == 0 || country_count > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS {
        return Err(ExtractError::InvalidCount {
            name: "capwar_country_count",
            value: country_count,
        });
    }
    if !is_heap_ptr(country_table) {
        return Err(ExtractError::InvalidRoot(
            "capwar_country_table_unavailable",
        ));
    }
    let expected_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for slot in 0..country_count {
        let slot_addr = country_table
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("capwar_country_table_overflow"))?;
        let Ok(country_ptr) = mem.read_u64(slot_addr) else {
            continue;
        };
        if country_ptr == 0 || !is_heap_ptr(country_ptr) {
            continue;
        }
        let Ok(vtable) = mem.read_u64(country_ptr) else {
            continue;
        };
        if vtable != expected_vtable {
            continue;
        }
        let Ok(tag_db_id) = mem.read_u32(add_addr(country_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        else {
            continue;
        };
        if !valid_tag_db_id(tag_db_id) || !seen.insert(tag_db_id) {
            continue;
        }
        roots.push(CountryRoot {
            tag_db_id,
            country_ptr,
        });
    }
    Ok(roots)
}

// --- discovery ----------------------------------------------------------------

/// One heap pass scanning BOTH war date-object vtables, keeping only addresses
/// that fully validate as war rows against the live country set.
pub fn discover_war_rows<M: CapWarMemory>(mem: &M) -> Result<CapWarDiscovery, ExtractError> {
    let roots = resolve_frame_roots(mem)?;
    let module_base = roots.module_base;
    let valid_tags: BTreeSet<u32> = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect();
    let date_vtables = war_date_vtables(module_base)?;
    let ranges = mem.readable_ranges(
        profile::DOMAIN53_HEAP_LO as u64,
        profile::DOMAIN53_HEAP_HI as u64,
    );
    let hits = scan_qword_hits_multi(mem, &ranges, &date_vtables)?;
    let mut war_row_addrs = BTreeSet::new();
    for hit in hits {
        let Some(row_addr) = hit.checked_sub(profile::DOMAIN53_WAR_DATE_VTABLE as u64) else {
            continue;
        };
        if read_war_row(mem, row_addr, &date_vtables, &valid_tags).is_ok() {
            war_row_addrs.insert(row_addr);
        }
    }
    Ok(CapWarDiscovery {
        war_row_addrs: war_row_addrs.into_iter().collect(),
        scan_range_count: ranges.len().min(u32::MAX as usize) as u32,
    })
}

/// P3 shared-sweep migration: the two absolute war date-object vtables the
/// discovery scans for (`[gregorian, game_date]`). The collector registers both
/// with the shared background `heap_sweep` service.
pub fn capwar_war_date_vtables(module_base: u64) -> Result<[u64; 2], ExtractError> {
    war_date_vtables(module_base)
}

/// P3 shared-sweep migration: build a [`CapWarDiscovery`] from raw scan hits
/// produced by the shared sweep service (the UNION of the two war date-vtable
/// needles from [`capwar_war_date_vtables`]), instead of running the full-heap
/// scan inline on the frame thread. This is the exact post-scan validation loop
/// from [`discover_war_rows`]: each hit is turned into a candidate row address
/// and kept only if it fully validates as a war row against the live country
/// set. Fails closed if the raw hit count is implausibly large (corrupt heap
/// enumeration), matching the inline scan's ceiling.
pub fn discover_war_rows_from_raw_hits<M: CapWarMemory>(
    mem: &M,
    raw_hits: &[u64],
) -> Result<CapWarDiscovery, ExtractError> {
    if raw_hits.len() > MAX_RAW_SCAN_HITS {
        return Err(ExtractError::InvalidRoot("capwar_raw_scan_hits_exceeded"));
    }
    let roots = resolve_frame_roots(mem)?;
    let module_base = roots.module_base;
    let valid_tags: BTreeSet<u32> = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect();
    let date_vtables = war_date_vtables(module_base)?;
    let mut war_row_addrs = BTreeSet::new();
    for &hit in raw_hits {
        let Some(row_addr) = hit.checked_sub(profile::DOMAIN53_WAR_DATE_VTABLE as u64) else {
            continue;
        };
        if read_war_row(mem, row_addr, &date_vtables, &valid_tags).is_ok() {
            war_row_addrs.insert(row_addr);
        }
    }
    Ok(CapWarDiscovery {
        war_row_addrs: war_row_addrs.into_iter().collect(),
        scan_range_count: 0,
    })
}

/// Ceiling on raw (pre-validation) scan hits; the CGameDate vtable is a very
/// common heap object (~1.5M live hits measured on D04). Beyond this the scan
/// is implausible and fails closed rather than ballooning memory.
const MAX_RAW_SCAN_HITS: usize = 8_000_000;

/// Scan committed ranges once for every needle in `needles` (aligned qwords).
///
/// A chunk read that fails or comes back short means the range changed
/// mid-scan (heap churn while the enumerated region list is stale). Following
/// the proven naval 49/50 scan semantics, the REST OF THAT RANGE is skipped
/// instead of failing the whole discovery: cached rows are revalidated every
/// frame and the stale-majority gate catches a genuinely broken cache, while
/// a row missed here is picked up by the next background refresh.
fn scan_qword_hits_multi<M: CapWarMemory>(
    mem: &M,
    ranges: &[(u64, usize)],
    needles: &[u64],
) -> Result<Vec<u64>, ExtractError> {
    let needle_bytes: Vec<[u8; 8]> = needles.iter().map(|needle| needle.to_le_bytes()).collect();
    let mut hits = Vec::new();
    const CHUNK: usize = 1 << 20;
    // Chunk overlap so an aligned needle straddling a chunk boundary is never
    // missed (16 is a multiple of 8, preserving qword alignment; duplicate
    // hits are deduped by the caller's address set).
    const OVERLAP: usize = 16;
    for (start, len) in ranges {
        let Some(end) = start.checked_add(*len as u64) else {
            continue;
        };
        let mut pos = *start;
        while pos < end {
            let remaining = (end - pos).min(usize::MAX as u64) as usize;
            let want = remaining.min(CHUNK);
            let Ok(blob) = mem.read_bytes(pos, want) else {
                // Range vanished mid-scan: skip its remainder (heap churn).
                break;
            };
            let mut offset = (8usize.wrapping_sub((pos as usize) % 8)) % 8;
            while offset + 8 <= blob.len() {
                let window: &[u8] = &blob[offset..offset + 8];
                if needle_bytes.iter().any(|needle| window == needle) {
                    hits.push(pos + offset as u64);
                    if hits.len() > MAX_RAW_SCAN_HITS {
                        return Err(ExtractError::InvalidRoot("capwar_scan_truncated"));
                    }
                }
                offset += 8;
            }
            if blob.len() < want {
                // Short read: same churn treatment as a failed read.
                break;
            }
            if blob.len() >= remaining {
                break;
            }
            pos = pos
                .checked_add(blob.len().saturating_sub(OVERLAP).max(1) as u64)
                .ok_or(ExtractError::InvalidRoot("capwar_scan_overflow"))?;
        }
    }
    Ok(hits)
}

// --- per-frame extraction ------------------------------------------------------

pub fn extract_frame_with_discovery<M: CapWarMemory>(
    mem: &M,
    discovery: &CapWarDiscovery,
) -> Result<ExtractedCapWarFrame, ExtractError> {
    let roots = resolve_frame_roots(mem)?;
    let module_base = roots.module_base;
    let date_vtables = war_date_vtables(module_base)?;
    let valid_tags: BTreeSet<u32> = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect();

    let expected_country_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;

    let mut observed_rows = 0u32;
    let mut skipped_rows = 0u32;

    // Country capitulation state: fresh read off every live CCountry.
    let mut countries = Vec::new();
    for country in &roots.countries {
        match read_country_capitulation(mem, country, expected_country_vtable) {
            Ok(Some(row)) => {
                countries.push(row);
                observed_rows = observed_rows.saturating_add(1);
            }
            // Validated absence: the diplomacy sub-object pointer slot read 0.
            Ok(None) => {}
            // Torn/implausible read: fail the frame closed via skip counting.
            Err(_) => skipped_rows = skipped_rows.saturating_add(1),
        }
    }

    // War rows: revalidate every cached address, re-read values fresh.
    //
    // Two distinct failure modes, treated DIFFERENTLY so a "clean" frame can
    // never silently drop a genuine war row:
    //  - a TORN read (`MemoryRead`): the row's memory was momentarily
    //    unreadable. The row may well still be a live war we just failed to
    //    read — dropping it silently would understate the war set on an
    //    otherwise-clean frame. Counted as a per-row SKIP, which fails the
    //    whole frame closed (the collector emits an empty PARTIAL record and
    //    the fold retains the previous state), exactly like a torn country row.
    //  - a STRUCTURALLY-INVALID row (`InvalidRoot`/`Validation`): the cached
    //    address no longer points at a valid war row (the war was retired or
    //    the pool slot was reused). This is EXPECTED heap churn between
    //    background refreshes; counted as `stale_dropped` and gated by the
    //    stale-majority test, never failing a frame on a minority.
    let mut wars = Vec::new();
    let mut seen_pairs = BTreeSet::new();
    let mut stale_dropped = 0u32;
    for row_addr in &discovery.war_row_addrs {
        match read_war_row(mem, *row_addr, &date_vtables, &valid_tags) {
            Ok(row) => {
                let key = if row.first_tag_db_id <= row.second_tag_db_id {
                    (row.first_tag_db_id, row.second_tag_db_id)
                } else {
                    (row.second_tag_db_id, row.first_tag_db_id)
                };
                if seen_pairs.insert(key) {
                    wars.push(row);
                    observed_rows = observed_rows.saturating_add(1);
                }
            }
            // Torn read of a cached war row: fail the frame closed (never a
            // silent clean-frame drop). A structurally-invalid row is churn.
            Err(ExtractError::MemoryRead { .. }) => skipped_rows = skipped_rows.saturating_add(1),
            Err(_) => stale_dropped = stale_dropped.saturating_add(1),
        }
    }
    let war_total = discovery.war_row_addrs.len();
    if war_total >= 8 && (stale_dropped as usize).saturating_mul(2) > war_total {
        return Err(ExtractError::InvalidRoot(CAPWAR_DISCOVERY_DEGRADED));
    }

    Ok(ExtractedCapWarFrame {
        countries,
        wars,
        observed_rows,
        skipped_rows,
        stale_dropped,
        country_roots: roots.countries.len().min(u32::MAX as usize) as u32,
    })
}

/// Read the capitulation block for one country.
///
/// Returns `Ok(None)` for a VALIDATED absence (the `CCountry+0xF88` diplomacy
/// sub-object pointer slot reads 0). Any torn or range-violating read is an
/// error so the caller can fail the frame closed — a real value is never
/// substituted with a default.
///
/// Before trusting any field, the CCountry object is RE-VALIDATED against the
/// snapshot taken at root resolution: the vtable slot must still hold the
/// pinned CCountry vtable AND the object's own tag_db_id must still match. This
/// defends against a country-table slot that was freed/reused between root
/// resolution and this read — a stale or reused pool slot cannot inject a
/// false capitulation row (e.g. the high-id 440-442 slots with no melt oracle
/// must be genuine, currently-initialized CCountry objects, not freed slots
/// that merely retain a plausible-looking vtable pointer).
fn read_country_capitulation<M: CapWarMemory>(
    mem: &M,
    country: &CountryRoot,
    expected_country_vtable: u64,
) -> Result<Option<CountryCapitulationRow>, ExtractError> {
    // Re-validate object identity (vtable + tag_db_id) at read time.
    let vtable = mem.read_u64(country.country_ptr)?;
    if vtable != expected_country_vtable {
        return Err(ExtractError::InvalidRoot("capwar_country_vtable_changed"));
    }
    let tag_db_id = mem.read_u32(add_addr(
        country.country_ptr,
        profile::DOMAIN53_OBJ_TAG_DB_ID,
    )?)?;
    if tag_db_id != country.tag_db_id {
        return Err(ExtractError::InvalidRoot("capwar_country_tag_changed"));
    }

    let diplomacy_ptr = mem.read_u64(add_addr(
        country.country_ptr,
        profile::DOMAIN60_DIPLOMACY_SUB_OBJECT,
    )?)?;
    if diplomacy_ptr == 0 {
        return Ok(None);
    }
    if !is_heap_ptr(diplomacy_ptr) {
        return Err(ExtractError::InvalidRoot("capwar_diplomacy_ptr_invalid"));
    }
    let block = mem.read_bytes(add_addr(diplomacy_ptr, CAP_BLOCK_BASE)?, CAP_BLOCK_LEN)?;
    let rel = |offset: usize| offset - CAP_BLOCK_BASE;
    let capitulated_byte = block[rel(profile::DOMAIN60_DIPLOMACY_CAPITULATED)];
    if capitulated_byte > 1 {
        return Err(ExtractError::Validation(format!(
            "capwar_capitulated_byte_{capitulated_byte}"
        )));
    }
    let read_i32_at = |offset: usize| -> i32 {
        let at = rel(offset);
        i32::from_le_bytes(block[at..at + 4].try_into().expect("i32 bytes"))
    };
    let capitulated_date_raw = read_i32_at(profile::DOMAIN60_DIPLOMACY_CAPITULATED_DATE);
    let last_surrender_date_raw = read_i32_at(profile::DOMAIN60_DIPLOMACY_LAST_SURRENDER_DATE);
    let faction_join_date_raw = read_i32_at(profile::DOMAIN60_DIPLOMACY_FACTION_JOIN_DATE);
    let legitimacy_raw = read_i32_at(profile::DOMAIN60_DIPLOMACY_LEGITIMACY);
    let exile_army_leaders = read_i32_at(profile::DOMAIN60_DIPLOMACY_EXILE_ARMY_LEADERS);
    if !date_raw_valid_nullable(capitulated_date_raw)
        || !date_raw_valid_nullable(last_surrender_date_raw)
        || !date_raw_valid_nullable(faction_join_date_raw)
    {
        return Err(ExtractError::InvalidRoot("capwar_date_out_of_window"));
    }
    if !(0..=MAX_LEGITIMACY_RAW).contains(&legitimacy_raw) {
        return Err(ExtractError::InvalidRoot("capwar_legitimacy_out_of_range"));
    }
    if !(0..=MAX_EXILE_ARMY_LEADERS).contains(&exile_army_leaders) {
        return Err(ExtractError::InvalidRoot(
            "capwar_exile_leaders_out_of_range",
        ));
    }
    Ok(Some(CountryCapitulationRow {
        tag_db_id: country.tag_db_id,
        capitulated: capitulated_byte == 1,
        capitulated_date_raw,
        last_surrender_date_raw,
        faction_join_date_raw,
        legitimacy_raw,
        exile_army_leaders,
    }))
}

/// Full war-row validation + value read (the proven p06 filter + start date).
fn read_war_row<M: CapWarMemory>(
    mem: &M,
    row_addr: u64,
    date_vtables: &[u64; 2],
    valid_tags: &BTreeSet<u32>,
) -> Result<WarRow, ExtractError> {
    let bytes = if profile::DOMAIN60_WAR_RESIDUALS_LIVE_ENABLED {
        // The authenticated D60 row is CWarRelation+0x10. Read the object
        // vtable and complete row twice as one coherent span; a torn object or
        // an alias with the right date/tag prefix must never emit H-B5 values.
        let object_addr = row_addr
            .checked_sub(0x10)
            .ok_or(ExtractError::InvalidRoot("capwar_war_object_underflow"))?;
        let object_len = profile::DOMAIN60_WAR_RESIDUAL_ROW_LEN + 0x10;
        let before = mem.read_bytes(object_addr, object_len)?;
        let after = mem.read_bytes(object_addr, object_len)?;
        if before != after {
            return Err(ExtractError::InvalidRoot("capwar_war_object_torn"));
        }
        let module_base = date_vtables[0]
            .checked_sub(profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA as u64)
            .ok_or(ExtractError::InvalidRoot("capwar_module_base_underflow"))?;
        let expected = add_addr(
            module_base,
            profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA,
        )?;
        let object_vtable = u64::from_le_bytes(before[0..8].try_into().expect("u64 bytes"));
        if object_vtable != expected {
            return Err(ExtractError::InvalidRoot(
                "capwar_cwarrelation_vtable_mismatch",
            ));
        }
        before[0x10..].to_vec()
    } else {
        mem.read_bytes(row_addr, WAR_ROW_READ_LEN)?
    };
    let read_u32_at = |offset: usize| -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 bytes"))
    };
    let read_i32_at = |offset: usize| -> i32 {
        i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 bytes"))
    };
    let vtable = u64::from_le_bytes(
        bytes[profile::DOMAIN53_WAR_DATE_VTABLE..profile::DOMAIN53_WAR_DATE_VTABLE + 8]
            .try_into()
            .expect("u64 bytes"),
    );
    if !date_vtables.contains(&vtable) {
        return Err(ExtractError::InvalidRoot("capwar_war_date_vtable_mismatch"));
    }
    let first = read_u32_at(profile::DOMAIN53_WAR_FIRST_TAG);
    let second = read_u32_at(profile::DOMAIN53_WAR_SECOND_TAG);
    if first == second
        || !valid_tag_db_id(first)
        || !valid_tag_db_id(second)
        || !valid_tags.contains(&first)
        || !valid_tags.contains(&second)
    {
        return Err(ExtractError::InvalidRoot("capwar_war_pair_invalid"));
    }
    let echo_first = read_u32_at(profile::DOMAIN53_WAR_ECHO_FIRST);
    let echo_second = read_u32_at(profile::DOMAIN53_WAR_ECHO_SECOND);
    if echo_first != first || echo_second != second {
        return Err(ExtractError::InvalidRoot("capwar_war_echo_mismatch"));
    }
    let first_casualties = i64::from(read_i32_at(profile::DOMAIN53_WAR_FIRST_CASUALTIES));
    let second_casualties = i64::from(read_i32_at(profile::DOMAIN53_WAR_SECOND_CASUALTIES));
    if !(0..=MAX_CASUALTIES).contains(&first_casualties)
        || !(0..=MAX_CASUALTIES).contains(&second_casualties)
    {
        return Err(ExtractError::InvalidRoot("capwar_war_casualties_invalid"));
    }
    let start_date_raw = read_i32_at(profile::DOMAIN60_WAR_START_DATE);
    if !war_start_date_raw_valid(start_date_raw) {
        return Err(ExtractError::InvalidRoot("capwar_war_start_date_invalid"));
    }
    let residuals = if profile::DOMAIN60_WAR_RESIDUALS_LIVE_ENABLED {
        Some(war_residuals_from_row_bytes(&bytes)?)
    } else {
        None
    };
    Ok(WarRow {
        first_tag_db_id: first,
        second_tag_db_id: second,
        start_date_raw,
        first_casualties,
        second_casualties,
        residuals,
    })
}

/// Resolve the complete current war-row set through the authenticated H-B5
/// owner route instead of scanning the heap. The route is bounded by the live
/// country table: for every viewer country, index its `+0xF88` relation table
/// by every target country's CID database index, then validate the `+0x2E8`
/// `CWarRelation*` (exact vtable, live byte) before admitting `object+0x10` as
/// the already-established D60 row.
///
/// A non-null malformed root or torn read rejects the discovery. Null table
/// entries and retired objects are measured absence. Addresses remain local to
/// the discovery cache and never enter the compact or replay carrier.
pub fn discover_war_rows_from_relation_roots<M: CapWarMemory>(
    mem: &M,
) -> Result<CapWarDiscovery, ExtractError> {
    let roots = resolve_frame_roots(mem)?;
    let module_base = roots.module_base;
    let valid_tags: BTreeSet<u32> = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect();
    let date_vtables = war_date_vtables(module_base)?;
    let expected_war_vtable = add_addr(
        module_base,
        profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA,
    )?;
    let mut target_indices = Vec::with_capacity(roots.countries.len());
    let mut unique_target_indices = BTreeSet::new();
    for target in &roots.countries {
        let target_index = mem.read_u32(add_addr(
            target.country_ptr,
            profile::DOMAIN53_COUNTRY_CID_INDEX,
        )?)?;
        if target_index >= profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS
            || !unique_target_indices.insert(target_index)
        {
            return Err(ExtractError::InvalidRoot(
                "capwar_relation_target_index_invalid",
            ));
        }
        target_indices.push(target_index);
    }

    let mut war_row_addrs = BTreeSet::new();
    let mut war_pairs = BTreeSet::new();
    let mut manager_count = 0u32;
    let mut table_count = 0u32;
    for viewer in &roots.countries {
        let manager = mem.read_u64(add_addr(
            viewer.country_ptr,
            profile::DOMAIN60_DIPLOMACY_SUB_OBJECT,
        )?)?;
        if manager == 0 {
            continue;
        }
        if !is_heap_ptr(manager) {
            return Err(ExtractError::InvalidRoot(
                "capwar_relation_manager_invalid",
            ));
        }
        manager_count = manager_count.saturating_add(1);
        let table = mem.read_u64(add_addr(
            manager,
            profile::DOMAIN60_WAR_RELATION_TABLE,
        )?)?;
        if table == 0 {
            continue;
        }
        if !is_heap_ptr(table) {
            return Err(ExtractError::InvalidRoot("capwar_relation_table_invalid"));
        }
        table_count = table_count.saturating_add(1);
        for target_index in &target_indices {
            let slot = table
                .checked_add(u64::from(*target_index) * 8)
                .ok_or(ExtractError::InvalidRoot(
                    "capwar_relation_slot_overflow",
                ))?;
            let relation = mem.read_u64(slot)?;
            if relation == 0 {
                continue;
            }
            if !is_heap_ptr(relation) {
                return Err(ExtractError::InvalidRoot("capwar_relation_invalid"));
            }
            let war_object = mem.read_u64(add_addr(
                relation,
                profile::DOMAIN60_WAR_RELATION_OBJECT_PTR,
            )?)?;
            if war_object == 0 {
                continue;
            }
            if !is_heap_ptr(war_object) {
                return Err(ExtractError::InvalidRoot(
                    "capwar_relation_object_invalid",
                ));
            }
            let retired = mem.read_bytes(
                add_addr(war_object, profile::DOMAIN60_WAR_OBJECT_RETIRED)?,
                1,
            )?[0];
            if retired != 0 {
                continue;
            }
            if mem.read_u64(war_object)? != expected_war_vtable {
                return Err(ExtractError::InvalidRoot(
                    "capwar_relation_object_vtable_mismatch",
                ));
            }
            let row_addr = add_addr(war_object, profile::DOMAIN60_WAR_OBJECT_TO_ROW)?;
            if war_row_addrs.contains(&row_addr) {
                continue;
            }
            let row = read_war_row(mem, row_addr, &date_vtables, &valid_tags)?;
            let pair = if row.first_tag_db_id <= row.second_tag_db_id {
                (row.first_tag_db_id, row.second_tag_db_id)
            } else {
                (row.second_tag_db_id, row.first_tag_db_id)
            };
            if !war_pairs.insert(pair) {
                return Err(ExtractError::InvalidRoot(
                    "capwar_relation_duplicate_war_pair",
                ));
            }
            war_row_addrs.insert(row_addr);
        }
    }
    if manager_count == 0 || table_count == 0 {
        return Err(ExtractError::InvalidRoot(
            "capwar_relation_roots_empty_implausible",
        ));
    }

    Ok(CapWarDiscovery {
        war_row_addrs: war_row_addrs.into_iter().collect(),
        scan_range_count: 0,
    })
}

/// Decode the exact H-B5 fields from an authenticated D60 row window. This is
/// public so the d53 sidecar can reuse the same typed discriminator law once
/// the single build-profile activation flag is promoted.
pub fn war_residuals_from_row_bytes(bytes: &[u8]) -> Result<WarResiduals, ExtractError> {
    if bytes.len() < profile::DOMAIN60_WAR_RESIDUAL_ROW_LEN {
        return Err(ExtractError::InvalidRoot("capwar_war_residual_row_short"));
    }
    let read_u32_at = |offset: usize| -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 bytes"))
    };
    let threat_fixed5_raw = i64::from_le_bytes(
        bytes[profile::DOMAIN60_WAR_THREAT_FIXED5_RAW
            ..profile::DOMAIN60_WAR_THREAT_FIXED5_RAW + 8]
            .try_into()
            .expect("i64 bytes"),
    );
    let first_was_instigator = match bytes[profile::DOMAIN60_WAR_FIRST_WAS_INSTIGATOR] {
        0 => false,
        1 => true,
        value => {
            return Err(ExtractError::Validation(format!(
                "capwar_first_was_instigator_{value}"
            )))
        }
    };
    let instigator_raw = read_u32_at(profile::DOMAIN60_WAR_HOSTILITY_REASON_INSTIGATOR);
    let defender_raw = read_u32_at(profile::DOMAIN60_WAR_HOSTILITY_REASON_DEFENDER);
    let hostility_reason_instigator = HostilityReason::from_raw(instigator_raw).ok_or_else(|| {
        ExtractError::Validation(format!(
            "capwar_hostility_reason_instigator_{instigator_raw}"
        ))
    })?;
    let hostility_reason_defender = HostilityReason::from_raw(defender_raw).ok_or_else(|| {
        ExtractError::Validation(format!(
            "capwar_hostility_reason_defender_{defender_raw}"
        ))
    })?;
    Ok(WarResiduals {
        threat_fixed5_raw,
        first_was_instigator,
        hostility_reason_instigator,
        hostility_reason_defender,
    })
}

/// Build the wire snapshot from an extracted frame (collector emit path).
pub fn snapshot_from_frame(
    frame: &ExtractedCapWarFrame,
    elapsed_wall_ms: u64,
    base_status_flags: u32,
) -> CapitulationWarSnapshot {
    let mut status_flags = base_status_flags;
    if !frame.wars.is_empty() {
        status_flags |= crate::compact::STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR;
    }
    if !frame.wars.is_empty() && frame.wars.iter().all(|row| row.residuals.is_some()) {
        status_flags |= crate::compact::STATUS_WAR_RESIDUALS_EXACT;
    }
    CapitulationWarSnapshot {
        elapsed_wall_ms,
        status_flags,
        countries: frame.countries.clone(),
        wars: frame.wars.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const MODULE_BASE: u64 = 0x1_4000_0000;
    const HEAP_BASE: u64 = 0x1_00_0000_0000;
    const GAME_STATE: u64 = HEAP_BASE + 0x1000;
    const COUNTRY_DATABASE: u64 = HEAP_BASE + 0x2000;
    const COUNTRY_TABLE: u64 = HEAP_BASE + 0x3000;

    fn real_date_raw(year: i32, doy: i32, hour: i32) -> i32 {
        DATE_EPOCH_CONST + (year * 365 + doy) * 24 + hour
    }

    #[derive(Default)]
    struct MockMemory {
        bytes: RefCell<BTreeMap<u64, u8>>,
        ranges: Vec<(u64, usize)>,
        unreadable: BTreeSet<u64>,
    }

    impl MockMemory {
        fn write(&self, address: u64, data: &[u8]) {
            let mut bytes = self.bytes.borrow_mut();
            for (index, byte) in data.iter().enumerate() {
                bytes.insert(address + index as u64, *byte);
            }
        }

        fn write_u64(&self, address: u64, value: u64) {
            self.write(address, &value.to_le_bytes());
        }

        fn write_u32(&self, address: u64, value: u32) {
            self.write(address, &value.to_le_bytes());
        }

        fn write_i32(&self, address: u64, value: i32) {
            self.write(address, &value.to_le_bytes());
        }
    }

    impl CapWarMemory for MockMemory {
        fn module_base(&self) -> u64 {
            MODULE_BASE
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            let bytes = self.bytes.borrow();
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let at = address + index as u64;
                if self.unreadable.contains(&at) {
                    return Err(ExtractError::MemoryRead { address, len });
                }
                out.push(bytes.get(&at).copied().unwrap_or(0));
            }
            Ok(out)
        }

        fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
            self.ranges.clone()
        }
    }

    fn country_vtable() -> u64 {
        MODULE_BASE + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64
    }

    fn setup_rtti(mem: &MockMemory) {
        // Complete-object-locator chain for the CCountry vtable RTTI check.
        let vtable = country_vtable();
        let col = MODULE_BASE + 0x100_0000;
        let type_descriptor_rva = 0x200_0000u32;
        let class_descriptor_rva = 0x300_0000u32;
        mem.write_u64(vtable - 8, col);
        mem.write_u32(col, 1);
        mem.write_u32(col + 0x0C, type_descriptor_rva);
        mem.write_u32(col + 0x10, class_descriptor_rva);
        mem.write_u32(col + 0x14, (col - MODULE_BASE) as u32);
        let type_descriptor = MODULE_BASE + u64::from(type_descriptor_rva);
        mem.write(type_descriptor + 0x10, b".?AVCCountry@@\0");
    }

    fn setup_roots(mem: &MockMemory, countries: &[(u32, u64)]) {
        mem.write_u64(
            MODULE_BASE + profile::CG_STATE_DATE_OFFSET as u64,
            GAME_STATE,
        );
        mem.write_u64(
            MODULE_BASE + profile::COUNTRY_DATABASE_OFFSET as u64,
            COUNTRY_DATABASE,
        );
        mem.write_u64(
            GAME_STATE + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64,
            COUNTRY_TABLE,
        );
        mem.write_u32(
            GAME_STATE + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64,
            countries.len() as u32,
        );
        for (slot, (tag_db_id, country_ptr)) in countries.iter().enumerate() {
            mem.write_u64(COUNTRY_TABLE + slot as u64 * 8, *country_ptr);
            mem.write_u64(*country_ptr, country_vtable());
            mem.write_u32(
                *country_ptr + profile::DOMAIN53_OBJ_TAG_DB_ID as u64,
                *tag_db_id,
            );
            mem.write_u32(
                *country_ptr + profile::DOMAIN53_COUNTRY_CID_INDEX as u64,
                slot as u32,
            );
        }
        setup_rtti(mem);
    }

    fn setup_capitulation(
        mem: &MockMemory,
        country_ptr: u64,
        diplomacy_ptr: u64,
        capitulated: bool,
        capitulated_date_raw: i32,
        legitimacy_raw: i32,
        exile: i32,
    ) {
        mem.write_u64(
            country_ptr + profile::DOMAIN60_DIPLOMACY_SUB_OBJECT as u64,
            diplomacy_ptr,
        );
        mem.write(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_CAPITULATED as u64,
            &[u8::from(capitulated)],
        );
        mem.write_i32(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_CAPITULATED_DATE as u64,
            capitulated_date_raw,
        );
        mem.write_i32(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_LAST_SURRENDER_DATE as u64,
            crate::compact::CGAMEDATE_NULL_RAW,
        );
        mem.write_i32(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_FACTION_JOIN_DATE as u64,
            crate::compact::CGAMEDATE_NULL_RAW,
        );
        mem.write_i32(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_LEGITIMACY as u64,
            legitimacy_raw,
        );
        mem.write_i32(
            diplomacy_ptr + profile::DOMAIN60_DIPLOMACY_EXILE_ARMY_LEADERS as u64,
            exile,
        );
    }

    fn setup_war_row(
        mem: &MockMemory,
        row_addr: u64,
        first: u32,
        second: u32,
        start_date_raw: i32,
        first_casualties: i32,
        second_casualties: i32,
    ) {
        // H-B5 promotion made the complete CWarRelation identity part of the
        // production admission law.  The authenticated row begins at
        // object+0x10, so every otherwise-valid test fixture must also carry
        // the exact object vtable immediately before the row.  Leaving this
        // zeroed models a structurally stale cache entry, not a live war.
        mem.write_u64(
            row_addr - profile::DOMAIN60_WAR_OBJECT_TO_ROW as u64,
            MODULE_BASE + profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA as u64,
        );
        let game_date_vtable = MODULE_BASE + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64;
        mem.write_u32(row_addr + profile::DOMAIN53_WAR_FIRST_TAG as u64, first);
        mem.write_u32(row_addr + profile::DOMAIN53_WAR_SECOND_TAG as u64, second);
        mem.write_u64(
            row_addr + profile::DOMAIN53_WAR_DATE_VTABLE as u64,
            game_date_vtable,
        );
        mem.write_i32(
            row_addr + profile::DOMAIN60_WAR_START_DATE as u64,
            start_date_raw,
        );
        mem.write_i32(
            row_addr + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64,
            first_casualties,
        );
        mem.write_i32(
            row_addr + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64,
            second_casualties,
        );
        mem.write_u32(row_addr + profile::DOMAIN53_WAR_ECHO_FIRST as u64, first);
        mem.write_u32(row_addr + profile::DOMAIN53_WAR_ECHO_SECOND as u64, second);
    }

    fn setup_relation_war_root(
        mem: &MockMemory,
        viewer_country: u64,
        target_index: u32,
        relation_manager: u64,
        relation_table: u64,
        relation_entry: u64,
        war_object: u64,
    ) {
        mem.write_u64(
            viewer_country + profile::DOMAIN60_DIPLOMACY_SUB_OBJECT as u64,
            relation_manager,
        );
        mem.write_u64(
            relation_manager + profile::DOMAIN60_WAR_RELATION_TABLE as u64,
            relation_table,
        );
        mem.write_u64(
            relation_table + u64::from(target_index) * 8,
            relation_entry,
        );
        mem.write_u64(
            relation_entry + profile::DOMAIN60_WAR_RELATION_OBJECT_PTR as u64,
            war_object,
        );
        mem.write_u64(
            war_object,
            MODULE_BASE + profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA as u64,
        );
        mem.write(
            war_object + profile::DOMAIN60_WAR_OBJECT_RETIRED as u64,
            &[0],
        );
    }

    fn base_mock() -> (MockMemory, u64, u64) {
        let mem = MockMemory::default();
        let ger_ptr = HEAP_BASE + 0x10_0000;
        let fra_ptr = HEAP_BASE + 0x20_0000;
        setup_roots(&mem, &[(47, ger_ptr), (3, fra_ptr)]);
        (mem, ger_ptr, fra_ptr)
    }

    #[test]
    fn extracts_capitulation_and_wars() {
        let (mut mem, ger_ptr, fra_ptr) = base_mock();
        let fra_date = real_date_raw(1940, 31 + 28 + 31 + 30 + 31 + 17, 2);
        setup_capitulation(
            &mem,
            fra_ptr,
            HEAP_BASE + 0x30_0000,
            true,
            fra_date,
            10_000_000,
            8,
        );
        setup_capitulation(
            &mem,
            ger_ptr,
            HEAP_BASE + 0x40_0000,
            false,
            crate::compact::CGAMEDATE_NULL_RAW,
            0,
            0,
        );
        let war_addr = HEAP_BASE + 0x50_0000;
        setup_war_row(
            &mem,
            war_addr,
            47,
            3,
            real_date_raw(1939, 244, 4),
            1110,
            948,
        );
        mem.ranges = vec![(war_addr, 0x100)];

        let discovery = discover_war_rows(&mem).expect("discovery");
        assert_eq!(discovery.war_row_addrs, vec![war_addr]);

        let frame = extract_frame_with_discovery(&mem, &discovery).expect("frame");
        assert_eq!(frame.skipped_rows, 0);
        assert_eq!(frame.stale_dropped, 0);
        assert_eq!(frame.country_roots, 2);
        assert_eq!(frame.countries.len(), 2);
        let fra = frame
            .countries
            .iter()
            .find(|row| row.tag_db_id == 3)
            .expect("france row");
        assert!(fra.capitulated);
        assert_eq!(fra.capitulated_date_raw, fra_date);
        assert_eq!(fra.legitimacy_raw, 10_000_000);
        assert_eq!(fra.exile_army_leaders, 8);
        let ger = frame
            .countries
            .iter()
            .find(|row| row.tag_db_id == 47)
            .expect("germany row");
        assert!(!ger.capitulated);
        assert_eq!(frame.wars.len(), 1);
        assert_eq!(frame.wars[0].first_casualties, 1110);
        assert_eq!(frame.wars[0].second_casualties, 948);
    }

    // P3: `discover_war_rows_from_raw_hits` (fed by the shared sweep service)
    // must produce the SAME validated war-row set as the inline `discover_war_rows`
    // scan, given the raw hit addresses the scan would have found (the date-vtable
    // qword sits at row_addr + DOMAIN53_WAR_DATE_VTABLE).
    #[test]
    fn discover_war_rows_from_raw_hits_matches_inline_scan() {
        let (mut mem, ger_ptr, fra_ptr) = base_mock();
        setup_capitulation(
            &mem,
            fra_ptr,
            HEAP_BASE + 0x30_0000,
            true,
            real_date_raw(1940, 100, 2),
            10_000_000,
            8,
        );
        setup_capitulation(
            &mem,
            ger_ptr,
            HEAP_BASE + 0x40_0000,
            false,
            crate::compact::CGAMEDATE_NULL_RAW,
            0,
            0,
        );
        let war_addr = HEAP_BASE + 0x50_0000;
        setup_war_row(
            &mem,
            war_addr,
            47,
            3,
            real_date_raw(1939, 244, 4),
            1110,
            948,
        );
        mem.ranges = vec![(war_addr, 0x100)];

        // Inline scan (the pre-P3 path) for the reference set.
        let inline = discover_war_rows(&mem).expect("inline discovery");
        assert_eq!(inline.war_row_addrs, vec![war_addr]);

        // The needles the collector registers with the shared sweep.
        let needles = capwar_war_date_vtables(MODULE_BASE).expect("needles");
        assert_eq!(
            needles[0],
            MODULE_BASE + profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA as u64
        );
        assert_eq!(
            needles[1],
            MODULE_BASE + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64
        );

        // The sweep would report a hit at the vtable qword inside the war row.
        let raw_hits = vec![war_addr + profile::DOMAIN53_WAR_DATE_VTABLE as u64];
        let from_hits =
            discover_war_rows_from_raw_hits(&mem, &raw_hits).expect("from-hits discovery");
        assert_eq!(from_hits.war_row_addrs, inline.war_row_addrs);

        // A raw hit that does not resolve to a valid war row is dropped (fail
        // closed), not blindly adopted.
        let junk_hits = vec![(HEAP_BASE + 0x99_0000) + profile::DOMAIN53_WAR_DATE_VTABLE as u64];
        let junk =
            discover_war_rows_from_raw_hits(&mem, &junk_hits).expect("junk discovery ok-but-empty");
        assert!(junk.war_row_addrs.is_empty());
    }

    #[test]
    fn bounded_relation_root_resolves_exact_cwarrelation_row() {
        let (mem, ger_ptr, fra_ptr) = base_mock();
        let manager = HEAP_BASE + 0x30_0000;
        let table = HEAP_BASE + 0x31_0000;
        let relation = HEAP_BASE + 0x32_0000;
        let war_object = HEAP_BASE + 0x50_0000;
        setup_relation_war_root(&mem, ger_ptr, 1, manager, table, relation, war_object);
        // The second country's clean null manager is measured absence.
        mem.write_u64(fra_ptr + profile::DOMAIN60_DIPLOMACY_SUB_OBJECT as u64, 0);
        setup_war_row(
            &mem,
            war_object + profile::DOMAIN60_WAR_OBJECT_TO_ROW as u64,
            47,
            3,
            real_date_raw(1939, 244, 4),
            1110,
            948,
        );

        let discovery =
            discover_war_rows_from_relation_roots(&mem).expect("bounded root discovery");
        assert_eq!(
            discovery.war_row_addrs,
            vec![war_object + profile::DOMAIN60_WAR_OBJECT_TO_ROW as u64]
        );
        assert_eq!(discovery.scan_range_count, 0);

        mem.write(
            war_object + profile::DOMAIN60_WAR_OBJECT_RETIRED as u64,
            &[1],
        );
        assert!(discover_war_rows_from_relation_roots(&mem)
            .expect("retired object is measured absence")
            .war_row_addrs
            .is_empty());
    }

    #[test]
    fn null_diplomacy_pointer_is_validated_absence_not_skip() {
        let (mut mem, ger_ptr, fra_ptr) = base_mock();
        setup_capitulation(
            &mem,
            ger_ptr,
            HEAP_BASE + 0x40_0000,
            false,
            crate::compact::CGAMEDATE_NULL_RAW,
            0,
            0,
        );
        // FRA: pointer slot present and readable but zero.
        mem.write_u64(fra_ptr + profile::DOMAIN60_DIPLOMACY_SUB_OBJECT as u64, 0);
        mem.ranges = Vec::new();
        let frame = extract_frame_with_discovery(&mem, &CapWarDiscovery::default()).expect("frame");
        assert_eq!(frame.skipped_rows, 0);
        assert_eq!(frame.countries.len(), 1);
        assert_eq!(frame.countries[0].tag_db_id, 47);
    }

    #[test]
    fn torn_capitulation_read_counts_as_skip() {
        let (mut mem, ger_ptr, fra_ptr) = base_mock();
        setup_capitulation(
            &mem,
            ger_ptr,
            HEAP_BASE + 0x40_0000,
            false,
            crate::compact::CGAMEDATE_NULL_RAW,
            0,
            0,
        );
        let fra_diplo = HEAP_BASE + 0x30_0000;
        setup_capitulation(
            &mem,
            fra_ptr,
            fra_diplo,
            true,
            real_date_raw(1940, 100, 2),
            0,
            0,
        );
        mem.unreadable
            .insert(fra_diplo + profile::DOMAIN60_DIPLOMACY_CAPITULATED as u64);
        let frame = extract_frame_with_discovery(&mem, &CapWarDiscovery::default()).expect("frame");
        assert_eq!(frame.skipped_rows, 1);
        assert_eq!(frame.countries.len(), 1);
    }

    #[test]
    fn capitulated_byte_above_one_is_a_skip_not_a_value() {
        let (mut mem, ger_ptr, _fra_ptr) = base_mock();
        let diplo = HEAP_BASE + 0x40_0000;
        setup_capitulation(
            &mem,
            ger_ptr,
            diplo,
            false,
            crate::compact::CGAMEDATE_NULL_RAW,
            0,
            0,
        );
        mem.write(diplo + profile::DOMAIN60_DIPLOMACY_CAPITULATED as u64, &[2]);
        let frame = extract_frame_with_discovery(&mem, &CapWarDiscovery::default()).expect("frame");
        assert_eq!(frame.skipped_rows, 1);
        assert!(frame.countries.iter().all(|row| row.tag_db_id != 47));
    }

    #[test]
    fn stale_war_rows_are_dropped_and_majority_degrades() {
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        // 8 cached addresses, all pointing at zeroed memory -> all stale.
        let discovery = CapWarDiscovery {
            war_row_addrs: (0..8).map(|i| HEAP_BASE + 0x60_0000 + i * 0x100).collect(),
            scan_range_count: 1,
        };
        mem.ranges = Vec::new();
        let error = extract_frame_with_discovery(&mem, &discovery).expect_err("degraded");
        assert_eq!(error, ExtractError::InvalidRoot(CAPWAR_DISCOVERY_DEGRADED));
    }

    #[test]
    fn single_stale_war_row_survives_without_degrading() {
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let war_addr = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, war_addr, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        let discovery = CapWarDiscovery {
            war_row_addrs: vec![war_addr, HEAP_BASE + 0x70_0000],
            scan_range_count: 1,
        };
        let frame = extract_frame_with_discovery(&mem, &discovery).expect("frame");
        assert_eq!(frame.wars.len(), 1);
        assert_eq!(frame.stale_dropped, 1);
    }

    #[test]
    fn torn_war_row_read_is_a_skip_not_a_silent_drop() {
        // A cached war row whose memory is momentarily UNREADABLE (a torn read,
        // MemoryRead) may still be a live war. It must count as a per-row SKIP
        // (which fails the whole frame closed via the collector) — NOT a
        // stale_dropped that a minority test would tolerate, silently dropping a
        // real war row from an otherwise-"clean" frame.
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let good = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, good, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        let torn = HEAP_BASE + 0x90_0000;
        setup_war_row(&mem, torn, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        // First byte of the torn row's read span is unreadable -> MemoryRead.
        mem.unreadable.insert(torn);
        let discovery = CapWarDiscovery {
            war_row_addrs: vec![good, torn],
            scan_range_count: 1,
        };
        let frame = extract_frame_with_discovery(&mem, &discovery).expect("frame");
        assert_eq!(frame.skipped_rows, 1, "torn war row must be a skip");
        assert_eq!(frame.stale_dropped, 0, "torn read is not a stale drop");
    }

    #[test]
    fn structurally_invalid_war_row_is_stale_not_skip() {
        // A readable-but-invalid cached row (echo mismatch = the pool slot was
        // reused / war retired) is EXPECTED churn: stale_dropped, not a skip.
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let good = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, good, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        let retired = HEAP_BASE + 0x91_0000;
        setup_war_row(&mem, retired, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        mem.write_u32(retired + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 9999);
        let discovery = CapWarDiscovery {
            war_row_addrs: vec![good, retired],
            scan_range_count: 1,
        };
        let frame = extract_frame_with_discovery(&mem, &discovery).expect("frame");
        assert_eq!(frame.skipped_rows, 0);
        assert_eq!(frame.stale_dropped, 1);
        assert_eq!(frame.wars.len(), 1);
    }

    #[test]
    fn read_country_rejects_slot_whose_vtable_changed_since_resolution() {
        // read_country_capitulation RE-VALIDATES object identity: a slot that
        // was a valid CCountry at root resolution but whose vtable no longer
        // matches (freed/reused in the window before this read) must be an
        // error — never a false capitulation row. Tested at the read level
        // because the frame-level root walk would otherwise pre-filter it.
        let (mut mem, _ger_ptr, fra_ptr) = base_mock();
        setup_capitulation(
            &mem,
            fra_ptr,
            HEAP_BASE + 0x30_0000,
            true,
            real_date_raw(1940, 100, 2),
            0,
            0,
        );
        let country = CountryRoot {
            tag_db_id: 3,
            country_ptr: fra_ptr,
        };
        // Baseline: with the correct vtable it reads a capitulation row.
        let ok = read_country_capitulation(&mem, &country, country_vtable())
            .expect("reads")
            .expect("row present");
        assert!(ok.capitulated);
        // Vtable changed after resolution -> rejected.
        mem.write_u64(fra_ptr, MODULE_BASE + 0xDEAD_00);
        assert!(matches!(
            read_country_capitulation(&mem, &country, country_vtable()),
            Err(ExtractError::InvalidRoot("capwar_country_vtable_changed"))
        ));
    }

    #[test]
    fn read_country_rejects_slot_whose_tag_changed_since_resolution() {
        // Same object, tag_db_id changed (a reused slot now holds a different
        // country): reject rather than mislabel a capitulation row.
        let (mut mem, _ger_ptr, fra_ptr) = base_mock();
        setup_capitulation(
            &mem,
            fra_ptr,
            HEAP_BASE + 0x30_0000,
            true,
            real_date_raw(1940, 100, 2),
            0,
            0,
        );
        let country = CountryRoot {
            tag_db_id: 3,
            country_ptr: fra_ptr,
        };
        mem.write_u32(fra_ptr + profile::DOMAIN53_OBJ_TAG_DB_ID as u64, 999);
        assert!(matches!(
            read_country_capitulation(&mem, &country, country_vtable()),
            Err(ExtractError::InvalidRoot("capwar_country_tag_changed"))
        ));
    }

    #[test]
    fn discovery_skips_unreadable_ranges_instead_of_failing() {
        // A range that vanished between enumeration and read (heap churn)
        // must be SKIPPED — discovery still succeeds with rows from healthy
        // ranges (naval 49/50 precedent), never failing the whole scan.
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let good = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, good, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        let vanished = HEAP_BASE + 0x80_0000;
        mem.unreadable.insert(vanished);
        mem.ranges = vec![(vanished, 0x1000), (good, 0x100)];
        let discovery = discover_war_rows(&mem).expect("discovery tolerates churn");
        assert_eq!(discovery.war_row_addrs, vec![good]);
        assert_eq!(discovery.scan_range_count, 2);
    }

    #[test]
    fn war_row_with_out_of_window_date_is_rejected() {
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let war_addr = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, war_addr, 47, 3, 123, 10, 20);
        let discovery = discover_war_rows(&mem).expect("discovery");
        assert!(discovery.war_row_addrs.is_empty());
    }

    #[test]
    fn discovery_keeps_only_fully_validated_rows() {
        let (mut mem, _ger_ptr, _fra_ptr) = base_mock();
        let good = HEAP_BASE + 0x50_0000;
        setup_war_row(&mem, good, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        // Decoy: date vtable present but echo pair does not match.
        let decoy = HEAP_BASE + 0x51_0000;
        setup_war_row(&mem, decoy, 47, 3, real_date_raw(1939, 244, 4), 10, 20);
        mem.write_u32(decoy + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 9999);
        mem.ranges = vec![(good, 0x100), (decoy, 0x100)];
        let discovery = discover_war_rows(&mem).expect("discovery");
        assert_eq!(discovery.war_row_addrs, vec![good]);
    }

    #[test]
    fn snapshot_badges_war_accumulator_only_with_rows() {
        let frame = ExtractedCapWarFrame {
            wars: vec![WarRow {
                first_tag_db_id: 1,
                second_tag_db_id: 2,
                start_date_raw: real_date_raw(1939, 244, 4),
                first_casualties: 1,
                second_casualties: 2,
                residuals: None,
            }],
            ..ExtractedCapWarFrame::default()
        };
        let snapshot = snapshot_from_frame(&frame, 3, 0);
        assert_ne!(
            snapshot.status_flags & crate::compact::STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR,
            0
        );
        let empty = snapshot_from_frame(&ExtractedCapWarFrame::default(), 3, 0);
        assert_eq!(empty.status_flags, 0);
        assert_eq!(
            decode_or_panic(&crate::compact::encode_payload(&snapshot))
                .wars
                .len(),
            1
        );
    }

    #[test]
    fn h_b5_row_decoder_preserves_exact_values_and_rejects_bad_discriminators() {
        let mut bytes = vec![0u8; profile::DOMAIN60_WAR_RESIDUAL_ROW_LEN];
        bytes[profile::DOMAIN60_WAR_THREAT_FIXED5_RAW
            ..profile::DOMAIN60_WAR_THREAT_FIXED5_RAW + 8]
            .copy_from_slice(&(-987_654i64).to_le_bytes());
        bytes[profile::DOMAIN60_WAR_FIRST_WAS_INSTIGATOR] = 1;
        bytes[profile::DOMAIN60_WAR_HOSTILITY_REASON_INSTIGATOR
            ..profile::DOMAIN60_WAR_HOSTILITY_REASON_INSTIGATOR + 4]
            .copy_from_slice(&HostilityReason::Puppet.raw().to_le_bytes());
        bytes[profile::DOMAIN60_WAR_HOSTILITY_REASON_DEFENDER
            ..profile::DOMAIN60_WAR_HOSTILITY_REASON_DEFENDER + 4]
            .copy_from_slice(&HostilityReason::Guarantee.raw().to_le_bytes());

        let residuals = war_residuals_from_row_bytes(&bytes).expect("valid H-B5 row");
        assert_eq!(residuals.threat_fixed5_raw, -987_654);
        assert!(residuals.first_was_instigator);
        assert_eq!(residuals.hostility_reason_instigator, HostilityReason::Puppet);
        assert_eq!(residuals.hostility_reason_defender, HostilityReason::Guarantee);

        bytes[profile::DOMAIN60_WAR_FIRST_WAS_INSTIGATOR] = 2;
        assert!(war_residuals_from_row_bytes(&bytes).is_err());
        bytes[profile::DOMAIN60_WAR_FIRST_WAS_INSTIGATOR] = 0;
        bytes[profile::DOMAIN60_WAR_HOSTILITY_REASON_DEFENDER
            ..profile::DOMAIN60_WAR_HOSTILITY_REASON_DEFENDER + 4]
            .copy_from_slice(&6u32.to_le_bytes());
        assert!(war_residuals_from_row_bytes(&bytes).is_err());
    }

    fn decode_or_panic(payload: &[u8]) -> CapitulationWarSnapshot {
        crate::compact::decode_snapshot_payload(0, payload).expect("decodes")
    }
}
