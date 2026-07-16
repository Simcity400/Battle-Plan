//! air_loss_stats_live (domain 61) — per-country CUMULATIVE air-combat loss statistics.
//!
//! This is the compact-packet DATA layer for the C1 crack (statistic id 23, the per-country
//! 89-field aggregate air-loss history record). Cracked + orchestrator-verified root chain
//! (see `extractor/native/runs/cracklist_c1_air_stats/EVIDENCE.md` and `EVIDENCE_FINISH`):
//!
//! ```text
//! CGameState  = u64[module_base + 0x3316E90]
//! registry    = u64[CGameState + 0x690]        // per-country stats registry
//! mgr_array   = u64[registry + 0x30]           // per-country stat-manager pointer array
//! country_mgr = u64[mgr_array + country_idx*8]
//! stat_base   = u64[country_mgr + 0x158]        // 89-field aggregate-history array base
//! loophist    = u64[stat_base + 23*8]           // CLoopHistory* (air = stat key 23)
//! record      = u64[ u64[loophist+0x20] + 0x30 ]
//! field[i]    = i64[ u64[record] + i*8 ] / 100000
//! ```
//!
//! FAIL-CLOSED, both directions:
//!   * `country_idx` is the stats registry's internal slot. It is bound to a country tag by
//!     cross-checking the EXTRACTOR's own country table (`CGameState+0x310`, indexed by
//!     `tag_db_id`): a row is emitted ONLY when `country_table[idx]` is a valid CCountry
//!     (vtable RVA 0x27D0E90) whose `tag_db_id (@+0x08) == idx`. This binding is PROVEN
//!     live (index self-consistent 437/440; the stat-bearing managers resolve through this
//!     table to exactly the 1939.12 belligerent set — GER richest, SOV+FIN share the
//!     Winter-War stat key, ITA at idx 16 carries air key 23; see EVIDENCE_FINISH §B).
//!   * Any torn/invalid read, a CLoopHistory vtable mismatch, a field_count != 89, an absent
//!     air stat, a failed self-ref/yearly-window guard, or an out-of-bounds
//!     value FAILS THE ROW CLOSED (absent, never 0).
//!
//! SEMANTICS: only the six VALUE-PINNED channels are emitted with labels (the remaining ~18
//! of the 24 channels are still a residual multi-writer trace — do NOT label them here). The
//! raw record is available for the labeled indices only; unlabeled fields are intentionally
//! omitted so the packet never promotes an unproven label.
//!
//! SCHEDULING: domain 61 is production-scheduled after the D04 1944 populated
//! fixture revalidation proved 23/23 countries and 2047/2047 fields against the
//! melt oracle with the yearly `lh+0x20` cumulative window.

use std::collections::BTreeMap;

pub const DOMAIN_ID: u16 = 61;
pub const DOMAIN_NAME: &str = "air_loss_stats_live";

// ---- proven root-chain offsets (build 870ebf; RVA == module-base-relative) ----
pub const CGAMESTATE_RVA: u64 = 0x3316E90;
pub const REGISTRY_OFF: u64 = 0x690;
pub const MGR_ARRAY_OFF: u64 = 0x30;
pub const STAT_BASE_OFF: u64 = 0x158;
pub const AIR_STAT_KEY: u64 = 23;
pub const LOOPHIST_VTABLE_RVA: u64 = 0x29BBE58;
pub const LOOPHIST_SELF_REF_OFF: u64 = 0x28;
pub const LOOPHIST_FIELDCOUNT_OFF: u64 = 0x34;
pub const LOOPHIST_WINDOW_OFF: u64 = 0x20;
pub const LOOPHIST_RECFLDS_OFF: u64 = 0x30;
pub const LOOPHIST_WINDOW_CAPACITY_OFF: u64 = 0x20;
pub const EXPECTED_FIELD_COUNT: u32 = 89;
pub const EXPECTED_YEARLY_CAPACITY: u32 = 5;
pub const FIELD_SCALE: i64 = 100_000;

// ---- binding guard offsets (the EXTRACTOR's own country index) ----
pub const COUNTRY_TABLE_OFF: u64 = 0x310;
pub const COUNTRY_COUNT_OFF: u64 = 0x31C;
pub const COUNTRY_VTABLE_RVA: u64 = 0x27D0E90;
pub const COUNTRY_TAG_DB_ID_OFF: u64 = 0x08;

/// Value-pinned air channels: (field index, label). These six are the only channels with a
/// proven value-pin (D04 fixture) — the rest of the 24 remain an open multi-writer trace.
pub const PINNED_CHANNELS: &[(u32, &str)] = &[
    (3, "our_fighters_lost"),
    (10, "our_bombers_lost"),
    (44, "enemy_fighters_lost"),
    (51, "enemy_bombers_lost"),
    (58, "enemy_cas_lost"),
    (80, "enemy_bombers_disrupted"),
];

/// Max plausible cumulative loss (raw ×100000): 10,000,000 planes. Above this the row fails
/// closed (guards against a torn/aliased read passing the structural checks).
pub const MAX_CHANNEL_RAW: i64 = 1_000_000_000_000;

// ---- packet schema ----
pub const SCHEMA_VERSION: u32 = 1;
pub const PAYLOAD_HEADER_LEN: usize = 32;
/// Row: owner_tag_db_id u32 | country_idx u32 | flags u32 | pad u32 | 6 × i64 channel = 16+48.
pub const ROW_LEN: usize = 64;

pub const STATUS_OK: u32 = 0;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 0;
pub const STATUS_BINDING_UNVERIFIED: u32 = 1 << 1;
pub const STATUS_PARTIAL: u32 = 1 << 2;
/// Legacy lane-local status from the pre-promotion packet. Production emission
/// no longer sets this after D04 1944 value validation.
pub const STATUS_VALUE_VALIDATION_FIXTURE_REQUIRED: u32 = 1 << 3;

pub const ROW_FLAG_BINDING_VERIFIED: u32 = 1 << 0;
pub const ROW_FLAG_STRUCT_VERIFIED: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirLossStatRow {
    pub owner_tag_db_id: u32,
    pub country_idx: u32,
    pub flags: u32,
    /// Six value-pinned channels, in `PINNED_CHANNELS` order, raw ×100000 fixed-point.
    pub channels: [i64; 6],
}

impl AirLossStatRow {
    /// Human value (planes) for pinned channel `n` (0..6).
    pub fn channel_planes(&self, n: usize) -> f64 {
        self.channels[n] as f64 / FIELD_SCALE as f64
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirLossStatHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub schema_version: u32,
    pub field_count: u32,
}

// ---------------------------------------------------------------------------
// encode / decode
// ---------------------------------------------------------------------------
pub fn encode_payload(header: &AirLossStatHeader, rows: &[AirLossStatRow]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * ROW_LEN);
    p.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    p.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    p.extend_from_slice(&header.status_flags.to_le_bytes());
    p.extend_from_slice(&header.schema_version.to_le_bytes());
    p.extend_from_slice(&header.field_count.to_le_bytes());
    p.extend_from_slice(&(ROW_LEN as u32).to_le_bytes()); // self-describing row length
    p.extend_from_slice(&0u32.to_le_bytes()); // pad to a 32-byte header
    debug_assert_eq!(p.len(), PAYLOAD_HEADER_LEN);
    for r in rows {
        p.extend_from_slice(&r.owner_tag_db_id.to_le_bytes());
        p.extend_from_slice(&r.country_idx.to_le_bytes());
        p.extend_from_slice(&r.flags.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // pad
        for c in &r.channels {
            p.extend_from_slice(&c.to_le_bytes());
        }
    }
    p
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirLossGateSummary {
    pub status_flags: u32,
    pub schema_version: u32,
    pub field_count: u32,
    pub row_len: u32,
    pub row_count: u64,
    pub binding_verified_rows: u64,
    /// Per-label totals across all rows (raw ×100000 summed).
    pub channel_totals: BTreeMap<&'static str, i64>,
    pub parse_errors: Vec<String>,
}

/// Strict decode used by the replay/verification gate. Re-validates the self-describing row
/// length, field_count, and that every value is in range; accumulates per-label totals.
pub fn decode_payload(payload: &[u8]) -> AirLossGateSummary {
    let mut s = AirLossGateSummary::default();
    if payload.len() < PAYLOAD_HEADER_LEN {
        s.parse_errors.push(format!(
            "payload {} < header {}",
            payload.len(),
            PAYLOAD_HEADER_LEN
        ));
        return s;
    }
    let declared_rows = u32::from_le_bytes(payload[8..12].try_into().unwrap());
    s.status_flags = u32::from_le_bytes(payload[12..16].try_into().unwrap());
    s.schema_version = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    s.field_count = u32::from_le_bytes(payload[20..24].try_into().unwrap());
    s.row_len = u32::from_le_bytes(payload[24..28].try_into().unwrap());
    if s.field_count != EXPECTED_FIELD_COUNT {
        s.parse_errors.push(format!(
            "field_count {} != {}",
            s.field_count, EXPECTED_FIELD_COUNT
        ));
    }
    if s.row_len as usize != ROW_LEN {
        s.parse_errors
            .push(format!("row_len {} != {}", s.row_len, ROW_LEN));
        return s;
    }
    let body = &payload[PAYLOAD_HEADER_LEN..];
    if body.len() % ROW_LEN != 0 {
        s.parse_errors.push(format!(
            "body {} not a multiple of row_len {}",
            body.len(),
            ROW_LEN
        ));
        return s;
    }
    let n = body.len() / ROW_LEN;
    if n as u32 != declared_rows {
        s.parse_errors
            .push(format!("declared_rows {} != decoded {}", declared_rows, n));
    }
    for i in 0..n {
        let o = i * ROW_LEN;
        let flags = u32::from_le_bytes(body[o + 8..o + 12].try_into().unwrap());
        if flags & ROW_FLAG_BINDING_VERIFIED != 0 {
            s.binding_verified_rows += 1;
        }
        for (k, (_, label)) in PINNED_CHANNELS.iter().enumerate() {
            let vo = o + 16 + k * 8;
            let raw = i64::from_le_bytes(body[vo..vo + 8].try_into().unwrap());
            if raw < 0 || raw > MAX_CHANNEL_RAW {
                s.parse_errors
                    .push(format!("row {i} channel {label} raw {raw} out of range"));
            }
            *s.channel_totals.entry(*label).or_insert(0) += raw;
        }
        s.row_count += 1;
    }
    s
}

pub fn decode_payload_rows(
    payload: &[u8],
) -> Result<(AirLossStatHeader, Vec<AirLossStatRow>), Vec<String>> {
    let mut errors = Vec::new();
    if payload.len() < PAYLOAD_HEADER_LEN {
        return Err(vec![format!(
            "payload {} < header {}",
            payload.len(),
            PAYLOAD_HEADER_LEN
        )]);
    }
    let header = AirLossStatHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().unwrap()),
        status_flags: u32::from_le_bytes(payload[12..16].try_into().unwrap()),
        schema_version: u32::from_le_bytes(payload[16..20].try_into().unwrap()),
        field_count: u32::from_le_bytes(payload[20..24].try_into().unwrap()),
    };
    let declared_rows = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let row_len = u32::from_le_bytes(payload[24..28].try_into().unwrap()) as usize;
    if header.schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "schema_version {} != {}",
            header.schema_version, SCHEMA_VERSION
        ));
    }
    if header.field_count != EXPECTED_FIELD_COUNT {
        errors.push(format!(
            "field_count {} != {}",
            header.field_count, EXPECTED_FIELD_COUNT
        ));
    }
    if row_len != ROW_LEN {
        errors.push(format!("row_len {} != {}", row_len, ROW_LEN));
    }
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(declared_rows.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        errors.push(format!(
            "payload len {} != expected {}",
            payload.len(),
            expected_len
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut rows = Vec::with_capacity(declared_rows);
    for i in 0..declared_rows {
        let o = PAYLOAD_HEADER_LEN + i * ROW_LEN;
        let flags = u32::from_le_bytes(payload[o + 8..o + 12].try_into().unwrap());
        if flags & !(ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED) != 0 {
            errors.push(format!("row {i} has unknown flags {flags:#x}"));
        }
        if flags & (ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED)
            != (ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED)
        {
            errors.push(format!("row {i} is missing required validation flags"));
        }
        let mut channels = [0i64; 6];
        for (k, (_, label)) in PINNED_CHANNELS.iter().enumerate() {
            let vo = o + 16 + k * 8;
            let raw = i64::from_le_bytes(payload[vo..vo + 8].try_into().unwrap());
            if raw < 0 || raw > MAX_CHANNEL_RAW {
                errors.push(format!("row {i} channel {label} raw {raw} out of range"));
            }
            channels[k] = raw;
        }
        rows.push(AirLossStatRow {
            owner_tag_db_id: u32::from_le_bytes(payload[o..o + 4].try_into().unwrap()),
            country_idx: u32::from_le_bytes(payload[o + 4..o + 8].try_into().unwrap()),
            flags,
            channels,
        });
    }
    if errors.is_empty() {
        Ok((header, rows))
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// live derivation (fail-closed) — testable via the Mem trait (no live process needed)
// ---------------------------------------------------------------------------

/// Minimal read-only memory interface. `None` on any unreadable address (torn read) so the
/// derivation can fail the row closed rather than fabricate a value.
pub trait Mem {
    fn u32(&self, addr: u64) -> Option<u32>;
    fn u64(&self, addr: u64) -> Option<u64>;
    fn i64(&self, addr: u64) -> Option<i64>;
}

/// Result of reading one country slot. Clean null optional links are positive
/// absence; unreadable fields and failed guards are failures, never absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirLossRowRead {
    Present(AirLossStatRow),
    Absent,
    Failed,
}

fn ok_ptr(v: u64) -> bool {
    (0x1_0000..0x7FFF_FFFF_FFFF).contains(&v)
}

/// Derive one country's air-loss row, or `None` (fail closed). Emits ONLY when the country_idx
/// binding is verified AND the CLoopHistory structure matches AND all six pinned channels read
/// in range. `country_idx` is the stats-registry slot (== tag_db_id after the binding check).
pub fn derive_country_row<M: Mem>(
    mem: &M,
    module_base: u64,
    country_idx: u32,
) -> Option<AirLossStatRow> {
    let cgs = mem.u64(module_base + CGAMESTATE_RVA)?;
    if !ok_ptr(cgs) {
        return None;
    }
    // --- binding guard: country_table[idx] is a CCountry with tag_db_id == idx ---
    let country_table = mem.u64(cgs + COUNTRY_TABLE_OFF)?;
    let country_count = mem.u32(cgs + COUNTRY_COUNT_OFF)?;
    if !ok_ptr(country_table) || country_idx >= country_count {
        return None;
    }
    let cc = mem.u64(country_table + u64::from(country_idx) * 8)?;
    if !ok_ptr(cc) {
        return None;
    }
    if mem.u64(cc)? != module_base + COUNTRY_VTABLE_RVA {
        return None; // not a CCountry — binding unverifiable, fail closed
    }
    let tag_db_id = mem.u32(cc + COUNTRY_TAG_DB_ID_OFF)?;
    if tag_db_id != country_idx {
        return None; // index not parallel for this slot — fail closed
    }
    // --- stats chain ---
    let registry = mem.u64(cgs + REGISTRY_OFF)?;
    let mgr_array = mem.u64(registry + MGR_ARRAY_OFF)?;
    if !ok_ptr(registry) || !ok_ptr(mgr_array) {
        return None;
    }
    let country_mgr = mem.u64(mgr_array + u64::from(country_idx) * 8)?;
    if !ok_ptr(country_mgr) {
        return None;
    }
    let stat_base = mem.u64(country_mgr + STAT_BASE_OFF)?;
    if !ok_ptr(stat_base) {
        return None;
    }
    let loophist = mem.u64(stat_base + AIR_STAT_KEY * 8)?;
    if !ok_ptr(loophist) {
        return None; // country carries no air stat -> ABSENT (not zero)
    }
    // --- structural guard ---
    if mem.u64(loophist)? != module_base + LOOPHIST_VTABLE_RVA {
        return None;
    }
    if mem.u32(loophist + LOOPHIST_FIELDCOUNT_OFF)? != EXPECTED_FIELD_COUNT {
        return None;
    }
    if mem.u64(loophist + LOOPHIST_SELF_REF_OFF)? != loophist {
        return None;
    }
    let window = mem.u64(loophist + LOOPHIST_WINDOW_OFF)?;
    if !ok_ptr(window) {
        return None;
    }
    if mem.u32(window + LOOPHIST_WINDOW_CAPACITY_OFF)? != EXPECTED_YEARLY_CAPACITY {
        return None;
    }
    let recflds = mem.u64(window + LOOPHIST_RECFLDS_OFF)?;
    if !ok_ptr(recflds) {
        return None;
    }
    let arr = mem.u64(recflds)?;
    if !ok_ptr(arr) {
        return None;
    }
    let mut channels = [0i64; 6];
    for (k, (idx, _)) in PINNED_CHANNELS.iter().enumerate() {
        let raw = mem.i64(arr + u64::from(*idx) * 8)?;
        if raw < 0 || raw > MAX_CHANNEL_RAW {
            return None; // implausible -> fail the whole row closed
        }
        channels[k] = raw;
    }
    Some(AirLossStatRow {
        owner_tag_db_id: tag_db_id,
        country_idx,
        flags: ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED,
        channels,
    })
}

/// Tri-state production reader. Clean null optional links are `Absent`; every
/// unreadable field or failed structural guard is `Failed`.
pub fn derive_country_row_result<M: Mem>(
    mem: &M,
    module_base: u64,
    country_idx: u32,
) -> AirLossRowRead {
    let Some(cgs) = mem.u64(module_base + CGAMESTATE_RVA) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(cgs) {
        return AirLossRowRead::Failed;
    }
    let Some(country_table) = mem.u64(cgs + COUNTRY_TABLE_OFF) else {
        return AirLossRowRead::Failed;
    };
    let Some(country_count) = mem.u32(cgs + COUNTRY_COUNT_OFF) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(country_table) || country_idx >= country_count {
        return AirLossRowRead::Failed;
    }
    let Some(cc) = mem.u64(country_table + u64::from(country_idx) * 8) else {
        return AirLossRowRead::Failed;
    };
    if cc == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(cc) {
        return AirLossRowRead::Failed;
    }
    let Some(country_vtable) = mem.u64(cc) else {
        return AirLossRowRead::Failed;
    };
    if country_vtable != module_base + COUNTRY_VTABLE_RVA {
        return AirLossRowRead::Failed;
    }
    let Some(tag_db_id) = mem.u32(cc + COUNTRY_TAG_DB_ID_OFF) else {
        return AirLossRowRead::Failed;
    };
    if tag_db_id != country_idx {
        return AirLossRowRead::Failed;
    }
    let Some(registry) = mem.u64(cgs + REGISTRY_OFF) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(registry) {
        return AirLossRowRead::Failed;
    }
    let Some(mgr_array) = mem.u64(registry + MGR_ARRAY_OFF) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(mgr_array) {
        return AirLossRowRead::Failed;
    }
    let Some(country_mgr) = mem.u64(mgr_array + u64::from(country_idx) * 8) else {
        return AirLossRowRead::Failed;
    };
    if country_mgr == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(country_mgr) {
        return AirLossRowRead::Failed;
    }
    let Some(stat_base) = mem.u64(country_mgr + STAT_BASE_OFF) else {
        return AirLossRowRead::Failed;
    };
    if stat_base == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(stat_base) {
        return AirLossRowRead::Failed;
    }
    let Some(loophist) = mem.u64(stat_base + AIR_STAT_KEY * 8) else {
        return AirLossRowRead::Failed;
    };
    if loophist == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(loophist) {
        return AirLossRowRead::Failed;
    }
    let Some(loophist_vtable) = mem.u64(loophist) else {
        return AirLossRowRead::Failed;
    };
    if loophist_vtable != module_base + LOOPHIST_VTABLE_RVA {
        return AirLossRowRead::Failed;
    }
    let Some(field_count) = mem.u32(loophist + LOOPHIST_FIELDCOUNT_OFF) else {
        return AirLossRowRead::Failed;
    };
    if field_count != EXPECTED_FIELD_COUNT {
        return AirLossRowRead::Failed;
    }
    let Some(self_ref) = mem.u64(loophist + LOOPHIST_SELF_REF_OFF) else {
        return AirLossRowRead::Failed;
    };
    if self_ref != loophist {
        return AirLossRowRead::Failed;
    }
    let Some(window) = mem.u64(loophist + LOOPHIST_WINDOW_OFF) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(window) {
        return AirLossRowRead::Failed;
    }
    let Some(window_capacity) = mem.u32(window + LOOPHIST_WINDOW_CAPACITY_OFF) else {
        return AirLossRowRead::Failed;
    };
    if window_capacity != EXPECTED_YEARLY_CAPACITY {
        return AirLossRowRead::Failed;
    }
    let Some(recflds) = mem.u64(window + LOOPHIST_RECFLDS_OFF) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(recflds) {
        return AirLossRowRead::Failed;
    }
    let Some(arr) = mem.u64(recflds) else {
        return AirLossRowRead::Failed;
    };
    if !ok_ptr(arr) {
        return AirLossRowRead::Failed;
    }
    let mut channels = [0i64; 6];
    for (k, (idx, _)) in PINNED_CHANNELS.iter().enumerate() {
        let Some(raw) = mem.i64(arr + u64::from(*idx) * 8) else {
            return AirLossRowRead::Failed;
        };
        if raw < 0 || raw > MAX_CHANNEL_RAW {
            return AirLossRowRead::Failed;
        }
        channels[k] = raw;
    }
    AirLossRowRead::Present(AirLossStatRow {
        owner_tag_db_id: tag_db_id,
        country_idx,
        flags: ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const BASE: u64 = 0x1_4000_0000;

    struct MockMem {
        u: HashMap<u64, u64>,
        i: HashMap<u64, i64>,
        w: HashMap<u64, u32>,
    }
    impl MockMem {
        fn new() -> Self {
            MockMem {
                u: HashMap::new(),
                i: HashMap::new(),
                w: HashMap::new(),
            }
        }
        fn su(&mut self, a: u64, v: u64) {
            self.u.insert(a, v);
        }
        fn si(&mut self, a: u64, v: i64) {
            self.i.insert(a, v);
        }
        fn sw(&mut self, a: u64, v: u32) {
            self.w.insert(a, v);
        }
    }
    impl Mem for MockMem {
        fn u32(&self, a: u64) -> Option<u32> {
            self.w.get(&a).copied()
        }
        fn u64(&self, a: u64) -> Option<u64> {
            self.u.get(&a).copied()
        }
        fn i64(&self, a: u64) -> Option<i64> {
            self.i.get(&a).copied()
        }
    }

    /// Build a memory image that resolves the full chain for one country (idx=1).
    fn good_image(idx: u32) -> MockMem {
        let mut m = MockMem::new();
        let cgs = 0x2000_0000_u64;
        let country_table = 0x2100_0000_u64;
        let cc = 0x2200_0000_u64;
        let registry = 0x2300_0000_u64;
        let mgr_array = 0x2400_0000_u64;
        let country_mgr = 0x2500_0000_u64;
        let stat_base = 0x2600_0000_u64;
        let loophist = 0x2700_0000_u64;
        let window = 0x2800_0000_u64;
        let recflds = 0x2900_0000_u64;
        let arr = 0x2A00_0000_u64;
        m.su(BASE + CGAMESTATE_RVA, cgs);
        // binding
        m.su(cgs + COUNTRY_TABLE_OFF, country_table);
        m.sw(cgs + COUNTRY_COUNT_OFF, 440);
        m.su(country_table + u64::from(idx) * 8, cc);
        m.su(cc, BASE + COUNTRY_VTABLE_RVA);
        m.sw(cc + COUNTRY_TAG_DB_ID_OFF, idx);
        // stats chain
        m.su(cgs + REGISTRY_OFF, registry);
        m.su(registry + MGR_ARRAY_OFF, mgr_array);
        m.su(mgr_array + u64::from(idx) * 8, country_mgr);
        m.su(country_mgr + STAT_BASE_OFF, stat_base);
        m.su(stat_base + AIR_STAT_KEY * 8, loophist);
        m.su(loophist, BASE + LOOPHIST_VTABLE_RVA);
        m.su(loophist + LOOPHIST_SELF_REF_OFF, loophist);
        m.sw(loophist + LOOPHIST_FIELDCOUNT_OFF, EXPECTED_FIELD_COUNT);
        m.su(loophist + LOOPHIST_WINDOW_OFF, window);
        m.sw(
            window + LOOPHIST_WINDOW_CAPACITY_OFF,
            EXPECTED_YEARLY_CAPACITY,
        );
        m.su(window + LOOPHIST_RECFLDS_OFF, recflds);
        m.su(recflds, arr);
        // pinned channel values (planes × scale)
        let vals = [3i64, 10, 44, 51, 58, 80];
        for (k, (fi, _)) in PINNED_CHANNELS.iter().enumerate() {
            m.si(arr + u64::from(*fi) * 8, vals[k] * FIELD_SCALE);
        }
        m
    }

    #[test]
    fn derives_full_chain_when_bound() {
        let m = good_image(1);
        let row = derive_country_row(&m, BASE, 1).expect("row");
        assert_eq!(row.owner_tag_db_id, 1);
        assert_eq!(row.country_idx, 1);
        assert!(row.flags & ROW_FLAG_BINDING_VERIFIED != 0);
        assert!(row.flags & ROW_FLAG_STRUCT_VERIFIED != 0);
        assert_eq!(row.channels[0], 3 * FIELD_SCALE);
        assert_eq!(row.channel_planes(5), 80.0);
    }

    #[test]
    fn fails_closed_on_binding_tag_mismatch() {
        let mut m = good_image(1);
        // corrupt the tag_db_id so it no longer equals idx
        m.sw(0x2200_0000 + COUNTRY_TAG_DB_ID_OFF, 999);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_wrong_country_vtable() {
        let mut m = good_image(1);
        m.su(0x2200_0000, BASE + 0xDEAD); // wrong CCountry vtable
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_field_count_mismatch() {
        let mut m = good_image(1);
        m.sw(0x2700_0000 + LOOPHIST_FIELDCOUNT_OFF, 64);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_wrong_loophistory_vtable() {
        let mut m = good_image(1);
        m.su(0x2700_0000, BASE + 0x1234);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_loophistory_self_ref_mismatch() {
        let mut m = good_image(1);
        m.su(0x2700_0000 + LOOPHIST_SELF_REF_OFF, 0x2700_0010);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_yearly_capacity_mismatch() {
        let mut m = good_image(1);
        m.sw(0x2800_0000 + LOOPHIST_WINDOW_CAPACITY_OFF, 1);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_when_air_stat_absent() {
        let mut m = good_image(1);
        // country carries no key-23 CLoopHistory (peaceful country) -> absent, not zero
        m.su(0x2600_0000 + AIR_STAT_KEY * 8, 0);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_out_of_range_channel() {
        let mut m = good_image(1);
        m.si(0x2A00_0000 + 3 * 8, MAX_CHANNEL_RAW + 1);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_torn_read() {
        let mut m = good_image(1);
        m.u.remove(&(0x2700_0000 + LOOPHIST_WINDOW_OFF)); // window read now returns None
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn tri_state_distinguishes_clean_absence_from_torn_read() {
        let mut absent = good_image(1);
        absent.su(0x2600_0000 + AIR_STAT_KEY * 8, 0);
        assert_eq!(
            derive_country_row_result(&absent, BASE, 1),
            AirLossRowRead::Absent
        );

        let mut torn = good_image(1);
        torn.u.remove(&(0x2700_0000 + LOOPHIST_WINDOW_OFF));
        assert_eq!(
            derive_country_row_result(&torn, BASE, 1),
            AirLossRowRead::Failed
        );
    }

    #[test]
    fn tri_state_reports_present_row() {
        assert!(matches!(
            derive_country_row_result(&good_image(1), BASE, 1),
            AirLossRowRead::Present(_)
        ));
    }

    #[test]
    fn fails_closed_when_idx_beyond_country_count() {
        let m = good_image(1);
        assert!(derive_country_row(&m, BASE, 441).is_none());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let rows = vec![
            derive_country_row(&good_image(1), BASE, 1).unwrap(),
            AirLossStatRow {
                owner_tag_db_id: 16,
                country_idx: 16,
                flags: ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED,
                channels: [100_000, 0, 200_000, 0, 0, 300_000],
            },
        ];
        let header = AirLossStatHeader {
            elapsed_wall_ms: 42,
            status_flags: STATUS_VALUE_VALIDATION_FIXTURE_REQUIRED,
            schema_version: SCHEMA_VERSION,
            field_count: EXPECTED_FIELD_COUNT,
        };
        let payload = encode_payload(&header, &rows);
        assert_eq!(payload.len(), PAYLOAD_HEADER_LEN + 2 * ROW_LEN);
        let s = decode_payload(&payload);
        assert!(s.parse_errors.is_empty(), "{:?}", s.parse_errors);
        assert_eq!(s.row_count, 2);
        assert_eq!(s.binding_verified_rows, 2);
        assert_eq!(s.field_count, EXPECTED_FIELD_COUNT);
        assert_eq!(s.row_len as usize, ROW_LEN);
        assert_eq!(
            *s.channel_totals.get("our_fighters_lost").unwrap(),
            3 * FIELD_SCALE + 100_000
        );
        assert_eq!(
            *s.channel_totals.get("enemy_bombers_disrupted").unwrap(),
            80 * FIELD_SCALE + 300_000
        );
    }

    #[test]
    fn decode_rejects_stale_row_len() {
        let header = AirLossStatHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            schema_version: SCHEMA_VERSION,
            field_count: EXPECTED_FIELD_COUNT,
        };
        let mut payload = encode_payload(&header, &[]);
        payload[24..28].copy_from_slice(&48u32.to_le_bytes()); // corrupt row_len (!= ROW_LEN)
        let s = decode_payload(&payload);
        assert!(s.parse_errors.iter().any(|e| e.contains("row_len")));
    }

    #[test]
    fn decode_rejects_bad_field_count() {
        let header = AirLossStatHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            schema_version: SCHEMA_VERSION,
            field_count: 64,
        };
        let payload = encode_payload(&header, &[]);
        let s = decode_payload(&payload);
        assert!(s.parse_errors.iter().any(|e| e.contains("field_count")));
    }

    #[test]
    fn decode_rows_returns_strict_rows() {
        let row = derive_country_row(&good_image(1), BASE, 1).unwrap();
        let payload = encode_payload(
            &AirLossStatHeader {
                elapsed_wall_ms: 7,
                status_flags: STATUS_OK,
                schema_version: SCHEMA_VERSION,
                field_count: EXPECTED_FIELD_COUNT,
            },
            &[row],
        );
        let (header, rows) = decode_payload_rows(&payload).expect("strict rows");
        assert_eq!(header.elapsed_wall_ms, 7);
        assert_eq!(rows, vec![row]);
    }

    #[test]
    fn pinned_channels_are_the_six_value_pinned() {
        let labels: Vec<&str> = PINNED_CHANNELS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            vec![
                "our_fighters_lost",
                "our_bombers_lost",
                "enemy_fighters_lost",
                "enemy_bombers_lost",
                "enemy_cas_lost",
                "enemy_bombers_disrupted",
            ]
        );
    }
}
