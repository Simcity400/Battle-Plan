use std::collections::BTreeMap;

pub const DOMAIN_ID: u16 = 61;
pub const DOMAIN_NAME: &str = "air_loss_stats_live";

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
pub const BUILDINGS_BOMBED_DISPLAY_SCALE: i64 = 1_000;

pub const COUNTRY_TABLE_OFF: u64 = 0x310;
pub const COUNTRY_COUNT_OFF: u64 = 0x31C;
pub const COUNTRY_TAG_EQUIVALENCE_OFF: u64 = 0x340;
pub const COUNTRY_VTABLE_RVA: u64 = 0x27D0E90;
pub const COUNTRY_TAG_DB_ID_OFF: u64 = 0x08;

pub const PINNED_CHANNELS: &[(u32, &str)] = &[
    (0, "our_buildings_bombed"),
    (1, "our_troops_bombed"),
    (2, "our_ships_bombed"),
    (3, "our_fighters_lost"),
    (10, "our_bombers_lost"),
    (17, "our_cas_lost"),
    (24, "our_transports_lost"),
    (39, "our_bombers_disrupted"),
    (36, "our_trains_lost"),
    (37, "our_trucks_lost"),
    (38, "our_railway_guns_lost"),
    (40, "our_missiles_lost"),
    (41, "enemy_buildings_bombed"),
    (42, "enemy_troops_bombed"),
    (43, "enemy_ships_sunk"),
    (44, "enemy_fighters_lost"),
    (51, "enemy_bombers_lost"),
    (58, "enemy_cas_lost"),
    (65, "enemy_transports_lost"),
    (80, "enemy_bombers_disrupted"),
    (77, "enemy_trains_lost"),
    (78, "enemy_trucks_lost"),
    (79, "enemy_railway_guns_lost"),
    (81, "enemy_missiles_lost"),
];

pub const CHANNEL_COUNT: usize = 24;

pub const CHANNEL_DISPLAY_DIVISORS: [i64; CHANNEL_COUNT] = [
    BUILDINGS_BOMBED_DISPLAY_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    BUILDINGS_BOMBED_DISPLAY_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
    FIELD_SCALE,
];

pub const MAX_CHANNEL_RAW: i64 = 1_000_000_000_000;

pub const SCHEMA_VERSION: u32 = 2;
pub const PAYLOAD_HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 208;

pub const STATUS_OK: u32 = 0;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 0;
pub const STATUS_BINDING_UNVERIFIED: u32 = 1 << 1;
pub const STATUS_PARTIAL: u32 = 1 << 2;
/// Legacy lane-local status from the pre-promotion packet. Production emission
/// no longer sets this after D04 1944 value validation.
///
/// This wording is a GATE INPUT, not decoration: `compact_contract`'s
/// `lane_status_bit_sweep` allow-lists domain 61 bits 0..=3 as provably
/// never-emitted and reads this exact sentence back as the retirement proof
/// (`compact_contract/tests/compact_contract_suite/lane_status_bit_sweep.rs` `NEVER_EMITTED`). An
/// external comment-strip removed it and the sweep failed; do not delete it
/// without re-triaging the bits there.
pub const STATUS_VALUE_VALIDATION_FIXTURE_REQUIRED: u32 = 1 << 3;

pub const ROW_FLAG_BINDING_VERIFIED: u32 = 1 << 0;
pub const ROW_FLAG_STRUCT_VERIFIED: u32 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirLossStatRow {
    pub owner_tag_db_id: u32,
    pub country_idx: u32,
    pub flags: u32,
    pub channels: [i64; CHANNEL_COUNT],
}

impl AirLossStatRow {
    pub fn channel_value(&self, n: usize) -> f64 {
        self.channels[n] as f64 / CHANNEL_DISPLAY_DIVISORS[n] as f64
    }

    pub fn channel_planes(&self, n: usize) -> f64 {
        self.channel_value(n)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirLossStatHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub schema_version: u32,
    pub field_count: u32,
}

pub fn encode_payload(header: &AirLossStatHeader, rows: &[AirLossStatRow]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * ROW_LEN);
    p.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    p.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    p.extend_from_slice(&header.status_flags.to_le_bytes());
    p.extend_from_slice(&header.schema_version.to_le_bytes());
    p.extend_from_slice(&header.field_count.to_le_bytes());
    p.extend_from_slice(&(ROW_LEN as u32).to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(p.len(), PAYLOAD_HEADER_LEN);
    for r in rows {
        p.extend_from_slice(&r.owner_tag_db_id.to_le_bytes());
        p.extend_from_slice(&r.country_idx.to_le_bytes());
        p.extend_from_slice(&r.flags.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
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
    pub channel_totals: BTreeMap<&'static str, i64>,
    pub parse_errors: Vec<String>,
}

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
        let mut channels = [0i64; CHANNEL_COUNT];
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

pub trait Mem {
    fn u32(&self, addr: u64) -> Option<u32>;
    fn u64(&self, addr: u64) -> Option<u64>;
    fn i64(&self, addr: u64) -> Option<i64>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AirLossRowFailureReason {
    CGameStateUnreadable = 1,
    CGameStateInvalid = 2,
    CountryTableUnreadable = 3,
    CountryCountUnreadable = 4,
    CountryTableInvalid = 5,
    CountryIndexOutOfBounds = 6,
    CountrySlotUnreadable = 7,
    CountryPointerInvalid = 8,
    CountryVtableUnreadable = 9,
    CountryVtableMismatch = 10,
    CountryTagUnreadable = 11,
    CountryTagMismatch = 12,
    RegistryUnreadable = 13,
    RegistryInvalid = 14,
    ManagerArrayUnreadable = 15,
    ManagerArrayInvalid = 16,
    CountryManagerUnreadable = 17,
    CountryManagerInvalid = 18,
    StatBaseUnreadable = 19,
    StatBaseInvalid = 20,
    LoopHistoryUnreadable = 21,
    LoopHistoryInvalid = 22,
    LoopHistoryVtableUnreadable = 23,
    LoopHistoryVtableMismatch = 24,
    FieldCountUnreadable = 25,
    FieldCountMismatch = 26,
    SelfReferenceUnreadable = 27,
    SelfReferenceMismatch = 28,
    WindowUnreadable = 29,
    WindowInvalid = 30,
    WindowCapacityUnreadable = 31,
    WindowCapacityMismatch = 32,
    RecordFieldsUnreadable = 33,
    RecordFieldsInvalid = 34,
    FieldArrayUnreadable = 35,
    FieldArrayInvalid = 36,
    ChannelUnreadable = 37,
    ChannelOutOfRange = 38,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AirLossRowFailure {
    pub country_idx: u32,
    pub field_index: Option<u32>,
    pub address: u64,
    pub reason: AirLossRowFailureReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirLossRowRead {
    Present(AirLossStatRow),
    Absent,
    Failed(AirLossRowFailure),
}

fn ok_ptr(v: u64) -> bool {
    (0x1_0000..0x7FFF_FFFF_FFFF).contains(&v)
}

pub fn derive_country_row<M: Mem>(
    mem: &M,
    module_base: u64,
    country_idx: u32,
) -> Option<AirLossStatRow> {
    let cgs = mem.u64(module_base + CGAMESTATE_RVA)?;
    if !ok_ptr(cgs) {
        return None;
    }
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
        return None;
    }
    let tag_db_id = mem.u32(cc + COUNTRY_TAG_DB_ID_OFF)?;
    if tag_db_id != country_idx {
        return None;
    }
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
        return None;
    }
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
    let mut channels = [0i64; CHANNEL_COUNT];
    for (k, (idx, _)) in PINNED_CHANNELS.iter().enumerate() {
        let raw = mem.i64(arr + u64::from(*idx) * 8)?;
        if raw < 0 || raw > MAX_CHANNEL_RAW {
            return None;
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

pub fn derive_country_row_result<M: Mem>(
    mem: &M,
    module_base: u64,
    country_idx: u32,
) -> AirLossRowRead {
    let failed = |reason, address, field_index| {
        AirLossRowRead::Failed(AirLossRowFailure {
            country_idx,
            field_index,
            address,
            reason,
        })
    };
    let cgs_addr = module_base + CGAMESTATE_RVA;
    let Some(cgs) = mem.u64(cgs_addr) else {
        return failed(
            AirLossRowFailureReason::CGameStateUnreadable,
            cgs_addr,
            None,
        );
    };
    if !ok_ptr(cgs) {
        return failed(AirLossRowFailureReason::CGameStateInvalid, cgs_addr, None);
    }
    let country_table_addr = cgs + COUNTRY_TABLE_OFF;
    let Some(country_table) = mem.u64(country_table_addr) else {
        return failed(
            AirLossRowFailureReason::CountryTableUnreadable,
            country_table_addr,
            None,
        );
    };
    let country_count_addr = cgs + COUNTRY_COUNT_OFF;
    let Some(country_count) = mem.u32(country_count_addr) else {
        return failed(
            AirLossRowFailureReason::CountryCountUnreadable,
            country_count_addr,
            None,
        );
    };
    if !ok_ptr(country_table) {
        return failed(
            AirLossRowFailureReason::CountryTableInvalid,
            country_table_addr,
            None,
        );
    }
    if country_idx >= country_count {
        return failed(
            AirLossRowFailureReason::CountryIndexOutOfBounds,
            country_count_addr,
            None,
        );
    }
    let country_slot_addr = country_table + u64::from(country_idx) * 8;
    let Some(cc) = mem.u64(country_slot_addr) else {
        return failed(
            AirLossRowFailureReason::CountrySlotUnreadable,
            country_slot_addr,
            None,
        );
    };
    if cc == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(cc) {
        return failed(
            AirLossRowFailureReason::CountryPointerInvalid,
            country_slot_addr,
            None,
        );
    }
    let Some(country_vtable) = mem.u64(cc) else {
        return failed(AirLossRowFailureReason::CountryVtableUnreadable, cc, None);
    };
    if country_vtable != module_base + COUNTRY_VTABLE_RVA {
        return failed(AirLossRowFailureReason::CountryVtableMismatch, cc, None);
    }
    let country_tag_addr = cc + COUNTRY_TAG_DB_ID_OFF;
    let Some(tag_db_id) = mem.u32(country_tag_addr) else {
        return failed(
            AirLossRowFailureReason::CountryTagUnreadable,
            country_tag_addr,
            None,
        );
    };
    if tag_db_id != country_idx {
        let alias_is_exact = mem
            .u64(cgs + COUNTRY_TAG_EQUIVALENCE_OFF)
            .filter(|equivalence| ok_ptr(*equivalence))
            .and_then(|equivalence| {
                let slot_canonical = mem.u32(equivalence + u64::from(country_idx) * 4)?;
                let object_canonical = mem.u32(equivalence + u64::from(tag_db_id) * 4)?;
                (slot_canonical != 0
                    && slot_canonical < country_count
                    && slot_canonical == object_canonical)
                    .then_some(slot_canonical)
            })
            .and_then(|canonical| {
                mem.u64(country_table + u64::from(canonical) * 8)
                    .map(|canonical_country| canonical_country == cc)
            })
            .unwrap_or(false);
        if alias_is_exact {
            return AirLossRowRead::Absent;
        }
        return failed(
            AirLossRowFailureReason::CountryTagMismatch,
            country_tag_addr,
            None,
        );
    }
    let registry_addr = cgs + REGISTRY_OFF;
    let Some(registry) = mem.u64(registry_addr) else {
        return failed(
            AirLossRowFailureReason::RegistryUnreadable,
            registry_addr,
            None,
        );
    };
    if !ok_ptr(registry) {
        return failed(
            AirLossRowFailureReason::RegistryInvalid,
            registry_addr,
            None,
        );
    }
    let mgr_array_addr = registry + MGR_ARRAY_OFF;
    let Some(mgr_array) = mem.u64(mgr_array_addr) else {
        return failed(
            AirLossRowFailureReason::ManagerArrayUnreadable,
            mgr_array_addr,
            None,
        );
    };
    if !ok_ptr(mgr_array) {
        return failed(
            AirLossRowFailureReason::ManagerArrayInvalid,
            mgr_array_addr,
            None,
        );
    }
    let country_mgr_addr = mgr_array + u64::from(country_idx) * 8;
    let Some(country_mgr) = mem.u64(country_mgr_addr) else {
        return failed(
            AirLossRowFailureReason::CountryManagerUnreadable,
            country_mgr_addr,
            None,
        );
    };
    if country_mgr == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(country_mgr) {
        return failed(
            AirLossRowFailureReason::CountryManagerInvalid,
            country_mgr_addr,
            None,
        );
    }
    let stat_base_addr = country_mgr + STAT_BASE_OFF;
    let Some(stat_base) = mem.u64(stat_base_addr) else {
        return failed(
            AirLossRowFailureReason::StatBaseUnreadable,
            stat_base_addr,
            None,
        );
    };
    if stat_base == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(stat_base) {
        return failed(
            AirLossRowFailureReason::StatBaseInvalid,
            stat_base_addr,
            None,
        );
    }
    let loophist_addr = stat_base + AIR_STAT_KEY * 8;
    let Some(loophist) = mem.u64(loophist_addr) else {
        return failed(
            AirLossRowFailureReason::LoopHistoryUnreadable,
            loophist_addr,
            None,
        );
    };
    if loophist == 0 {
        return AirLossRowRead::Absent;
    }
    if !ok_ptr(loophist) {
        return failed(
            AirLossRowFailureReason::LoopHistoryInvalid,
            loophist_addr,
            None,
        );
    }
    let Some(loophist_vtable) = mem.u64(loophist) else {
        return failed(
            AirLossRowFailureReason::LoopHistoryVtableUnreadable,
            loophist,
            None,
        );
    };
    if loophist_vtable != module_base + LOOPHIST_VTABLE_RVA {
        return failed(
            AirLossRowFailureReason::LoopHistoryVtableMismatch,
            loophist,
            None,
        );
    }
    let field_count_addr = loophist + LOOPHIST_FIELDCOUNT_OFF;
    let Some(field_count) = mem.u32(field_count_addr) else {
        return failed(
            AirLossRowFailureReason::FieldCountUnreadable,
            field_count_addr,
            None,
        );
    };
    if field_count != EXPECTED_FIELD_COUNT {
        return failed(
            AirLossRowFailureReason::FieldCountMismatch,
            field_count_addr,
            None,
        );
    }
    let self_ref_addr = loophist + LOOPHIST_SELF_REF_OFF;
    let Some(self_ref) = mem.u64(self_ref_addr) else {
        return failed(
            AirLossRowFailureReason::SelfReferenceUnreadable,
            self_ref_addr,
            None,
        );
    };
    if self_ref != loophist {
        return failed(
            AirLossRowFailureReason::SelfReferenceMismatch,
            self_ref_addr,
            None,
        );
    }
    let window_addr = loophist + LOOPHIST_WINDOW_OFF;
    let Some(window) = mem.u64(window_addr) else {
        return failed(AirLossRowFailureReason::WindowUnreadable, window_addr, None);
    };
    if !ok_ptr(window) {
        return failed(AirLossRowFailureReason::WindowInvalid, window_addr, None);
    }
    let window_capacity_addr = window + LOOPHIST_WINDOW_CAPACITY_OFF;
    let Some(window_capacity) = mem.u32(window_capacity_addr) else {
        return failed(
            AirLossRowFailureReason::WindowCapacityUnreadable,
            window_capacity_addr,
            None,
        );
    };
    if window_capacity != EXPECTED_YEARLY_CAPACITY {
        return failed(
            AirLossRowFailureReason::WindowCapacityMismatch,
            window_capacity_addr,
            None,
        );
    }
    let recflds_addr = window + LOOPHIST_RECFLDS_OFF;
    let Some(recflds) = mem.u64(recflds_addr) else {
        return failed(
            AirLossRowFailureReason::RecordFieldsUnreadable,
            recflds_addr,
            None,
        );
    };
    if !ok_ptr(recflds) {
        return failed(
            AirLossRowFailureReason::RecordFieldsInvalid,
            recflds_addr,
            None,
        );
    }
    let Some(arr) = mem.u64(recflds) else {
        return failed(AirLossRowFailureReason::FieldArrayUnreadable, recflds, None);
    };
    if !ok_ptr(arr) {
        return failed(AirLossRowFailureReason::FieldArrayInvalid, recflds, None);
    }
    let mut channels = [0i64; CHANNEL_COUNT];
    for (k, (idx, _)) in PINNED_CHANNELS.iter().enumerate() {
        let field_addr = arr + u64::from(*idx) * 8;
        let Some(raw) = mem.i64(field_addr) else {
            return failed(
                AirLossRowFailureReason::ChannelUnreadable,
                field_addr,
                Some(*idx),
            );
        };
        if raw < 0 || raw > MAX_CHANNEL_RAW {
            return failed(
                AirLossRowFailureReason::ChannelOutOfRange,
                field_addr,
                Some(*idx),
            );
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
        m.su(cgs + COUNTRY_TABLE_OFF, country_table);
        m.sw(cgs + COUNTRY_COUNT_OFF, 440);
        m.su(country_table + u64::from(idx) * 8, cc);
        m.su(cc, BASE + COUNTRY_VTABLE_RVA);
        m.sw(cc + COUNTRY_TAG_DB_ID_OFF, idx);
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
        for (k, (fi, _)) in PINNED_CHANNELS.iter().enumerate() {
            let raw = if k == 0 {
                3 * BUILDINGS_BOMBED_DISPLAY_SCALE
            } else {
                i64::from(*fi) * FIELD_SCALE
            };
            m.si(arr + u64::from(*fi) * 8, raw);
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
        assert_eq!(row.channels[0], 3 * BUILDINGS_BOMBED_DISPLAY_SCALE);
        assert_eq!(row.channel_value(0), 3.0);
        assert_eq!(row.channel_planes(19), 80.0);
    }

    #[test]
    fn fails_closed_on_binding_tag_mismatch() {
        let mut m = good_image(1);
        m.sw(0x2200_0000 + COUNTRY_TAG_DB_ID_OFF, 999);
        assert!(derive_country_row(&m, BASE, 1).is_none());
    }

    #[test]
    fn fails_closed_on_wrong_country_vtable() {
        let mut m = good_image(1);
        m.su(0x2200_0000, BASE + 0xDEAD);
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
        m.u.remove(&(0x2700_0000 + LOOPHIST_WINDOW_OFF));
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
        let AirLossRowRead::Failed(failure) = derive_country_row_result(&torn, BASE, 1) else {
            panic!("torn window read must be a diagnosed failure");
        };
        assert_eq!(failure.country_idx, 1);
        assert_eq!(failure.field_index, None);
        assert_eq!(failure.address, 0x2700_0000 + LOOPHIST_WINDOW_OFF);
        assert_eq!(failure.reason, AirLossRowFailureReason::WindowUnreadable);
    }

    #[test]
    fn tri_state_preserves_each_proven_null_optional_link_as_absent() {
        let mut country_absent = good_image(1);
        country_absent.su(0x2100_0000 + 8, 0);
        assert_eq!(
            derive_country_row_result(&country_absent, BASE, 1),
            AirLossRowRead::Absent
        );

        let mut manager_absent = good_image(1);
        manager_absent.su(0x2400_0000 + 8, 0);
        assert_eq!(
            derive_country_row_result(&manager_absent, BASE, 1),
            AirLossRowRead::Absent
        );

        let mut stat_base_absent = good_image(1);
        stat_base_absent.su(0x2500_0000 + STAT_BASE_OFF, 0);
        assert_eq!(
            derive_country_row_result(&stat_base_absent, BASE, 1),
            AirLossRowRead::Absent
        );

        let mut air_stat_absent = good_image(1);
        air_stat_absent.su(0x2600_0000 + AIR_STAT_KEY * 8, 0);
        assert_eq!(
            derive_country_row_result(&air_stat_absent, BASE, 1),
            AirLossRowRead::Absent
        );
    }

    #[test]
    fn tri_state_treats_exact_equivalence_alias_as_measured_absence() {
        let mut alias = good_image(365);
        let cgs = 0x2000_0000_u64;
        let country_table = 0x2100_0000_u64;
        let country = 0x2200_0000_u64;
        let equivalence = 0x2B00_0000_u64;
        let canonical = 1u32;

        alias.sw(country + COUNTRY_TAG_DB_ID_OFF, canonical);
        alias.su(cgs + COUNTRY_TAG_EQUIVALENCE_OFF, equivalence);
        alias.sw(equivalence + 365 * 4, canonical);
        alias.sw(equivalence + u64::from(canonical) * 4, canonical);
        alias.su(country_table + u64::from(canonical) * 8, country);

        assert_eq!(
            derive_country_row_result(&alias, BASE, 365),
            AirLossRowRead::Absent
        );
    }

    #[test]
    fn tri_state_rejects_unjoined_tag_mismatch_as_failure() {
        let mut mismatch = good_image(365);
        let cgs = 0x2000_0000_u64;
        let country = 0x2200_0000_u64;
        let equivalence = 0x2B00_0000_u64;

        mismatch.sw(country + COUNTRY_TAG_DB_ID_OFF, 1);
        mismatch.su(cgs + COUNTRY_TAG_EQUIVALENCE_OFF, equivalence);
        mismatch.sw(equivalence + 365 * 4, 1);
        mismatch.sw(equivalence + 4, 1);
        mismatch.su(0x2100_0000 + 8, 0x2C00_0000);

        let AirLossRowRead::Failed(failure) = derive_country_row_result(&mismatch, BASE, 365)
        else {
            panic!("unjoined mismatch must fail closed");
        };
        assert_eq!(failure.reason, AirLossRowFailureReason::CountryTagMismatch);
    }

    #[test]
    fn tri_state_reports_exact_channel_failure() {
        let mut torn = good_image(1);
        let field_index = PINNED_CHANNELS[3].0;
        let address = 0x2A00_0000 + u64::from(field_index) * 8;
        torn.i.remove(&address);

        let AirLossRowRead::Failed(failure) = derive_country_row_result(&torn, BASE, 1) else {
            panic!("torn channel read must be a diagnosed failure");
        };
        assert_eq!(failure.country_idx, 1);
        assert_eq!(failure.field_index, Some(field_index));
        assert_eq!(failure.address, address);
        assert_eq!(failure.reason, AirLossRowFailureReason::ChannelUnreadable);
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
        let mut additional_channels = [0; CHANNEL_COUNT];
        additional_channels[3] = 100_000;
        additional_channels[19] = 300_000;
        let rows = vec![
            derive_country_row(&good_image(1), BASE, 1).unwrap(),
            AirLossStatRow {
                owner_tag_db_id: 16,
                country_idx: 16,
                flags: ROW_FLAG_BINDING_VERIFIED | ROW_FLAG_STRUCT_VERIFIED,
                channels: additional_channels,
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
        payload[24..28].copy_from_slice(&48u32.to_le_bytes());
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
    fn channels_match_the_full_ui_table_and_labels() {
        let fields: Vec<u32> = PINNED_CHANNELS.iter().map(|(field, _)| *field).collect();
        assert_eq!(
            fields,
            vec![
                0, 1, 2, 3, 10, 17, 24, 39, 36, 37, 38, 40, 41, 42, 43, 44, 51, 58, 65, 80, 77, 78,
                79, 81,
            ]
        );
        let labels: Vec<&str> = PINNED_CHANNELS.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            labels,
            vec![
                "our_buildings_bombed",
                "our_troops_bombed",
                "our_ships_bombed",
                "our_fighters_lost",
                "our_bombers_lost",
                "our_cas_lost",
                "our_transports_lost",
                "our_bombers_disrupted",
                "our_trains_lost",
                "our_trucks_lost",
                "our_railway_guns_lost",
                "our_missiles_lost",
                "enemy_buildings_bombed",
                "enemy_troops_bombed",
                "enemy_ships_sunk",
                "enemy_fighters_lost",
                "enemy_bombers_lost",
                "enemy_cas_lost",
                "enemy_transports_lost",
                "enemy_bombers_disrupted",
                "enemy_trains_lost",
                "enemy_trucks_lost",
                "enemy_railway_guns_lost",
                "enemy_missiles_lost",
            ]
        );
        assert_eq!(PINNED_CHANNELS.len(), CHANNEL_COUNT);
        assert_eq!(CHANNEL_DISPLAY_DIVISORS[0], 1_000);
        assert_eq!(CHANNEL_DISPLAY_DIVISORS[12], 1_000);
        assert!(CHANNEL_DISPLAY_DIVISORS
            .iter()
            .enumerate()
            .all(|(index, divisor)| index == 0 || index == 12 || *divisor == FIELD_SCALE));
    }
}
