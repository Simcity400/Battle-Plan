use std::collections::BTreeSet;

pub mod source_evidence;
pub mod stream_certification;

pub const CHARACTER_VTABLE_RVA_870EBF: usize = 0x2967E30;
pub const SCIENTIST_VTABLE_RVA_870EBF: usize = 0x2967E88;

pub const COUNTRY_OFF_SCIENTIST_ROSTER: usize = 0xFF0;
pub const SCIENTIST_ROSTER_VECTOR: usize = 0xA0;
pub const SCIENTIST_ROSTER_VECTOR_COUNT: usize = 0x0C;
pub const SCIENTIST_ROSTER_WRAPPER_CHARACTER: usize = 0x08;

pub const CHARACTER_TYPE: u32 = 73;
pub const CHARACTER_OFF_TYPE: usize = 0x08;
pub const CHARACTER_OFF_ID: usize = 0x0C;
pub const CHARACTER_OFF_DISPLAY: usize = 0x28;
pub const CHARACTER_OFF_TOKEN: usize = 0x48;
pub const CHARACTER_OFF_SCIENTIST_PTR: usize = 0xC0;

pub const SCIENTIST_OFF_TRAIT_COUNT: usize = 0x38;
pub const SCIENTIST_OFF_SKILLS_PTR: usize = 0x110;
pub const SKILL_STRIDE: usize = 0x20;
pub const SKILL_LEVEL_OFF: usize = 0x18;
pub const SKILL_HEADER_BASE_870EBF: u16 = 0x56C6;

pub const PROJECTS_LIVE_DOMAIN_ID: u16 = 41;
pub const COMPACT_PROJECT_HEADER_LEN: usize = 64;
pub const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1: u32 = 3;
pub const COMPACT_PROJECT_ROOT_COUNTERS_LEN: usize = 24;
pub const COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN: usize = 48;
pub const COMPACT_PROJECT_PROGRESS_COUNTERS_LEN: usize = 16;
pub const COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN: usize = COMPACT_PROJECT_HEADER_LEN
    + COMPACT_PROJECT_ROOT_COUNTERS_LEN
    + COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN
    + COMPACT_PROJECT_PROGRESS_COUNTERS_LEN;
pub const COMPACT_PROJECT_SOURCE_SCAN: u32 = 1;

pub const MAX_SCIENTISTS_PER_COUNTRY: u32 = 512;
pub const MAX_SCIENTISTS_TOTAL: usize = 4_096;
pub const MAX_SCIENTIST_TOKEN_LEN: usize = 160;
pub const MAX_SCIENTIST_DISPLAY_LEN: usize = 160;
pub const MAX_TRAITS: u32 = 256;
pub const MAX_SKILL_LEVEL: i32 = 1_000;
pub const MAX_CHARACTER_ID: u32 = 10_000_000;

pub const COMPACT_SCIENTIST_HEADER_LEN: usize = 36;
pub const COMPACT_SCIENTIST_ROW_LEN: usize = 32;
pub const COMPACT_SCIENTIST_FLAG_NAME_TOKEN: u32 = 1;
pub const COMPACT_SCIENTIST_FLAG_DISPLAY_NAME: u32 = 1 << 1;
pub const COMPACT_SCIENTIST_FLAG_TRAIT_COUNT: u32 = 1 << 2;
pub const COMPACT_SCIENTIST_FLAG_SKILLS: u32 = 1 << 3;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;

pub const HEAP_LO: usize = 0x10000000000;
pub const HEAP_HI: usize = 0x7FF000000000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScientistProfile {
    pub module_base: usize,
    pub character_vtable_rva: usize,
    pub scientist_vtable_rva: usize,
    pub skill_header_base: u16,
}

impl ScientistProfile {
    pub fn vanilla_870ebf(module_base: usize) -> Self {
        Self {
            module_base,
            character_vtable_rva: CHARACTER_VTABLE_RVA_870EBF,
            scientist_vtable_rva: SCIENTIST_VTABLE_RVA_870EBF,
            skill_header_base: SKILL_HEADER_BASE_870EBF,
        }
    }

    fn character_vtable(self) -> usize {
        self.module_base.saturating_add(self.character_vtable_rva)
    }

    fn scientist_vtable(self) -> usize {
        self.module_base.saturating_add(self.scientist_vtable_rva)
    }
}

pub trait Memory {
    fn read_exact(&self, addr: usize, out: &mut [u8]) -> bool;

    fn u16(&self, addr: usize) -> Option<u16> {
        let mut bytes = [0u8; 2];
        self.read_exact(addr, &mut bytes)
            .then(|| u16::from_le_bytes(bytes))
    }

    fn u32(&self, addr: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read_exact(addr, &mut bytes)
            .then(|| u32::from_le_bytes(bytes))
    }

    fn i32(&self, addr: usize) -> Option<i32> {
        let mut bytes = [0u8; 4];
        self.read_exact(addr, &mut bytes)
            .then(|| i32::from_le_bytes(bytes))
    }

    fn u64(&self, addr: usize) -> Option<u64> {
        let mut bytes = [0u8; 8];
        self.read_exact(addr, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScientistPin {
    pub character_addr: usize,
    pub char_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScientistRow {
    pub char_id: u32,
    pub name_token: Option<String>,
    pub display_name: Option<String>,
    pub specialization_nuclear: i32,
    pub specialization_naval: i32,
    pub specialization_air: i32,
    pub specialization_land: i32,
    pub trait_count: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtractionCounters {
    pub countries_seen: u32,
    /// Countries whose optional scientist-roster root was read as absent/empty.
    /// This is a measured empty source, not a source failure.
    pub absent_roster_roots: u32,
    pub roster_vectors_seen: u32,
    pub observed_pins: u32,
    pub emitted_rows: u32,
    pub invalid_roster_roots: u32,
    pub invalid_roster_slots: u32,
    pub invalid_scientist_rows: u32,
    pub duplicate_pins: u32,
    pub duplicate_character_ids: u32,
    pub too_many: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractionResult {
    pub rows: Vec<ScientistRow>,
    pub counters: ExtractionCounters,
    pub status_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    ShortHeader,
    BadHeader,
    BadLength,
    BadCount,
    BadFlags,
    BadCharId,
    BadTraitCount,
    BadSkill,
    BadString,
    DuplicateCharId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedScientistPayload {
    pub elapsed_wall_us: u64,
    pub status_flags: u32,
    pub observed_pins: u32,
    pub rows: Vec<ScientistRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedProjectsLiveScientistPayload {
    pub elapsed_wall_ms: u64,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_pins: u32,
    pub rows: Vec<ScientistRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateReport {
    pub ok: bool,
    pub problems: Vec<&'static str>,
    pub row_count: usize,
    pub observed_pins: u32,
}

pub fn extract_scientists<M: Memory>(
    mem: &M,
    country_ptrs: &[usize],
    profile: ScientistProfile,
) -> ExtractionResult {
    let mut counters = ExtractionCounters::default();
    let mut seen_addresses = BTreeSet::new();
    let mut seen_character_ids = BTreeSet::new();
    let mut rows = Vec::new();
    for &country_ptr in country_ptrs {
        counters.countries_seen = counters.countries_seen.saturating_add(1);
        match scientist_pins_from_country(mem, country_ptr, profile) {
            RootRead::Absent => {
                counters.absent_roster_roots = counters.absent_roster_roots.saturating_add(1);
            }
            RootRead::Invalid => {
                counters.invalid_roster_roots = counters.invalid_roster_roots.saturating_add(1);
            }
            RootRead::Partial(pins) => {
                counters.roster_vectors_seen = counters.roster_vectors_seen.saturating_add(1);
                counters.invalid_roster_slots = counters.invalid_roster_slots.saturating_add(1);
                collect_rows(
                    mem,
                    profile,
                    pins,
                    &mut seen_addresses,
                    &mut seen_character_ids,
                    &mut rows,
                    &mut counters,
                );
            }
            RootRead::Valid(pins) => {
                counters.roster_vectors_seen = counters.roster_vectors_seen.saturating_add(1);
                collect_rows(
                    mem,
                    profile,
                    pins,
                    &mut seen_addresses,
                    &mut seen_character_ids,
                    &mut rows,
                    &mut counters,
                );
            }
        }
        if seen_addresses.len() > MAX_SCIENTISTS_TOTAL {
            counters.too_many = true;
            rows.clear();
            break;
        }
    }
    counters.emitted_rows = rows.len().min(u32::MAX as usize) as u32;
    let mut status_flags = 0;
    let measured_roster_sources = counters
        .absent_roster_roots
        .saturating_add(counters.roster_vectors_seen);
    if counters.too_many || measured_roster_sources == 0 {
        status_flags |= STATUS_SOURCE_UNAVAILABLE;
    }
    if counters.invalid_roster_roots > 0
        || counters.invalid_roster_slots > 0
        || counters.invalid_scientist_rows > 0
        || counters.duplicate_character_ids > 0
        || counters.too_many
        || counters.emitted_rows < counters.observed_pins
    {
        status_flags |= STATUS_PARTIAL_DATA;
    }
    ExtractionResult {
        rows,
        counters,
        status_flags,
    }
}

fn collect_rows<M: Memory>(
    mem: &M,
    profile: ScientistProfile,
    pins: Vec<ScientistPin>,
    seen_addresses: &mut BTreeSet<usize>,
    seen_character_ids: &mut BTreeSet<u32>,
    rows: &mut Vec<ScientistRow>,
    counters: &mut ExtractionCounters,
) {
    for pin in pins {
        if !seen_addresses.insert(pin.character_addr) {
            counters.duplicate_pins = counters.duplicate_pins.saturating_add(1);
            continue;
        }
        counters.observed_pins = counters.observed_pins.saturating_add(1);
        if !seen_character_ids.insert(pin.char_id) {
            counters.duplicate_character_ids = counters.duplicate_character_ids.saturating_add(1);
            continue;
        }
        match read_scientist_row(mem, pin, profile) {
            Some(row) => rows.push(row),
            None => {
                counters.invalid_scientist_rows = counters.invalid_scientist_rows.saturating_add(1);
            }
        }
    }
}

enum RootRead<T> {
    Absent,
    Valid(T),
    Partial(T),
    Invalid,
}

fn scientist_pins_from_country<M: Memory>(
    mem: &M,
    country_ptr: usize,
    profile: ScientistProfile,
) -> RootRead<Vec<ScientistPin>> {
    if !heap_ptr(country_ptr) {
        return RootRead::Invalid;
    }
    let Some(roster_root) = mem
        .u64(country_ptr.saturating_add(COUNTRY_OFF_SCIENTIST_ROSTER))
        .map(|value| value as usize)
    else {
        return RootRead::Invalid;
    };
    if roster_root == 0 {
        return RootRead::Absent;
    }
    if !heap_ptr(roster_root) {
        return RootRead::Invalid;
    }
    let header = roster_root.saturating_add(SCIENTIST_ROSTER_VECTOR);
    let Some(count) = mem.u32(header.saturating_add(SCIENTIST_ROSTER_VECTOR_COUNT)) else {
        return RootRead::Invalid;
    };
    if count == 0 {
        return RootRead::Absent;
    }
    if count > MAX_SCIENTISTS_PER_COUNTRY {
        return RootRead::Invalid;
    }
    let Some(begin) = mem.u64(header).map(|value| value as usize) else {
        return RootRead::Invalid;
    };
    if !heap_ptr(begin) {
        return RootRead::Invalid;
    }
    let mut pins = Vec::new();
    for index in 0..count as usize {
        let Some(wrapper) = mem
            .u64(begin.saturating_add(index.saturating_mul(8)))
            .map(|value| value as usize)
        else {
            return partial_or_invalid(pins);
        };
        if !heap_ptr(wrapper) {
            return partial_or_invalid(pins);
        }
        let Some(character_addr) = mem
            .u64(wrapper.saturating_add(SCIENTIST_ROSTER_WRAPPER_CHARACTER))
            .map(|value| value as usize)
        else {
            return partial_or_invalid(pins);
        };
        let Some(pin) = validate_scientist_pin(mem, character_addr, profile) else {
            return partial_or_invalid(pins);
        };
        pins.push(pin);
    }
    RootRead::Valid(pins)
}

fn partial_or_invalid<T>(rows: Vec<T>) -> RootRead<Vec<T>> {
    if rows.is_empty() {
        RootRead::Invalid
    } else {
        RootRead::Partial(rows)
    }
}

fn validate_scientist_pin<M: Memory>(
    mem: &M,
    character_addr: usize,
    profile: ScientistProfile,
) -> Option<ScientistPin> {
    if !heap_ptr(character_addr) || mem.u64(character_addr)? as usize != profile.character_vtable()
    {
        return None;
    }
    if mem.u32(character_addr.saturating_add(CHARACTER_OFF_TYPE))? != CHARACTER_TYPE {
        return None;
    }
    let char_id = mem.u32(character_addr.saturating_add(CHARACTER_OFF_ID))?;
    if char_id == 0 || char_id > MAX_CHARACTER_ID {
        return None;
    }
    let scientist_ptr =
        mem.u64(character_addr.saturating_add(CHARACTER_OFF_SCIENTIST_PTR))? as usize;
    if !heap_ptr(scientist_ptr) || mem.u64(scientist_ptr)? as usize != profile.scientist_vtable() {
        return None;
    }
    Some(ScientistPin {
        character_addr,
        char_id,
    })
}

fn read_scientist_row<M: Memory>(
    mem: &M,
    pin: ScientistPin,
    profile: ScientistProfile,
) -> Option<ScientistRow> {
    let current = validate_scientist_pin(mem, pin.character_addr, profile)?;
    if current.char_id != pin.char_id {
        return None;
    }
    let scientist_ptr = mem.u64(
        pin.character_addr
            .saturating_add(CHARACTER_OFF_SCIENTIST_PTR),
    )? as usize;
    let trait_count = mem
        .u32(scientist_ptr.saturating_add(SCIENTIST_OFF_TRAIT_COUNT))
        .filter(|value| *value <= MAX_TRAITS);
    let skills = read_skills(mem, scientist_ptr, profile.skill_header_base)?;
    Some(ScientistRow {
        char_id: pin.char_id,
        name_token: read_msvc_string(
            mem,
            pin.character_addr.saturating_add(CHARACTER_OFF_TOKEN),
            MAX_SCIENTIST_TOKEN_LEN,
        )
        .filter(|value| is_script_token(value)),
        display_name: read_msvc_string(
            mem,
            pin.character_addr.saturating_add(CHARACTER_OFF_DISPLAY),
            MAX_SCIENTIST_DISPLAY_LEN,
        ),
        specialization_nuclear: skills[0],
        specialization_naval: skills[1],
        specialization_air: skills[2],
        specialization_land: skills[3],
        trait_count,
    })
}

fn read_skills<M: Memory>(mem: &M, scientist_ptr: usize, header_base: u16) -> Option<[i32; 4]> {
    let skills_ptr = mem.u64(scientist_ptr.saturating_add(SCIENTIST_OFF_SKILLS_PTR))? as usize;
    if !heap_ptr(skills_ptr) {
        return None;
    }
    let mut out = [0i32; 4];
    for (idx, slot) in out.iter_mut().enumerate() {
        let rec = skills_ptr.saturating_add(idx.saturating_mul(SKILL_STRIDE));
        if mem.u16(rec)? != header_base.checked_add(idx as u16)? {
            return None;
        }
        let level = mem.i32(rec.saturating_add(SKILL_LEVEL_OFF))?;
        if !(0..=MAX_SKILL_LEVEL).contains(&level) {
            return None;
        }
        *slot = level;
    }
    Some(out)
}

fn read_msvc_string<M: Memory>(mem: &M, addr: usize, max_len: usize) -> Option<String> {
    let size = mem.u64(addr.saturating_add(0x10))? as usize;
    let cap = mem.u64(addr.saturating_add(0x18))? as usize;
    if size == 0 || size > max_len || cap < size {
        return None;
    }
    let data_addr = if cap < 16 {
        addr
    } else {
        let ptr = mem.u64(addr)? as usize;
        if !heap_ptr(ptr) {
            return None;
        }
        ptr
    };
    let mut bytes = vec![0u8; size];
    if !mem.read_exact(data_addr, &mut bytes) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn is_script_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SCIENTIST_TOKEN_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

pub fn encode_scientist_payload(
    rows: &[ScientistRow],
    elapsed_wall_us: u64,
    status_flags: u32,
    observed_pins: u32,
) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let mut payload =
        Vec::with_capacity(COMPACT_SCIENTIST_HEADER_LEN + rows.len() * COMPACT_SCIENTIST_ROW_LEN);
    payload.extend_from_slice(&elapsed_wall_us.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&observed_pins.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        let mut flags = COMPACT_SCIENTIST_FLAG_SKILLS;
        if row.name_token.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_NAME_TOKEN;
        }
        if row.display_name.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_DISPLAY_NAME;
        }
        if row.trait_count.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_TRAIT_COUNT;
        }
        payload.extend_from_slice(&row.char_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.trait_count.unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(&row.specialization_nuclear.to_le_bytes());
        payload.extend_from_slice(&row.specialization_naval.to_le_bytes());
        payload.extend_from_slice(&row.specialization_air.to_le_bytes());
        payload.extend_from_slice(&row.specialization_land.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in rows {
        append_compact_string(&mut payload, row.name_token.as_deref());
        append_compact_string(&mut payload, row.display_name.as_deref());
    }
    payload
}

pub fn encode_projects_live_scientists_payload(
    rows: &[ScientistRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_pins: u32,
) -> Vec<u8> {
    let scientist_count = rows.len().min(u32::MAX as usize) as u32;
    let row_count = scientist_count;
    let mut payload = Vec::with_capacity(
        COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN
            + rows.len() * COMPACT_SCIENTIST_ROW_LEN
            + rows.len() * 4,
    );
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&scientist_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&observed_pins.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&COMPACT_PROJECT_SOURCE_SCAN.to_le_bytes());
    payload.extend_from_slice(&COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.resize(COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN, 0);
    append_project_scientist_rows(&mut payload, rows);
    for row in rows {
        append_compact_string(&mut payload, row.name_token.as_deref());
        append_compact_string(&mut payload, row.display_name.as_deref());
    }
    payload
}

pub fn decode_scientist_payload(payload: &[u8]) -> Result<DecodedScientistPayload, DecodeError> {
    if payload.len() < COMPACT_SCIENTIST_HEADER_LEN {
        return Err(DecodeError::ShortHeader);
    }
    let elapsed_wall_us = read_u64_le(payload, 0);
    let row_count = read_u32_le(payload, 8) as usize;
    let status_flags = read_u32_le(payload, 12);
    let observed_pins = read_u32_le(payload, 16);
    let emitted_rows = read_u32_le(payload, 20) as usize;
    if emitted_rows != row_count || row_count > MAX_SCIENTISTS_TOTAL {
        return Err(DecodeError::BadCount);
    }
    let Some(rows_end) = COMPACT_SCIENTIST_HEADER_LEN
        .checked_add(row_count.saturating_mul(COMPACT_SCIENTIST_ROW_LEN))
    else {
        return Err(DecodeError::BadLength);
    };
    if payload.len() < rows_end {
        return Err(DecodeError::BadLength);
    }
    let mut cursor = COMPACT_SCIENTIST_HEADER_LEN;
    let mut rows = Vec::with_capacity(row_count);
    let mut flags = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    for _ in 0..row_count {
        let char_id = read_u32_le(payload, cursor);
        let row_flags = read_u32_le(payload, cursor + 4);
        let trait_count = read_u32_le(payload, cursor + 8);
        let nuclear = read_i32_le(payload, cursor + 12);
        let naval = read_i32_le(payload, cursor + 16);
        let air = read_i32_le(payload, cursor + 20);
        let land = read_i32_le(payload, cursor + 24);
        let reserved = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_SCIENTIST_ROW_LEN;
        if char_id == 0 || char_id > MAX_CHARACTER_ID {
            return Err(DecodeError::BadCharId);
        }
        if !seen.insert(char_id) {
            return Err(DecodeError::DuplicateCharId);
        }
        let known_flags = COMPACT_SCIENTIST_FLAG_NAME_TOKEN
            | COMPACT_SCIENTIST_FLAG_DISPLAY_NAME
            | COMPACT_SCIENTIST_FLAG_TRAIT_COUNT
            | COMPACT_SCIENTIST_FLAG_SKILLS;
        if row_flags & !known_flags != 0 || reserved != 0 {
            return Err(DecodeError::BadFlags);
        }
        if trait_count != u32::MAX && trait_count > MAX_TRAITS {
            return Err(DecodeError::BadTraitCount);
        }
        if trait_count == u32::MAX && row_flags & COMPACT_SCIENTIST_FLAG_TRAIT_COUNT != 0 {
            return Err(DecodeError::BadTraitCount);
        }
        for skill in [nuclear, naval, air, land] {
            if !(0..=MAX_SKILL_LEVEL).contains(&skill) {
                return Err(DecodeError::BadSkill);
            }
        }
        flags.push(row_flags);
        rows.push(ScientistRow {
            char_id,
            name_token: None,
            display_name: None,
            specialization_nuclear: nuclear,
            specialization_naval: naval,
            specialization_air: air,
            specialization_land: land,
            trait_count: (trait_count != u32::MAX).then_some(trait_count),
        });
    }
    for (row, row_flags) in rows.iter_mut().zip(flags.into_iter()) {
        let token = read_compact_string(payload, &mut cursor, MAX_SCIENTIST_TOKEN_LEN)?;
        let display = read_compact_string(payload, &mut cursor, MAX_SCIENTIST_DISPLAY_LEN)?;
        if token.is_some() != (row_flags & COMPACT_SCIENTIST_FLAG_NAME_TOKEN != 0) {
            return Err(DecodeError::BadString);
        }
        if display.is_some() != (row_flags & COMPACT_SCIENTIST_FLAG_DISPLAY_NAME != 0) {
            return Err(DecodeError::BadString);
        }
        row.name_token = token;
        row.display_name = display;
    }
    if cursor != payload.len() {
        return Err(DecodeError::BadLength);
    }
    Ok(DecodedScientistPayload {
        elapsed_wall_us,
        status_flags,
        observed_pins,
        rows,
    })
}

pub fn decode_projects_live_scientists_payload(
    payload: &[u8],
) -> Result<DecodedProjectsLiveScientistPayload, DecodeError> {
    if payload.len() < COMPACT_PROJECT_HEADER_LEN {
        return Err(DecodeError::ShortHeader);
    }
    let elapsed_wall_ms = read_u64_le(payload, 0);
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12);
    let status_flags = read_u32_le(payload, 16);
    let facility_count = read_u32_le(payload, 20) as usize;
    let project_count = read_u32_le(payload, 24) as usize;
    let scientist_count = read_u32_le(payload, 28) as usize;
    let observed_scientist_rows = read_u32_le(payload, 40);
    let bridge_rows_supplied = read_u32_le(payload, 44);
    let source_kind = read_u32_le(payload, 48);
    let header_extension = read_u32_le(payload, 52);
    if header_extension != COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
        || source_kind != COMPACT_PROJECT_SOURCE_SCAN
        || bridge_rows_supplied != 0
    {
        return Err(DecodeError::BadHeader);
    }
    if payload.len() < COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN {
        return Err(DecodeError::BadLength);
    }
    let project_state_count = read_u32_le(payload, 136) as usize;
    let breakthrough_count = read_u32_le(payload, 140) as usize;
    if facility_count != 0
        || project_count != 0
        || project_state_count != 0
        || breakthrough_count != 0
        || row_count != scientist_count
        || scientist_count > MAX_SCIENTISTS_TOTAL
    {
        return Err(DecodeError::BadCount);
    }
    let mut cursor = COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN;
    let (mut rows, flags) = decode_fixed_scientist_rows(payload, &mut cursor, scientist_count)?;
    decode_scientist_strings(payload, &mut cursor, &mut rows, &flags)?;
    if cursor != payload.len() {
        return Err(DecodeError::BadLength);
    }
    Ok(DecodedProjectsLiveScientistPayload {
        elapsed_wall_ms,
        changed_rows,
        status_flags,
        observed_pins: observed_scientist_rows,
        rows,
    })
}

pub fn gate_payload(decoded: &DecodedScientistPayload) -> GateReport {
    let mut problems = Vec::new();
    if decoded.status_flags & STATUS_SOURCE_UNAVAILABLE != 0 {
        problems.push("source_unavailable");
    }
    if decoded.status_flags & STATUS_PARTIAL_DATA != 0 {
        problems.push("partial_data");
    }
    if decoded.rows.is_empty() {
        problems.push("no_scientist_rows");
    }
    if decoded.rows.len() as u32 != decoded.observed_pins {
        problems.push("observed_emitted_mismatch");
    }
    GateReport {
        ok: problems.is_empty(),
        problems,
        row_count: decoded.rows.len(),
        observed_pins: decoded.observed_pins,
    }
}

pub fn gate_projects_live_scientist_payload(
    decoded: &DecodedProjectsLiveScientistPayload,
) -> GateReport {
    let mut problems = Vec::new();
    if decoded.status_flags & STATUS_SOURCE_UNAVAILABLE != 0 {
        problems.push("source_unavailable");
    }
    if decoded.status_flags & STATUS_PARTIAL_DATA != 0 {
        problems.push("partial_data");
    }
    if decoded.rows.is_empty() {
        problems.push("no_scientist_rows");
    }
    if decoded.rows.len() as u32 != decoded.observed_pins {
        problems.push("observed_emitted_mismatch");
    }
    GateReport {
        ok: problems.is_empty(),
        problems,
        row_count: decoded.rows.len(),
        observed_pins: decoded.observed_pins,
    }
}

fn append_project_scientist_rows(payload: &mut Vec<u8>, rows: &[ScientistRow]) {
    for row in rows {
        let mut flags = COMPACT_SCIENTIST_FLAG_SKILLS;
        if row.name_token.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_NAME_TOKEN;
        }
        if row.display_name.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_DISPLAY_NAME;
        }
        if row.trait_count.is_some() {
            flags |= COMPACT_SCIENTIST_FLAG_TRAIT_COUNT;
        }
        payload.extend_from_slice(&row.char_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.trait_count.unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(&row.specialization_nuclear.to_le_bytes());
        payload.extend_from_slice(&row.specialization_naval.to_le_bytes());
        payload.extend_from_slice(&row.specialization_air.to_le_bytes());
        payload.extend_from_slice(&row.specialization_land.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
}

fn decode_fixed_scientist_rows(
    payload: &[u8],
    cursor: &mut usize,
    row_count: usize,
) -> Result<(Vec<ScientistRow>, Vec<u32>), DecodeError> {
    let Some(rows_end) = cursor.checked_add(row_count.saturating_mul(COMPACT_SCIENTIST_ROW_LEN))
    else {
        return Err(DecodeError::BadLength);
    };
    if payload.len() < rows_end {
        return Err(DecodeError::BadLength);
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut flags = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    for _ in 0..row_count {
        let char_id = read_u32_le(payload, *cursor);
        let row_flags = read_u32_le(payload, *cursor + 4);
        let trait_count = read_u32_le(payload, *cursor + 8);
        let nuclear = read_i32_le(payload, *cursor + 12);
        let naval = read_i32_le(payload, *cursor + 16);
        let air = read_i32_le(payload, *cursor + 20);
        let land = read_i32_le(payload, *cursor + 24);
        let reserved = read_u32_le(payload, *cursor + 28);
        *cursor += COMPACT_SCIENTIST_ROW_LEN;
        if char_id == 0 || char_id > MAX_CHARACTER_ID {
            return Err(DecodeError::BadCharId);
        }
        if !seen.insert(char_id) {
            return Err(DecodeError::DuplicateCharId);
        }
        let known_flags = COMPACT_SCIENTIST_FLAG_NAME_TOKEN
            | COMPACT_SCIENTIST_FLAG_DISPLAY_NAME
            | COMPACT_SCIENTIST_FLAG_TRAIT_COUNT
            | COMPACT_SCIENTIST_FLAG_SKILLS;
        if row_flags & !known_flags != 0 || reserved != 0 {
            return Err(DecodeError::BadFlags);
        }
        if trait_count != u32::MAX && trait_count > MAX_TRAITS {
            return Err(DecodeError::BadTraitCount);
        }
        if trait_count == u32::MAX && row_flags & COMPACT_SCIENTIST_FLAG_TRAIT_COUNT != 0 {
            return Err(DecodeError::BadTraitCount);
        }
        for skill in [nuclear, naval, air, land] {
            if !(0..=MAX_SKILL_LEVEL).contains(&skill) {
                return Err(DecodeError::BadSkill);
            }
        }
        flags.push(row_flags);
        rows.push(ScientistRow {
            char_id,
            name_token: None,
            display_name: None,
            specialization_nuclear: nuclear,
            specialization_naval: naval,
            specialization_air: air,
            specialization_land: land,
            trait_count: (trait_count != u32::MAX).then_some(trait_count),
        });
    }
    Ok((rows, flags))
}

fn decode_scientist_strings(
    payload: &[u8],
    cursor: &mut usize,
    rows: &mut [ScientistRow],
    flags: &[u32],
) -> Result<(), DecodeError> {
    for (row, row_flags) in rows.iter_mut().zip(flags.iter().copied()) {
        let token = read_compact_string(payload, cursor, MAX_SCIENTIST_TOKEN_LEN)?;
        let display = read_compact_string(payload, cursor, MAX_SCIENTIST_DISPLAY_LEN)?;
        if token.is_some() != (row_flags & COMPACT_SCIENTIST_FLAG_NAME_TOKEN != 0) {
            return Err(DecodeError::BadString);
        }
        if display.is_some() != (row_flags & COMPACT_SCIENTIST_FLAG_DISPLAY_NAME != 0) {
            return Err(DecodeError::BadString);
        }
        row.name_token = token;
        row.display_name = display;
    }
    Ok(())
}

fn append_compact_string(payload: &mut Vec<u8>, value: Option<&str>) {
    if let Some(value) = value {
        payload.extend_from_slice(&(value.len().min(u16::MAX as usize) as u16).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    } else {
        payload.extend_from_slice(&0u16.to_le_bytes());
    }
}

fn read_compact_string(
    payload: &[u8],
    cursor: &mut usize,
    max_len: usize,
) -> Result<Option<String>, DecodeError> {
    if *cursor + 2 > payload.len() {
        return Err(DecodeError::BadString);
    }
    let len = u16::from_le_bytes([payload[*cursor], payload[*cursor + 1]]) as usize;
    *cursor += 2;
    if len == 0 {
        return Ok(None);
    }
    if len > max_len || *cursor + len > payload.len() {
        return Err(DecodeError::BadString);
    }
    let value = String::from_utf8(payload[*cursor..*cursor + len].to_vec())
        .map_err(|_| DecodeError::BadString)?;
    *cursor += len;
    Ok(Some(value))
}

fn heap_ptr(value: usize) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value) && value % 8 == 0
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
}

fn read_u64_le(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FixtureMemory {
        bytes: BTreeMap<usize, u8>,
    }

    impl FixtureMemory {
        fn write(&mut self, addr: usize, bytes: &[u8]) {
            for (offset, byte) in bytes.iter().copied().enumerate() {
                self.bytes.insert(addr + offset, byte);
            }
        }

        fn put_u16(&mut self, addr: usize, value: u16) {
            self.write(addr, &value.to_le_bytes());
        }

        fn put_u32(&mut self, addr: usize, value: u32) {
            self.write(addr, &value.to_le_bytes());
        }

        fn put_i32(&mut self, addr: usize, value: i32) {
            self.write(addr, &value.to_le_bytes());
        }

        fn put_u64(&mut self, addr: usize, value: u64) {
            self.write(addr, &value.to_le_bytes());
        }

        fn sso_string(&mut self, addr: usize, value: &str) {
            self.write(addr, value.as_bytes());
            self.put_u64(addr + 0x10, value.len() as u64);
            self.put_u64(addr + 0x18, 15);
        }
    }

    impl Memory for FixtureMemory {
        fn read_exact(&self, addr: usize, out: &mut [u8]) -> bool {
            for (offset, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(addr + offset)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    fn fixture(valid_skill_header: bool) -> (FixtureMemory, ScientistProfile, usize) {
        let base = HEAP_LO;
        let profile = ScientistProfile::vanilla_870ebf(base);
        let country = HEAP_LO + 0x10_000;
        let roster = HEAP_LO + 0x20_000;
        let slots = HEAP_LO + 0x30_000;
        let wrapper = HEAP_LO + 0x40_000;
        let character = HEAP_LO + 0x50_000;
        let scientist = HEAP_LO + 0x60_000;
        let skills = HEAP_LO + 0x70_000;
        let mut mem = FixtureMemory::default();
        mem.put_u64(country + COUNTRY_OFF_SCIENTIST_ROSTER, roster as u64);
        mem.put_u64(roster + SCIENTIST_ROSTER_VECTOR, slots as u64);
        mem.put_u32(
            roster + SCIENTIST_ROSTER_VECTOR + SCIENTIST_ROSTER_VECTOR_COUNT,
            1,
        );
        mem.put_u64(slots, wrapper as u64);
        mem.put_u64(
            wrapper + SCIENTIST_ROSTER_WRAPPER_CHARACTER,
            character as u64,
        );
        mem.put_u64(character, profile.character_vtable() as u64);
        mem.put_u32(character + CHARACTER_OFF_TYPE, CHARACTER_TYPE);
        mem.put_u32(character + CHARACTER_OFF_ID, 44);
        mem.sso_string(character + CHARACTER_OFF_TOKEN, "POR_scientist");
        mem.sso_string(character + CHARACTER_OFF_DISPLAY, "Ana Ferreira");
        mem.put_u64(character + CHARACTER_OFF_SCIENTIST_PTR, scientist as u64);
        mem.put_u64(scientist, profile.scientist_vtable() as u64);
        mem.put_u32(scientist + SCIENTIST_OFF_TRAIT_COUNT, 3);
        mem.put_u64(scientist + SCIENTIST_OFF_SKILLS_PTR, skills as u64);
        let header_base = if valid_skill_header {
            profile.skill_header_base
        } else {
            0x56F7
        };
        for (idx, level) in [4, 0, 1, 2].iter().copied().enumerate() {
            let rec = skills + idx * SKILL_STRIDE;
            mem.put_u16(rec, header_base + idx as u16);
            mem.put_i32(rec + SKILL_LEVEL_OFF, level);
        }
        (mem, profile, country)
    }

    #[test]
    fn extracts_scientist_rows_from_validated_country_roster() {
        let (mem, profile, country) = fixture(true);
        let result = extract_scientists(&mem, &[country], profile);

        assert_eq!(result.status_flags, 0);
        assert_eq!(result.counters.observed_pins, 1);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].char_id, 44);
        assert_eq!(result.rows[0].name_token.as_deref(), Some("POR_scientist"));
        assert_eq!(result.rows[0].display_name.as_deref(), Some("Ana Ferreira"));
        assert_eq!(result.rows[0].specialization_nuclear, 4);
        assert_eq!(result.rows[0].specialization_land, 2);
        assert_eq!(result.rows[0].trait_count, Some(3));
    }

    #[test]
    fn rejects_old_skill_header_base_as_partial_data() {
        let (mem, profile, country) = fixture(false);
        let result = extract_scientists(&mem, &[country], profile);

        assert!(result.rows.is_empty());
        assert_eq!(result.counters.observed_pins, 1);
        assert_eq!(result.counters.invalid_scientist_rows, 1);
        assert_ne!(result.status_flags & STATUS_PARTIAL_DATA, 0);
        assert_eq!(result.status_flags & STATUS_SOURCE_UNAVAILABLE, 0);
    }

    #[test]
    fn compact_payload_round_trips_and_gates_clean() {
        let (mem, profile, country) = fixture(true);
        let result = extract_scientists(&mem, &[country], profile);
        let payload = encode_scientist_payload(
            &result.rows,
            123,
            result.status_flags,
            result.counters.observed_pins,
        );
        let decoded = decode_scientist_payload(&payload).unwrap();
        let gate = gate_payload(&decoded);

        assert_eq!(decoded.elapsed_wall_us, 123);
        assert_eq!(decoded.rows, result.rows);
        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.row_count, 1);
        assert_eq!(gate.observed_pins, 1);
    }

    #[test]
    fn projects_live_domain_payload_round_trips_and_gates_clean() {
        let (mem, profile, country) = fixture(true);
        let result = extract_scientists(&mem, &[country], profile);
        let payload = encode_projects_live_scientists_payload(
            &result.rows,
            456,
            result.status_flags,
            result.counters.observed_pins,
        );

        assert_eq!(PROJECTS_LIVE_DOMAIN_ID, 41);
        assert_eq!(
            payload.len(),
            COMPACT_PROJECT_HEADER_PROGRESS_V1_LEN
                + COMPACT_SCIENTIST_ROW_LEN
                + 2
                + "POR_scientist".len()
                + 2
                + "Ana Ferreira".len()
        );
        assert_eq!(read_u32_le(&payload, 8), 1);
        assert_eq!(read_u32_le(&payload, 28), 1);
        assert_eq!(read_u32_le(&payload, 40), 1);
        assert_eq!(
            read_u32_le(&payload, 52),
            COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
        );

        let decoded = decode_projects_live_scientists_payload(&payload).unwrap();
        let gate = gate_projects_live_scientist_payload(&decoded);

        assert_eq!(decoded.elapsed_wall_ms, 456);
        assert_eq!(decoded.rows, result.rows);
        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.row_count, 1);
        assert_eq!(gate.observed_pins, 1);
    }

    #[test]
    fn projects_live_domain_gate_reports_partial_or_observed_mismatch() {
        let row = ScientistRow {
            char_id: 44,
            name_token: None,
            display_name: None,
            specialization_nuclear: 1,
            specialization_naval: 2,
            specialization_air: 3,
            specialization_land: 4,
            trait_count: Some(1),
        };
        let payload = encode_projects_live_scientists_payload(&[row], 0, STATUS_PARTIAL_DATA, 2);
        let decoded = decode_projects_live_scientists_payload(&payload).unwrap();
        let gate = gate_projects_live_scientist_payload(&decoded);

        assert!(!gate.ok);
        assert_eq!(
            gate.problems,
            vec!["partial_data", "observed_emitted_mismatch"]
        );
    }

    #[test]
    fn compact_decoder_rejects_duplicate_char_ids() {
        let row = ScientistRow {
            char_id: 44,
            name_token: None,
            display_name: None,
            specialization_nuclear: 1,
            specialization_naval: 2,
            specialization_air: 3,
            specialization_land: 4,
            trait_count: Some(1),
        };
        let payload = encode_scientist_payload(&[row.clone(), row], 0, 0, 2);

        assert_eq!(
            decode_scientist_payload(&payload),
            Err(DecodeError::DuplicateCharId)
        );
    }

    #[test]
    fn gate_reports_partial_or_observed_mismatch() {
        let row = ScientistRow {
            char_id: 44,
            name_token: None,
            display_name: None,
            specialization_nuclear: 1,
            specialization_naval: 2,
            specialization_air: 3,
            specialization_land: 4,
            trait_count: None,
        };
        let payload = encode_scientist_payload(&[row], 0, STATUS_PARTIAL_DATA, 2);
        let decoded = decode_scientist_payload(&payload).unwrap();
        let gate = gate_payload(&decoded);

        assert!(!gate.ok);
        assert_eq!(
            gate.problems,
            vec!["partial_data", "observed_emitted_mismatch"]
        );
    }

    #[test]
    fn absent_optional_roster_is_measured_empty_not_source_unavailable() {
        let profile = ScientistProfile::vanilla_870ebf(HEAP_LO);
        let country = HEAP_LO + 0x90_000;
        let mut mem = FixtureMemory::default();
        mem.put_u64(country + COUNTRY_OFF_SCIENTIST_ROSTER, 0);

        let result = extract_scientists(&mem, &[country], profile);

        assert!(result.rows.is_empty());
        assert_eq!(result.counters.absent_roster_roots, 1);
        assert_eq!(result.status_flags, 0);
    }

    #[test]
    fn duplicate_cross_country_pin_does_not_inflate_source_observed_roster() {
        let (mut mem, profile, first_country) = fixture(true);
        let second_country = HEAP_LO + 0x91_000;
        let second_roster = HEAP_LO + 0x92_000;
        let second_slots = HEAP_LO + 0x93_000;
        let shared_wrapper = HEAP_LO + 0x40_000;
        mem.put_u64(
            second_country + COUNTRY_OFF_SCIENTIST_ROSTER,
            second_roster as u64,
        );
        mem.put_u64(second_roster + SCIENTIST_ROSTER_VECTOR, second_slots as u64);
        mem.put_u32(
            second_roster + SCIENTIST_ROSTER_VECTOR + SCIENTIST_ROSTER_VECTOR_COUNT,
            1,
        );
        mem.put_u64(second_slots, shared_wrapper as u64);

        let result = extract_scientists(&mem, &[first_country, second_country], profile);

        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.counters.observed_pins, 1);
        assert_eq!(result.counters.duplicate_pins, 1);
        assert_eq!(result.status_flags, 0);
    }

    #[test]
    fn no_country_roster_authority_fails_closed() {
        let mem = FixtureMemory::default();
        let profile = ScientistProfile::vanilla_870ebf(HEAP_LO);
        let result = extract_scientists(&mem, &[], profile);
        assert_ne!(result.status_flags & STATUS_SOURCE_UNAVAILABLE, 0);
    }

    #[test]
    fn nonnull_invalid_roster_root_is_not_clean_absence() {
        let profile = ScientistProfile::vanilla_870ebf(HEAP_LO);
        let country = HEAP_LO + 0x94_000;
        let mut mem = FixtureMemory::default();
        mem.put_u64(country + COUNTRY_OFF_SCIENTIST_ROSTER, (HEAP_LO + 1) as u64);

        let result = extract_scientists(&mem, &[country], profile);

        assert_eq!(result.counters.absent_roster_roots, 0);
        assert_eq!(result.counters.invalid_roster_roots, 1);
        assert_ne!(result.status_flags & STATUS_SOURCE_UNAVAILABLE, 0);
        assert_ne!(result.status_flags & STATUS_PARTIAL_DATA, 0);
    }
}
