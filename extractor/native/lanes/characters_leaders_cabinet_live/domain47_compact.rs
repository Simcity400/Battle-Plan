use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

#[cfg(not(test))]
use compact_contract::{parse_compact_record_header, RecordFramingError};

pub const DOMAIN_ID: u16 = 47;
pub const DOMAIN_NAME: &str = "characters_leaders_cabinet_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const HEADER_LEN: usize = 64;
pub const STRING_NONE: u32 = u32::MAX;
pub const SKILL_NONE: u32 = u32::MAX;
pub const BYTE_NONE: u8 = u8::MAX;
pub const EXPERIENCE_RAW_NONE: i32 = i32::MIN;
pub const MAX_STRING_BYTES: usize = 160;
pub const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_CHARACTER_ID: u32 = 2_000_000;
pub const MAX_UNIT_LEADER_ID: u32 = i32::MAX as u32;
pub const MAX_TRAITS_PER_LEADER: u16 = 64;
pub const MAX_SKILL: u32 = 15;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const SCHEMA_ROOTS_VALIDATED: u32 = 1;
pub const SCHEMA_STATIC_NAME_CLOSURE: u32 = 1 << 1;
pub const SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS: u32 = 1 << 2;

pub const ROW_CHARACTER: u8 = 1;
pub const ROW_UNIT_LEADER: u8 = 2;
pub const ROW_UNIT_LEADER_TRAIT: u8 = 3;
pub const ROW_COUNTRY_LEADER_ROLE: u8 = 4;
pub const ROW_ACTIVE_RULER: u8 = 5;
pub const ROW_ADVISOR_ROSTER: u8 = 6;
pub const ROW_APPOINTED_ADVISOR: u8 = 7;
pub const ROW_CHARACTER_PORTRAIT: u8 = 8;

pub const UNIT_LEADER_ARMY: u8 = 1;
pub const UNIT_LEADER_NAVY: u8 = 2;

pub const ROW_FLAG_AMBIGUOUS: u16 = 1;
pub const ROW_FLAG_SKILL_PRESENT: u16 = 1 << 1;
pub const ROW_FLAG_EXPERIENCE_PRESENT: u16 = 1 << 2;

const CHARACTER_ROW_LEN: u8 = 20;
const UNIT_LEADER_ROW_LEN: u8 = 42;
const TRAIT_ROW_LEN: u8 = 16;
const COUNTRY_LEADER_ROLE_ROW_LEN: u8 = 20;
const ACTIVE_RULER_ROW_LEN: u8 = 28;
const ADVISOR_ROW_LEN: u8 = 24;
const CHARACTER_PORTRAIT_ROW_LEN: u8 = 24;
/// The widest row `encode_payload` can emit, used ONLY to size the row buffer
/// up front. Asserted against every row-length constant by
/// `max_row_len_covers_every_row_kind`, so adding a wider row kind without
/// updating this is a test failure rather than a silent re-introduction of the
/// reallocation it exists to avoid.
const MAX_ROW_LEN: u8 = UNIT_LEADER_ROW_LEN;
const ADVISOR_CABINET_BUCKETS: &[&[u8]] = &[
    b"political_advisor",
    b"army_chief",
    b"navy_chief",
    b"air_chief",
    b"high_command",
    b"theorist",
    b"unknown",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderRow<'a> {
    pub country_tag_db_id: u32,
    pub unit_leader_id: u32,
    pub char_id: u32,
    pub leader_kind: u8,
    pub skill: Option<u32>,
    pub experience_raw: Option<i32>,
    pub attack: Option<u8>,
    pub defense: Option<u8>,
    pub planning: Option<u8>,
    pub logistics: Option<u8>,
    pub maneuvering: Option<u8>,
    pub coordination: Option<u8>,
    pub token: Option<&'a str>,
    pub name: Option<&'a str>,
    pub trait_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderTraitRow<'a> {
    pub unit_leader_id: u32,
    pub ordinal: u16,
    pub trait_token: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeaderRoleRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRulerRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub ruling_party: Option<&'a str>,
    pub name: Option<&'a str>,
    pub token: Option<&'a str>,
    pub leader_vec_size: u16,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub slot: Option<&'a str>,
    pub ledger: Option<&'a str>,
    pub idea_token: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPortraitRow<'a> {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub role: &'a str,
    pub size: &'a str,
    pub gfx: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterLaneRow<'a> {
    Character(CharacterRow<'a>),
    UnitLeader(UnitLeaderRow<'a>),
    UnitLeaderTrait(UnitLeaderTraitRow<'a>),
    CountryLeaderRole(CountryLeaderRoleRow<'a>),
    ActiveRuler(ActiveRulerRow<'a>),
    AdvisorRoster(AdvisorRow<'a>),
    AppointedAdvisor(AdvisorRow<'a>),
    CharacterPortrait(CharacterPortraitRow<'a>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain44GateSummary {
    pub status_flags_or: u32,
    pub schema_flags_or: u32,
    pub row_count_total: u64,
    pub character_rows: u64,
    pub unit_leader_rows: u64,
    pub trait_rows: u64,
    pub country_leader_role_rows: u64,
    pub active_ruler_rows: u64,
    pub advisor_roster_rows: u64,
    pub appointed_advisor_rows: u64,
    pub character_portrait_rows: u64,
    pub string_count: u64,
    pub string_bytes: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDomain47Payload {
    pub status_flags: u32,
    pub schema_flags: u32,
    pub rows: Vec<DecodedDomain47Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodedDomain47Row {
    Character {
        country_tag_db_id: u32,
        char_id: u32,
        name: Option<String>,
        token: Option<String>,
    },
    UnitLeader {
        country_tag_db_id: u32,
        unit_leader_id: u32,
        char_id: u32,
        leader_kind: u8,
        skill: Option<u32>,
        experience_raw: Option<i32>,
        attack: Option<u8>,
        defense: Option<u8>,
        planning: Option<u8>,
        logistics: Option<u8>,
        maneuvering: Option<u8>,
        coordination: Option<u8>,
        token: Option<String>,
        name: Option<String>,
    },
    UnitLeaderTrait {
        unit_leader_id: u32,
        ordinal: u16,
        trait_token: String,
    },
    CountryLeaderRole {
        country_tag_db_id: u32,
        char_id: u32,
        name: Option<String>,
        token: Option<String>,
    },
    ActiveRuler {
        country_tag_db_id: u32,
        char_id: u32,
        ruling_party: Option<String>,
        name: Option<String>,
        token: Option<String>,
        leader_vec_size: u16,
        ambiguous: bool,
    },
    AdvisorRoster {
        country_tag_db_id: u32,
        char_id: u32,
        slot: Option<String>,
        ledger: Option<String>,
        idea_token: String,
    },
    AppointedAdvisor {
        country_tag_db_id: u32,
        char_id: u32,
        slot: Option<String>,
        ledger: Option<String>,
        idea_token: String,
    },
    CharacterPortrait {
        country_tag_db_id: u32,
        char_id: u32,
        role: String,
        size: String,
        gfx: String,
    },
}

/// Interning side table for the payload's string block.
///
/// `values` IS the encoded block: `encode_into` walks it in order and the ids
/// handed back are indices into it, so its contents and its order are wire
/// state. `by_value` is a pure lookup accelerator over the same strings and
/// is never iterated — nothing about the emitted bytes can depend on how it
/// is keyed or ordered.
///
/// WHY IT IS SHAPED LIKE THIS (lane DB, 2026-08-06). Domain 47 emits ~28k
/// rows and ~25k distinct strings per frame, and `intern` is called on nearly
/// every row field, so this is the encoder's whole hot loop. The offline cost
/// bench (`d47_cost_read_bracket_observation_vs_compute`) measured
/// `encode_frame_payload` at 6.87 ms against 1.85 ms for the engine walk it
/// serialises — the encoder cost 3.7x as much as reading the game. Three
/// things in the previous shape caused that, and each is fixed here WITHOUT
/// changing a single emitted byte:
///
///  * The map was a `BTreeMap<String, _>`, so every lookup was O(log n) FULL
///    STRING COMPARISONS — ~15 memcmps against a 25k-entry table, ~28k times
///    a frame. A hash map answers the same question in one.
///  * Every distinct string was allocated TWICE, once for `values` and once
///    for the map key. `Rc<str>` stores it once and the map holds a second
///    handle to the same bytes.
///  * `is_domain47_string_value` — which DECODES the whole string as UTF-8
///    chars — ran ahead of the lookup, so it re-validated every repeat
///    occurrence of an already-interned string.
///
/// The validation move is the only one that changes WHEN work happens rather
/// than how fast it is, so it is worth stating why it is not a relaxation: the
/// checks are pure functions of the string's bytes, and an id is only ever
/// returned for a value that passed them the first time it was seen. A value
/// that would fail still fails, on its first occurrence, with the same panic —
/// there is no input for which the old shape rejects and this one accepts.
#[derive(Default)]
struct StringTable {
    values: Vec<Rc<str>>,
    by_value: HashMap<Rc<str>, u32>,
}

impl StringTable {
    fn intern(&mut self, value: Option<&str>) -> u32 {
        let Some(value) = value else {
            return STRING_NONE;
        };
        // Already interned => already validated, by construction: the only
        // path that inserts an id runs both asserts below first.
        if let Some(id) = self.by_value.get(value) {
            return *id;
        }
        assert!(
            value.len() <= MAX_STRING_BYTES,
            "character lane string exceeds max length"
        );
        assert!(
            is_domain47_string_value(value),
            "character lane string must be valid non-control UTF-8"
        );
        let id = self.values.len().min(u32::MAX as usize) as u32;
        // ONE allocation, two handles: `values` owns the encode order and the
        // map owns the lookup, over the same bytes.
        let shared: Rc<str> = Rc::from(value);
        self.values.push(Rc::clone(&shared));
        self.by_value.insert(shared, id);
        id
    }

    fn encoded_len(&self) -> usize {
        self.values.iter().map(|value| 2 + value.len()).sum()
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        for value in &self.values {
            out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            out.extend_from_slice(value.as_bytes());
        }
    }
}

fn is_domain47_string_value(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

#[derive(Default)]
struct RowCounts {
    total: u32,
    characters: u32,
    unit_leaders: u32,
    traits: u32,
    country_leader_roles: u32,
    active_rulers: u32,
    advisor_roster: u32,
    appointed_advisors: u32,
    character_portraits: u32,
}

pub fn encode_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    schema_flags: u32,
    rows: &[CharacterLaneRow<'_>],
) -> Vec<u8> {
    let mut strings = StringTable::default();
    // ONE allocation for the row block instead of ~20 doubling reallocations,
    // each of which memcpy'd everything written so far. `MAX_ROW_LEN` is the
    // widest row this encoder can emit, so the reservation is an upper bound
    // and the buffer is never grown again; it is transient and dropped at the
    // end of this call. Purely an allocation decision — a capacity hint cannot
    // change a byte of what is written into it.
    let mut row_bytes = Vec::with_capacity(rows.len().saturating_mul(MAX_ROW_LEN as usize));
    let mut counts = RowCounts::default();

    for row in rows {
        counts.total = counts.total.saturating_add(1);
        match row {
            CharacterLaneRow::Character(row) => {
                counts.characters = counts.characters.saturating_add(1);
                push_header(&mut row_bytes, ROW_CHARACTER, CHARACTER_ROW_LEN, 0);
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
            }
            CharacterLaneRow::UnitLeader(row) => {
                counts.unit_leaders = counts.unit_leaders.saturating_add(1);
                let mut flags = 0u16;
                if row.skill.is_some() {
                    flags |= ROW_FLAG_SKILL_PRESENT;
                }
                if row.experience_raw.is_some() {
                    flags |= ROW_FLAG_EXPERIENCE_PRESENT;
                }
                push_header(&mut row_bytes, ROW_UNIT_LEADER, UNIT_LEADER_ROW_LEN, flags);
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.unit_leader_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, row.skill.unwrap_or(SKILL_NONE));
                push_i32(
                    &mut row_bytes,
                    row.experience_raw.unwrap_or(EXPERIENCE_RAW_NONE),
                );
                push_u32(&mut row_bytes, strings.intern(row.token));
                push_u32(&mut row_bytes, strings.intern(row.name));
                row_bytes.extend_from_slice(&row.trait_count.to_le_bytes());
                row_bytes.push(row.attack.unwrap_or(BYTE_NONE));
                row_bytes.push(row.defense.unwrap_or(BYTE_NONE));
                row_bytes.push(row.planning.unwrap_or(BYTE_NONE));
                row_bytes.push(row.logistics.unwrap_or(BYTE_NONE));
                row_bytes.push(row.maneuvering.unwrap_or(BYTE_NONE));
                row_bytes.push(row.coordination.unwrap_or(BYTE_NONE));
                row_bytes.push(row.leader_kind);
                row_bytes.push(0);
            }
            CharacterLaneRow::UnitLeaderTrait(row) => {
                counts.traits = counts.traits.saturating_add(1);
                push_header(&mut row_bytes, ROW_UNIT_LEADER_TRAIT, TRAIT_ROW_LEN, 0);
                push_u32(&mut row_bytes, row.unit_leader_id);
                row_bytes.extend_from_slice(&row.ordinal.to_le_bytes());
                row_bytes.extend_from_slice(&0u16.to_le_bytes());
                push_u32(&mut row_bytes, strings.intern(Some(row.trait_token)));
            }
            CharacterLaneRow::CountryLeaderRole(row) => {
                counts.country_leader_roles = counts.country_leader_roles.saturating_add(1);
                push_header(
                    &mut row_bytes,
                    ROW_COUNTRY_LEADER_ROLE,
                    COUNTRY_LEADER_ROLE_ROW_LEN,
                    0,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
            }
            CharacterLaneRow::ActiveRuler(row) => {
                counts.active_rulers = counts.active_rulers.saturating_add(1);
                let flags = if row.ambiguous { ROW_FLAG_AMBIGUOUS } else { 0 };
                push_header(
                    &mut row_bytes,
                    ROW_ACTIVE_RULER,
                    ACTIVE_RULER_ROW_LEN,
                    flags,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(row.ruling_party));
                push_u32(&mut row_bytes, strings.intern(row.name));
                push_u32(&mut row_bytes, strings.intern(row.token));
                row_bytes.extend_from_slice(&row.leader_vec_size.to_le_bytes());
                row_bytes.extend_from_slice(&0u16.to_le_bytes());
            }
            CharacterLaneRow::AdvisorRoster(row) => {
                counts.advisor_roster = counts.advisor_roster.saturating_add(1);
                encode_advisor_row(&mut row_bytes, &mut strings, ROW_ADVISOR_ROSTER, row);
            }
            CharacterLaneRow::AppointedAdvisor(row) => {
                counts.appointed_advisors = counts.appointed_advisors.saturating_add(1);
                encode_advisor_row(&mut row_bytes, &mut strings, ROW_APPOINTED_ADVISOR, row);
            }
            CharacterLaneRow::CharacterPortrait(row) => {
                counts.character_portraits = counts.character_portraits.saturating_add(1);
                push_header(
                    &mut row_bytes,
                    ROW_CHARACTER_PORTRAIT,
                    CHARACTER_PORTRAIT_ROW_LEN,
                    0,
                );
                push_u32(&mut row_bytes, row.country_tag_db_id);
                push_u32(&mut row_bytes, row.char_id);
                push_u32(&mut row_bytes, strings.intern(Some(row.role)));
                push_u32(&mut row_bytes, strings.intern(Some(row.size)));
                push_u32(&mut row_bytes, strings.intern(Some(row.gfx)));
            }
        }
    }

    let mut out = Vec::with_capacity(HEADER_LEN + strings.encoded_len() + row_bytes.len());
    push_u64(&mut out, elapsed_wall_ms);
    push_u32(&mut out, status_flags);
    push_u32(&mut out, counts.total);
    push_u32(&mut out, strings.values.len().min(u32::MAX as usize) as u32);
    push_u32(
        &mut out,
        strings.encoded_len().min(u32::MAX as usize) as u32,
    );
    push_u32(&mut out, counts.characters);
    push_u32(&mut out, counts.unit_leaders);
    push_u32(&mut out, counts.country_leader_roles);
    push_u32(&mut out, counts.active_rulers);
    push_u32(&mut out, counts.advisor_roster);
    push_u32(&mut out, counts.appointed_advisors);
    push_u32(&mut out, counts.traits);
    push_u32(&mut out, row_bytes.len().min(u32::MAX as usize) as u32);
    push_u32(&mut out, schema_flags);
    push_u32(&mut out, 0);
    strings.encode_into(&mut out);
    out.extend_from_slice(&row_bytes);
    out
}

pub fn encode_compact_data_record(sequence: u64, game_hour_raw: i32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID.to_le_bytes());
    out.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain44GateSummary {
    let mut summary = Domain44GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: character lane payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let status_flags = read_u32(payload, 8);
    let expected_total = read_u32(payload, 12) as u64;
    let string_count = read_u32(payload, 16) as usize;
    let string_bytes = read_u32(payload, 20) as usize;
    let expected_characters = read_u32(payload, 24) as u64;
    let expected_unit_leaders = read_u32(payload, 28) as u64;
    let expected_country_leader_roles = read_u32(payload, 32) as u64;
    let expected_active_rulers = read_u32(payload, 36) as u64;
    let expected_advisor_roster = read_u32(payload, 40) as u64;
    let expected_appointed_advisors = read_u32(payload, 44) as u64;
    let expected_traits = read_u32(payload, 48) as u64;
    let row_bytes = read_u32(payload, 52) as usize;
    let schema_flags = read_u32(payload, 56);
    let reserved0 = read_u32(payload, 60);
    summary.status_flags_or |= status_flags;
    summary.schema_flags_or |= schema_flags;

    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: character lane reserved header field is nonzero"
        ));
    }

    let mut cursor = HEADER_LEN;
    let string_start = cursor;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        if cursor + 2 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} has truncated length"
            ));
            return summary;
        }
        let len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;
        if len > MAX_STRING_BYTES {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} length {len} exceeds {MAX_STRING_BYTES}"
            ));
        }
        if cursor + len > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} is truncated"
            ));
            return summary;
        }
        let raw = &payload[cursor..cursor + len];
        match std::str::from_utf8(raw) {
            Ok(value) if is_domain47_string_value(value) => {}
            Ok(_) => summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} contains control characters"
            )),
            Err(_) => summary.parse_errors.push(format!(
                "offset {offset}: string table entry {index} is invalid UTF-8"
            )),
        }
        strings.push(raw);
        cursor += len;
    }
    let measured_string_bytes = cursor.saturating_sub(string_start);
    summary.string_count = string_count as u64;
    summary.string_bytes = measured_string_bytes as u64;
    if measured_string_bytes != string_bytes {
        summary.parse_errors.push(format!(
            "offset {offset}: string_bytes header {string_bytes} != decoded {measured_string_bytes}"
        ));
    }
    if cursor + row_bytes != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: row_bytes header {row_bytes} does not match remaining {}",
            payload.len().saturating_sub(cursor)
        ));
        if cursor + row_bytes > payload.len() {
            return summary;
        }
    }

    let rows_end = cursor.saturating_add(row_bytes).min(payload.len());
    let mut row_index = 0usize;
    let mut seen_characters = BTreeSet::new();
    let mut seen_unit_leaders = BTreeSet::new();
    let mut seen_country_roles = BTreeSet::new();
    let mut seen_active_rulers = BTreeSet::new();
    let mut seen_advisor_rows = BTreeSet::new();
    let mut seen_portrait_rows = BTreeSet::new();
    let mut declared_trait_counts = BTreeMap::<u32, u16>::new();
    let mut observed_trait_counts = BTreeMap::<u32, u16>::new();

    while cursor < rows_end {
        if cursor + 4 > rows_end {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has truncated row header"
            ));
            break;
        }
        let row_offset = cursor;
        let kind = payload[cursor];
        let row_len = payload[cursor + 1] as usize;
        let flags = read_u16(payload, cursor + 2);
        if row_len < 4 || cursor + row_len > rows_end {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} kind {kind} has invalid length {row_len}"
            ));
            break;
        }
        summary.row_count_total = summary.row_count_total.saturating_add(1);
        match kind {
            ROW_CHARACTER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    CHARACTER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let name_sid = read_u32(payload, cursor + 12);
                let token_sid = read_u32(payload, cursor + 16);
                summary.character_rows = summary.character_rows.saturating_add(1);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                if !seen_characters.insert(char_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates character id {char_id}"
                    ));
                }
            }
            ROW_UNIT_LEADER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    UNIT_LEADER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let unit_id = read_u32(payload, cursor + 8);
                let char_id = read_u32(payload, cursor + 12);
                let skill = read_u32(payload, cursor + 16);
                let experience_raw = read_i32(payload, cursor + 20);
                let token_sid = read_u32(payload, cursor + 24);
                let name_sid = read_u32(payload, cursor + 28);
                let trait_count = read_u16(payload, cursor + 32);
                let leader_kind = payload[cursor + 40];
                summary.unit_leader_rows = summary.unit_leader_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_unit_leader(&mut summary, offset, row_index, unit_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                if skill != SKILL_NONE && skill > MAX_SKILL {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid unit-leader skill {skill}"
                    ));
                }
                if (flags & ROW_FLAG_SKILL_PRESENT != 0) != (skill != SKILL_NONE) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} skill-present flag disagrees with value"
                    ));
                }
                if (flags & ROW_FLAG_EXPERIENCE_PRESENT != 0)
                    != (experience_raw != EXPERIENCE_RAW_NONE)
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} experience-present flag disagrees with value"
                    ));
                }
                if trait_count > MAX_TRAITS_PER_LEADER {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} declares {trait_count} traits"
                    ));
                }
                if leader_kind != UNIT_LEADER_ARMY && leader_kind != UNIT_LEADER_NAVY {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid leader kind {leader_kind}"
                    ));
                }
                if !seen_unit_leaders.insert(unit_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates unit leader id {unit_id}"
                    ));
                }
                declared_trait_counts.insert(unit_id, trait_count);
            }
            ROW_UNIT_LEADER_TRAIT => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    TRAIT_ROW_LEN,
                );
                let unit_id = read_u32(payload, cursor + 4);
                let ordinal = read_u16(payload, cursor + 8);
                let trait_sid = read_u32(payload, cursor + 12);
                summary.trait_rows = summary.trait_rows.saturating_add(1);
                validate_unit_leader(&mut summary, offset, row_index, unit_id);
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "trait_token",
                    trait_sid,
                    strings.len(),
                );
                if ordinal >= MAX_TRAITS_PER_LEADER {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} has invalid trait ordinal {ordinal}"
                    ));
                }
                let entry = observed_trait_counts.entry(unit_id).or_default();
                *entry = entry.saturating_add(1);
            }
            ROW_COUNTRY_LEADER_ROLE => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    COUNTRY_LEADER_ROLE_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let name_sid = read_u32(payload, cursor + 12);
                let token_sid = read_u32(payload, cursor + 16);
                summary.country_leader_role_rows =
                    summary.country_leader_role_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                if !seen_country_roles.insert(char_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates country-leader role char id {char_id}"
                    ));
                }
            }
            ROW_ACTIVE_RULER => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    ACTIVE_RULER_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let party_sid = read_u32(payload, cursor + 12);
                let name_sid = read_u32(payload, cursor + 16);
                let token_sid = read_u32(payload, cursor + 20);
                let vec_size = read_u16(payload, cursor + 24);
                summary.active_ruler_rows = summary.active_ruler_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, true);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "ruling_party",
                    party_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "name",
                    name_sid,
                    strings.len(),
                );
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "token",
                    token_sid,
                    strings.len(),
                );
                if vec_size == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} active-ruler leader_vec_size is zero"
                    ));
                }
                if (flags & ROW_FLAG_AMBIGUOUS != 0) != (vec_size != 1) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} ambiguous flag disagrees with leader_vec_size {vec_size}"
                    ));
                }
                if !seen_active_rulers.insert(country) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates active ruler for country tag id {country}"
                    ));
                }
            }
            ROW_ADVISOR_ROSTER | ROW_APPOINTED_ADVISOR => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    ADVISOR_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let slot_sid = read_u32(payload, cursor + 12);
                let ledger_sid = read_u32(payload, cursor + 16);
                let idea_sid = read_u32(payload, cursor + 20);
                validate_country(&mut summary, offset, row_index, country, true);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_optional_string(
                    &mut summary,
                    offset,
                    row_index,
                    "slot",
                    slot_sid,
                    strings.len(),
                );
                validate_required_string_value(
                    &mut summary,
                    offset,
                    row_index,
                    "cabinet_bucket",
                    ledger_sid,
                    &strings,
                    ADVISOR_CABINET_BUCKETS,
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "idea_token",
                    idea_sid,
                    strings.len(),
                );
                if kind == ROW_ADVISOR_ROSTER {
                    summary.advisor_roster_rows = summary.advisor_roster_rows.saturating_add(1);
                } else {
                    summary.appointed_advisor_rows =
                        summary.appointed_advisor_rows.saturating_add(1);
                }
                if !seen_advisor_rows.insert((kind, country, char_id, idea_sid)) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates advisor row"
                    ));
                }
            }
            ROW_CHARACTER_PORTRAIT => {
                require_len(
                    &mut summary,
                    offset,
                    row_index,
                    kind,
                    row_len,
                    CHARACTER_PORTRAIT_ROW_LEN,
                );
                let country = read_u32(payload, cursor + 4);
                let char_id = read_u32(payload, cursor + 8);
                let role_sid = read_u32(payload, cursor + 12);
                let size_sid = read_u32(payload, cursor + 16);
                let gfx_sid = read_u32(payload, cursor + 20);
                summary.character_portrait_rows = summary.character_portrait_rows.saturating_add(1);
                validate_country(&mut summary, offset, row_index, country, false);
                validate_character(&mut summary, offset, row_index, char_id);
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_role",
                    role_sid,
                    strings.len(),
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_size",
                    size_sid,
                    strings.len(),
                );
                validate_required_string(
                    &mut summary,
                    offset,
                    row_index,
                    "portrait_gfx",
                    gfx_sid,
                    strings.len(),
                );
                if !seen_portrait_rows.insert((country, char_id, role_sid, size_sid, gfx_sid)) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: row {row_index} duplicates character portrait row"
                    ));
                }
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has unknown row kind {kind}"
            )),
        }
        if row_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has zero length at stream offset {row_offset}"
            ));
            break;
        }
        cursor += row_len;
        row_index += 1;
    }

    let observed_total = summary.row_count_total;
    let observed_characters = summary.character_rows;
    let observed_unit_leaders = summary.unit_leader_rows;
    let observed_country_leader_roles = summary.country_leader_role_rows;
    let observed_active_rulers = summary.active_ruler_rows;
    let observed_advisor_roster = summary.advisor_roster_rows;
    let observed_appointed_advisors = summary.appointed_advisor_rows;
    let observed_traits = summary.trait_rows;

    compare_count(
        &mut summary,
        offset,
        "row_count_total",
        expected_total,
        observed_total,
    );
    compare_count(
        &mut summary,
        offset,
        "character_rows",
        expected_characters,
        observed_characters,
    );
    compare_count(
        &mut summary,
        offset,
        "unit_leader_rows",
        expected_unit_leaders,
        observed_unit_leaders,
    );
    compare_count(
        &mut summary,
        offset,
        "country_leader_role_rows",
        expected_country_leader_roles,
        observed_country_leader_roles,
    );
    compare_count(
        &mut summary,
        offset,
        "active_ruler_rows",
        expected_active_rulers,
        observed_active_rulers,
    );
    compare_count(
        &mut summary,
        offset,
        "advisor_roster_rows",
        expected_advisor_roster,
        observed_advisor_roster,
    );
    compare_count(
        &mut summary,
        offset,
        "appointed_advisor_rows",
        expected_appointed_advisors,
        observed_appointed_advisors,
    );
    compare_count(
        &mut summary,
        offset,
        "trait_rows",
        expected_traits,
        observed_traits,
    );

    for (unit_id, expected) in declared_trait_counts {
        let observed = observed_trait_counts.get(&unit_id).copied().unwrap_or(0);
        if observed != expected {
            summary.parse_errors.push(format!(
                "offset {offset}: unit leader {unit_id} declared {expected} traits but decoded {observed}"
            ));
        }
    }
    for unit_id in observed_trait_counts.keys() {
        if !seen_unit_leaders.contains(unit_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: trait rows reference unit leader {unit_id} without a unit-leader row"
            ));
        }
    }

    add_unit(&mut summary, "character_rows", observed_characters);
    add_unit(&mut summary, "unit_leader_rows", observed_unit_leaders);
    add_unit(&mut summary, "unit_leader_trait_rows", observed_traits);
    add_unit(
        &mut summary,
        "country_leader_role_rows",
        observed_country_leader_roles,
    );
    add_unit(&mut summary, "active_ruler_rows", observed_active_rulers);
    add_unit(&mut summary, "advisor_roster_rows", observed_advisor_roster);
    add_unit(
        &mut summary,
        "appointed_advisor_rows",
        observed_appointed_advisors,
    );
    let character_portrait_rows = summary.character_portrait_rows;
    add_unit(
        &mut summary,
        "character_portrait_rows",
        character_portrait_rows,
    );
    summary
}

pub fn decode_payload_rows(
    offset: usize,
    payload: &[u8],
) -> Result<DecodedDomain47Payload, Vec<String>> {
    let summary = decode_payload(offset, payload);
    if !summary.parse_errors.is_empty() {
        return Err(summary.parse_errors);
    }
    decode_payload_rows_checked(payload).map_err(|err| vec![format!("offset {offset}: {err}")])
}

fn decode_payload_rows_checked(payload: &[u8]) -> Result<DecodedDomain47Payload, String> {
    need_payload(payload, HEADER_LEN, "domain47 header")?;
    let status_flags = read_u32(payload, 8);
    let expected_total = read_u32(payload, 12) as usize;
    let string_count = read_u32(payload, 16) as usize;
    let string_bytes = read_u32(payload, 20) as usize;
    let row_bytes = read_u32(payload, 52) as usize;
    let schema_flags = read_u32(payload, 56);

    let mut cursor = HEADER_LEN;
    let string_end = cursor
        .checked_add(string_bytes)
        .ok_or("domain47 string table length overflow")?;
    need_payload(payload, string_end, "domain47 string table")?;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        need_payload(payload, cursor + 2, "domain47 string length")?;
        let len = read_u16(payload, cursor) as usize;
        cursor += 2;
        if len > MAX_STRING_BYTES {
            return Err(format!(
                "domain47 string {index} exceeds {MAX_STRING_BYTES} bytes"
            ));
        }
        need_payload(payload, cursor + len, "domain47 string bytes")?;
        let raw = &payload[cursor..cursor + len];
        let value = std::str::from_utf8(raw)
            .map_err(|err| format!("domain47 string {index} invalid UTF-8: {err}"))?;
        if !is_domain47_string_value(value) {
            return Err(format!(
                "domain47 string {index} contains control characters"
            ));
        }
        strings.push(value.to_string());
        cursor += len;
    }
    if cursor != string_end {
        return Err("domain47 decoded string table length mismatch".into());
    }

    let rows_end = cursor
        .checked_add(row_bytes)
        .ok_or("domain47 row bytes overflow")?;
    if rows_end != payload.len() {
        return Err(format!(
            "domain47 row bytes {row_bytes} do not match payload remainder {}",
            payload.len().saturating_sub(cursor)
        ));
    }

    let mut rows = Vec::with_capacity(expected_total);
    while cursor < rows_end {
        need_payload(payload, cursor + 4, "domain47 row header")?;
        let kind = payload[cursor];
        let row_len = payload[cursor + 1] as usize;
        let flags = read_u16(payload, cursor + 2);
        if row_len < 4 {
            return Err(format!(
                "domain47 row kind {kind} has invalid length {row_len}"
            ));
        }
        need_payload(payload, cursor + row_len, "domain47 row body")?;
        match kind {
            ROW_CHARACTER => {
                require_decode_len(kind, row_len, CHARACTER_ROW_LEN)?;
                rows.push(DecodedDomain47Row::Character {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                });
            }
            ROW_UNIT_LEADER => {
                require_decode_len(kind, row_len, UNIT_LEADER_ROW_LEN)?;
                let skill_raw = read_u32(payload, cursor + 16);
                let experience_raw = read_i32(payload, cursor + 20);
                rows.push(DecodedDomain47Row::UnitLeader {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    unit_leader_id: read_u32(payload, cursor + 8),
                    char_id: read_u32(payload, cursor + 12),
                    leader_kind: payload[cursor + 40],
                    skill: if (flags & ROW_FLAG_SKILL_PRESENT) != 0 && skill_raw != SKILL_NONE {
                        Some(skill_raw)
                    } else {
                        None
                    },
                    experience_raw: if (flags & ROW_FLAG_EXPERIENCE_PRESENT) != 0
                        && experience_raw != EXPERIENCE_RAW_NONE
                    {
                        Some(experience_raw)
                    } else {
                        None
                    },
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 24))?,
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 28))?,
                    attack: decoded_optional_byte(payload[cursor + 34]),
                    defense: decoded_optional_byte(payload[cursor + 35]),
                    planning: decoded_optional_byte(payload[cursor + 36]),
                    logistics: decoded_optional_byte(payload[cursor + 37]),
                    maneuvering: decoded_optional_byte(payload[cursor + 38]),
                    coordination: decoded_optional_byte(payload[cursor + 39]),
                });
            }
            ROW_UNIT_LEADER_TRAIT => {
                require_decode_len(kind, row_len, TRAIT_ROW_LEN)?;
                rows.push(DecodedDomain47Row::UnitLeaderTrait {
                    unit_leader_id: read_u32(payload, cursor + 4),
                    ordinal: read_u16(payload, cursor + 8),
                    trait_token: decoded_required_string(&strings, read_u32(payload, cursor + 12))?,
                });
            }
            ROW_COUNTRY_LEADER_ROLE => {
                require_decode_len(kind, row_len, COUNTRY_LEADER_ROLE_ROW_LEN)?;
                rows.push(DecodedDomain47Row::CountryLeaderRole {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                });
            }
            ROW_ACTIVE_RULER => {
                require_decode_len(kind, row_len, ACTIVE_RULER_ROW_LEN)?;
                rows.push(DecodedDomain47Row::ActiveRuler {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    ruling_party: decoded_optional_string(
                        &strings,
                        read_u32(payload, cursor + 12),
                    )?,
                    name: decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                    token: decoded_optional_string(&strings, read_u32(payload, cursor + 20))?,
                    leader_vec_size: read_u16(payload, cursor + 24),
                    ambiguous: (flags & ROW_FLAG_AMBIGUOUS) != 0,
                });
            }
            ROW_ADVISOR_ROSTER | ROW_APPOINTED_ADVISOR => {
                require_decode_len(kind, row_len, ADVISOR_ROW_LEN)?;
                let row = (
                    read_u32(payload, cursor + 4),
                    read_u32(payload, cursor + 8),
                    decoded_optional_string(&strings, read_u32(payload, cursor + 12))?,
                    decoded_optional_string(&strings, read_u32(payload, cursor + 16))?,
                    decoded_required_string(&strings, read_u32(payload, cursor + 20))?,
                );
                if kind == ROW_ADVISOR_ROSTER {
                    rows.push(DecodedDomain47Row::AdvisorRoster {
                        country_tag_db_id: row.0,
                        char_id: row.1,
                        slot: row.2,
                        ledger: row.3,
                        idea_token: row.4,
                    });
                } else {
                    rows.push(DecodedDomain47Row::AppointedAdvisor {
                        country_tag_db_id: row.0,
                        char_id: row.1,
                        slot: row.2,
                        ledger: row.3,
                        idea_token: row.4,
                    });
                }
            }
            ROW_CHARACTER_PORTRAIT => {
                require_decode_len(kind, row_len, CHARACTER_PORTRAIT_ROW_LEN)?;
                rows.push(DecodedDomain47Row::CharacterPortrait {
                    country_tag_db_id: read_u32(payload, cursor + 4),
                    char_id: read_u32(payload, cursor + 8),
                    role: decoded_required_string(&strings, read_u32(payload, cursor + 12))?,
                    size: decoded_required_string(&strings, read_u32(payload, cursor + 16))?,
                    gfx: decoded_required_string(&strings, read_u32(payload, cursor + 20))?,
                });
            }
            _ => return Err(format!("domain47 unknown row kind {kind}")),
        }
        cursor += row_len;
    }

    Ok(DecodedDomain47Payload {
        status_flags,
        schema_flags,
        rows,
    })
}

fn encode_advisor_row(
    row_bytes: &mut Vec<u8>,
    strings: &mut StringTable,
    kind: u8,
    row: &AdvisorRow<'_>,
) {
    push_header(row_bytes, kind, ADVISOR_ROW_LEN, 0);
    push_u32(row_bytes, row.country_tag_db_id);
    push_u32(row_bytes, row.char_id);
    push_u32(row_bytes, strings.intern(row.slot));
    push_u32(row_bytes, strings.intern(row.ledger));
    push_u32(row_bytes, strings.intern(Some(row.idea_token)));
}

fn push_header(out: &mut Vec<u8>, kind: u8, row_len: u8, flags: u16) {
    out.push(kind);
    out.push(row_len);
    out.extend_from_slice(&flags.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

fn require_len(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    kind: u8,
    got: usize,
    want: u8,
) {
    if got != want as usize {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} kind {kind} length {got} != {}",
            want
        ));
    }
}

fn validate_country(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    country: u32,
    nonzero: bool,
) {
    if (nonzero && country == 0) || country > MAX_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid country_tag_db_id {country}"
        ));
    }
}

fn validate_character(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    char_id: u32,
) {
    if char_id == 0 || char_id > MAX_CHARACTER_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid char_id {char_id}"
        ));
    }
}

fn validate_unit_leader(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    unit_id: u32,
) {
    if unit_id == 0 || unit_id > MAX_UNIT_LEADER_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} has invalid unit_leader_id {unit_id}"
        ));
    }
}

fn validate_optional_string(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    string_count: usize,
) {
    if string_id != STRING_NONE && string_id as usize >= string_count {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid string id {string_id}"
        ));
    }
}

fn validate_required_string(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    string_count: usize,
) {
    if string_id == STRING_NONE || string_id as usize >= string_count {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid required string id {string_id}"
        ));
    }
}

fn validate_required_string_value(
    summary: &mut Domain44GateSummary,
    offset: usize,
    row_index: usize,
    field: &str,
    string_id: u32,
    strings: &[&[u8]],
    allowed: &[&[u8]],
) {
    if string_id == STRING_NONE || string_id as usize >= strings.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid required string id {string_id}"
        ));
        return;
    }
    let value = strings[string_id as usize];
    if !allowed.iter().any(|allowed_value| *allowed_value == value) {
        let value = String::from_utf8_lossy(value);
        summary.parse_errors.push(format!(
            "offset {offset}: row {row_index} field {field} has invalid value {value}"
        ));
    }
}

fn compare_count(
    summary: &mut Domain44GateSummary,
    offset: usize,
    label: &'static str,
    expected: u64,
    observed: u64,
) {
    if expected != observed {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} header {expected} != decoded {observed}"
        ));
    }
}

fn add_unit(summary: &mut Domain44GateSummary, key: &'static str, value: u64) {
    if value == 0 {
        return;
    }
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value);
}

fn need_payload(payload: &[u8], at_least: usize, label: &str) -> Result<(), String> {
    if payload.len() < at_least {
        Err(format!(
            "{label}: need {at_least} bytes, have {}",
            payload.len()
        ))
    } else {
        Ok(())
    }
}

fn require_decode_len(kind: u8, got: usize, want: u8) -> Result<(), String> {
    if got != want as usize {
        Err(format!("domain47 row kind {kind} length {got} != {}", want))
    } else {
        Ok(())
    }
}

fn decoded_optional_byte(raw: u8) -> Option<u8> {
    (raw != BYTE_NONE).then_some(raw)
}

fn decoded_optional_string(strings: &[String], string_id: u32) -> Result<Option<String>, String> {
    if string_id == STRING_NONE {
        return Ok(None);
    }
    decoded_required_string(strings, string_id).map(Some)
}

fn decoded_required_string(strings: &[String], string_id: u32) -> Result<String, String> {
    strings
        .get(string_id as usize)
        .cloned()
        .ok_or_else(|| format!("domain47 invalid string id {string_id}"))
}

#[cfg(not(test))]
fn main() {
    if let Err(err) = run_cli() {
        eprintln!("{err}");
        std::process::exit(2);
    }
}

#[cfg(not(test))]
fn run_cli() -> Result<(), String> {
    let mut args = std::env::args();
    let program = args
        .next()
        .unwrap_or_else(|| "domain47_compact".to_string());
    let Some(path) = args.next() else {
        return Err(format!("usage: {program} <domain47_payload_or_record.bin>"));
    };
    if args.next().is_some() {
        return Err(format!("usage: {program} <domain47_payload_or_record.bin>"));
    }
    let bytes = std::fs::read(&path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let (payload, wrapped_record) = payload_from_input(&bytes)?;
    let summary = decode_payload(0, payload);
    println!("{}", summary_json(&summary, wrapped_record, payload.len()));
    if !summary.parse_errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(test))]
fn payload_from_input(bytes: &[u8]) -> Result<(&[u8], bool), String> {
    if bytes.starts_with(COMPACT_RECORD_MAGIC) {
        let header = parse_compact_record_header(bytes).map_err(|err| match err {
            RecordFramingError::TruncatedHeader(_) | RecordFramingError::BadMagic => {
                format!("invalid compact record header: {err}")
            }
            RecordFramingError::UnsupportedVersion(version) => {
                format!("unsupported compact record version {version}")
            }
            RecordFramingError::UnsupportedHeaderLen(header_len) => {
                format!("header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}")
            }
            RecordFramingError::PayloadTooLarge(_) => {
                format!("invalid compact record header: {err}")
            }
        })?;
        if header.domain_id != DOMAIN_ID {
            return Err(format!("domain_id {} != {DOMAIN_ID}", header.domain_id));
        }
        if header.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
            return Err(format!(
                "record_type {} != {COMPACT_RECORD_TYPE_DOMAIN_DATA}",
                header.record_type
            ));
        }
        if bytes.len() != header.record_len() {
            return Err(format!(
                "record payload length {} plus header {COMPACT_RECORD_HEADER_LEN} != file bytes {}",
                header.payload_len,
                bytes.len()
            ));
        }
        compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            &bytes[COMPACT_RECORD_HEADER_LEN..],
        )
        .map_err(|error| format!("invalid compact record semantics: {error}"))?;
        return Ok((&bytes[COMPACT_RECORD_HEADER_LEN..], true));
    }
    Ok((bytes, false))
}

#[cfg(not(test))]
fn summary_json(
    summary: &Domain44GateSummary,
    wrapped_record: bool,
    payload_bytes: usize,
) -> String {
    let mut units = String::new();
    units.push('{');
    for (idx, (key, value)) in summary.data_units_total.iter().enumerate() {
        if idx > 0 {
            units.push(',');
        }
        units.push('"');
        units.push_str(key);
        units.push_str("\":");
        units.push_str(&value.to_string());
    }
    units.push('}');
    format!(
        "{{\"domain\":\"{}\",\"domain_id\":{},\"wrapped_record\":{},\"payload_bytes\":{},\"status_flags_or\":{},\"schema_flags_or\":{},\"row_count_total\":{},\"character_rows\":{},\"unit_leader_rows\":{},\"unit_leader_trait_rows\":{},\"country_leader_role_rows\":{},\"active_ruler_rows\":{},\"advisor_roster_rows\":{},\"appointed_advisor_rows\":{},\"character_portrait_rows\":{},\"string_count\":{},\"string_bytes\":{},\"data_units_total\":{},\"parse_errors\":{}}}",
        json_escape(DOMAIN_NAME),
        DOMAIN_ID,
        wrapped_record,
        payload_bytes,
        summary.status_flags_or,
        summary.schema_flags_or,
        summary.row_count_total,
        summary.character_rows,
        summary.unit_leader_rows,
        summary.trait_rows,
        summary.country_leader_role_rows,
        summary.active_ruler_rows,
        summary.advisor_roster_rows,
        summary.appointed_advisor_rows,
        summary.character_portrait_rows,
        summary.string_count,
        summary.string_bytes,
        units,
        json_string_array(&summary.parse_errors),
    )
}

#[cfg(not(test))]
fn json_string_array(values: &[String]) -> String {
    let mut out = String::from("[");
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(value));
        out.push('"');
    }
    out.push(']');
    out
}

#[cfg(not(test))]
fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MAX_ROW_LEN` sizes the row buffer up front. If a row kind is ever
    /// wider than it, the reservation stops being an upper bound and the
    /// buffer silently starts reallocating again — the exact cost it was
    /// introduced to remove, with nothing observable to say so.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: widening any of the seven
    /// EXISTING row-length constants past `MAX_ROW_LEN` without updating it.
    ///
    /// WHAT IT DOES NOT CATCH, stated because the list below is maintained by
    /// hand (Codex review 019fd6bf-a0af-7ca2-a974-880b6832bba6, OPEN 2): a
    /// NEW row kind whose constant is never added here, and a row arm that
    /// writes more bytes than its own constant declares. Neither is a
    /// byte-correctness defect — a capacity that is too small only makes
    /// `Vec` reallocate, which is the cost this reservation exists to avoid,
    /// not a wrong payload — so this is a coverage gap in a performance
    /// guard, and it is named rather than papered over.
    #[test]
    fn max_row_len_covers_every_row_kind() {
        for (name, len) in [
            ("character", CHARACTER_ROW_LEN),
            ("unit_leader", UNIT_LEADER_ROW_LEN),
            ("trait", TRAIT_ROW_LEN),
            ("country_leader_role", COUNTRY_LEADER_ROLE_ROW_LEN),
            ("active_ruler", ACTIVE_RULER_ROW_LEN),
            ("advisor", ADVISOR_ROW_LEN),
            ("character_portrait", CHARACTER_PORTRAIT_ROW_LEN),
        ] {
            assert!(
                len <= MAX_ROW_LEN,
                "{name} row is {len} bytes, wider than MAX_ROW_LEN {MAX_ROW_LEN}"
            );
        }
    }

    /// The string table's IDs are indices into the encode-order block, so
    /// they must follow FIRST-USE order and repeats must return the id the
    /// first occurrence got. The lookup side is a hash map, which has no
    /// order at all — this pins the property the emitted bytes actually
    /// depend on, rather than the container that happens to provide it.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: handing out ids from the
    /// lookup map's iteration order (or sorting `values`) instead of
    /// appending to `values` — which is exactly the mistake available now
    /// that the map is unordered, and which the previous `BTreeMap` shape
    /// would have masked by being sorted anyway.
    #[test]
    fn string_ids_follow_first_use_order_and_repeats_reuse_them() {
        let mut table = StringTable::default();
        // Deliberately NOT in sorted order: under the old BTreeMap a test
        // that used sorted inputs could not tell insertion order from key
        // order.
        assert_eq!(table.intern(Some("zulu")), 0);
        assert_eq!(table.intern(Some("alpha")), 1);
        assert_eq!(table.intern(Some("mike")), 2);
        assert_eq!(table.intern(Some("alpha")), 1, "repeat must reuse its id");
        assert_eq!(table.intern(Some("zulu")), 0, "repeat must reuse its id");
        assert_eq!(table.intern(None), STRING_NONE);
        assert_eq!(
            table.values.iter().map(|v| &**v).collect::<Vec<_>>(),
            vec!["zulu", "alpha", "mike"],
            "the encoded block must be in first-use order"
        );

        // The block the decoder will read back, byte for byte.
        let mut out = Vec::new();
        table.encode_into(&mut out);
        assert_eq!(out.len(), table.encoded_len());
        assert_eq!(
            out,
            b"\x04\x00zulu\x05\x00alpha\x04\x00mike".to_vec(),
            "the encoded string block changed"
        );
    }

    /// The validity checks moved BEHIND the interned-value lookup, so this
    /// pins that a bad string is still refused on its FIRST occurrence —
    /// i.e. that the move traded no rejection away.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping either assert, or
    /// interning the value before checking it.
    #[test]
    #[should_panic(expected = "character lane string must be valid non-control UTF-8")]
    fn a_control_character_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        table.intern(Some("has a \u{7} bell"));
    }

    /// The same, for the length bound.
    #[test]
    #[should_panic(expected = "character lane string exceeds max length")]
    fn an_overlong_string_is_still_refused_on_first_sight() {
        let mut table = StringTable::default();
        let overlong = "x".repeat(MAX_STRING_BYTES + 1);
        table.intern(Some(&overlong));
    }

    #[test]
    fn row_len_literals_are_stable() {
        // Pin the per-kind row-length literals (stale-const guard).
        assert_eq!(CHARACTER_ROW_LEN, 20);
        assert_eq!(UNIT_LEADER_ROW_LEN, 42);
        assert_eq!(TRAIT_ROW_LEN, 16);
        assert_eq!(COUNTRY_LEADER_ROLE_ROW_LEN, 20);
        assert_eq!(ACTIVE_RULER_ROW_LEN, 28);
        assert_eq!(ADVISOR_ROW_LEN, 24);
        assert_eq!(CHARACTER_PORTRAIT_ROW_LEN, 24);
    }

    /// The end of the chain that run 20260731-deflist-16 broke at the start
    /// of. The collector census now ADMITS country_tag_db_id == 0 (the
    /// engine's constructor default, meaning "no country"), so the wire
    /// format has to keep accepting it -- and the collector-side tests
    /// cannot see this file. Flipping the character row's
    /// `validate_country(.., nonzero)` to true re-breaks zero-tag output
    /// while every collector test stays green.
    #[test]
    fn a_character_with_no_country_survives_encode_and_decode() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 0,
                char_id: 4711,
                name: Some("Unaffiliated Character"),
                token: Some("generic_unaffiliated"),
            }),
            // An unassigned leader is the same shape one level down, and is
            // permitted for the same reason.
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 0,
                unit_leader_id: 991,
                char_id: 4711,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(3),
                experience_raw: Some(0),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("generic_unaffiliated"),
                name: Some("Unaffiliated Character"),
                trait_count: 0,
            }),
        ];
        let payload = encode_payload(1, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "a character with no country must encode and decode cleanly: {:?}",
            summary.parse_errors
        );
        assert_eq!(summary.row_count_total, 2);
        assert_eq!(summary.unit_leader_rows, 1);

        // And the ceiling above it is still enforced, so the two ends of the
        // range cannot drift apart: this is where an over-ceiling tag is
        // caught, which is why the census reports kind 32 rather than
        // silently handing the encoder a tag the profile forbids.
        let over = vec![CharacterLaneRow::Character(CharacterRow {
            country_tag_db_id: MAX_TAG_DB_ID + 1,
            char_id: 4712,
            name: None,
            token: None,
        })];
        let over_summary = decode_payload(0, &encode_payload(2, 0, SCHEMA_ROOTS_VALIDATED, &over));
        assert!(
            !over_summary.parse_errors.is_empty(),
            "a tag over the profile ceiling must not validate"
        );
    }

    #[test]
    fn encodes_full_character_leader_cabinet_slice() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 3,
                char_id: 2458,
                name: Some("Georgy Zhukov"),
                token: Some("SOV_georgy_zhukov"),
            }),
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 3,
                unit_leader_id: 676,
                char_id: 2458,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(8),
                experience_raw: Some(42_000_000),
                attack: Some(6),
                defense: Some(7),
                planning: Some(5),
                logistics: Some(10),
                maneuvering: None,
                coordination: None,
                token: Some("SOV_georgy_zhukov"),
                name: Some("Georgy Zhukov"),
                trait_count: 2,
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 676,
                ordinal: 0,
                trait_token: "brilliant_strategist",
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 676,
                ordinal: 1,
                trait_token: "panzer_leader",
            }),
            CharacterLaneRow::CountryLeaderRole(CountryLeaderRoleRow {
                country_tag_db_id: 3,
                char_id: 2450,
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
            }),
            CharacterLaneRow::ActiveRuler(ActiveRulerRow {
                country_tag_db_id: 3,
                char_id: 2450,
                ruling_party: Some("communism"),
                name: Some("Iosif Stalin"),
                token: Some("SOV_iosif_stalin"),
                leader_vec_size: 1,
                ambiguous: false,
            }),
            CharacterLaneRow::AdvisorRoster(AdvisorRow {
                country_tag_db_id: 2,
                char_id: 1001,
                slot: Some("political_advisor"),
                ledger: Some("political_advisor"),
                idea_token: "ENG_maxwell_aitken",
            }),
            CharacterLaneRow::AppointedAdvisor(AdvisorRow {
                country_tag_db_id: 2,
                char_id: 1001,
                slot: Some("political_advisor"),
                ledger: Some("political_advisor"),
                idea_token: "ENG_maxwell_aitken",
            }),
            CharacterLaneRow::CharacterPortrait(CharacterPortraitRow {
                country_tag_db_id: 3,
                char_id: 2458,
                role: "army",
                size: "large",
                gfx: "GFX_portrait_SOV_georgy_zhukov",
            }),
        ];
        let payload = encode_payload(
            123,
            0,
            SCHEMA_ROOTS_VALIDATED
                | SCHEMA_STATIC_NAME_CLOSURE
                | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &rows,
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.schema_flags_or & SCHEMA_ROOTS_VALIDATED,
            SCHEMA_ROOTS_VALIDATED
        );
        assert_eq!(summary.status_flags_or, 0);
        assert_eq!(summary.row_count_total, 9);
        assert_eq!(summary.unit_leader_rows, 1);
        assert_eq!(summary.trait_rows, 2);
        assert_eq!(summary.active_ruler_rows, 1);
        assert_eq!(summary.appointed_advisor_rows, 1);
        assert_eq!(summary.character_portrait_rows, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("unit_leader_trait_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("character_portrait_rows")
                .copied(),
            Some(1)
        );

        let compact = encode_compact_data_record(7, 42, &payload);
        assert_eq!(&compact[0..4], COMPACT_RECORD_MAGIC);
        assert_eq!(
            u16::from_le_bytes(compact[20..22].try_into().unwrap()),
            DOMAIN_ID
        );
        assert_eq!(
            u16::from_le_bytes(compact[22..24].try_into().unwrap()),
            COMPACT_RECORD_TYPE_DOMAIN_DATA
        );
    }

    #[test]
    fn accepts_measured_empty_appointee_slice() {
        let rows = vec![CharacterLaneRow::ActiveRuler(ActiveRulerRow {
            country_tag_db_id: 20,
            char_id: 300,
            ruling_party: Some("neutrality"),
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_antonio_de_oliveira_salazar"),
            leader_vec_size: 1,
            ambiguous: false,
        })];
        let payload = encode_payload(
            25,
            0,
            SCHEMA_ROOTS_VALIDATED | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            &rows,
        );
        let summary = decode_payload(12, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.appointed_advisor_rows, 0);
        assert_eq!(summary.active_ruler_rows, 1);
    }

    #[test]
    fn preserves_utf8_display_names() {
        let rows = vec![
            CharacterLaneRow::Character(CharacterRow {
                country_tag_db_id: 1,
                char_id: 701,
                name: Some("Günther Lütjens"),
                token: Some("GER_gunther_lutjens"),
            }),
            CharacterLaneRow::ActiveRuler(ActiveRulerRow {
                country_tag_db_id: 1,
                char_id: 701,
                ruling_party: Some("neutrality"),
                name: Some("Günther Lütjens"),
                token: Some("GER_gunther_lutjens"),
                leader_vec_size: 1,
                ambiguous: false,
            }),
        ];
        let payload = encode_payload(25, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let decoded = decode_payload_rows(0, &payload).unwrap();
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            DecodedDomain47Row::Character { name: Some(name), .. }
                if name == "Günther Lütjens"
        )));
        assert!(decoded.rows.iter().any(|row| matches!(
            row,
            DecodedDomain47Row::ActiveRuler { name: Some(name), .. }
                if name == "Günther Lütjens"
        )));
    }

    #[test]
    fn rejects_unknown_advisor_bucket_values_outside_enum() {
        let rows = vec![CharacterLaneRow::AdvisorRoster(AdvisorRow {
            country_tag_db_id: 2,
            char_id: 1001,
            slot: Some("political_advisor"),
            ledger: Some("political"),
            idea_token: "ENG_maxwell_aitken",
        })];
        let payload = encode_payload(25, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.iter().any(
                |err| err.contains("cabinet_bucket") && err.contains("invalid value political")
            ),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn rejects_trait_count_mismatch_and_duplicate_unit_leader() {
        let rows = vec![
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 1,
                unit_leader_id: 10,
                char_id: 100,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(4),
                experience_raw: Some(1),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("GER_test"),
                name: Some("Test"),
                trait_count: 2,
            }),
            CharacterLaneRow::UnitLeader(UnitLeaderRow {
                country_tag_db_id: 1,
                unit_leader_id: 10,
                char_id: 101,
                leader_kind: UNIT_LEADER_ARMY,
                skill: Some(4),
                experience_raw: Some(1),
                attack: Some(1),
                defense: Some(1),
                planning: Some(1),
                logistics: Some(1),
                maneuvering: None,
                coordination: None,
                token: Some("GER_test_2"),
                name: Some("Test 2"),
                trait_count: 0,
            }),
            CharacterLaneRow::UnitLeaderTrait(UnitLeaderTraitRow {
                unit_leader_id: 10,
                ordinal: 0,
                trait_token: "old_guard",
            }),
        ];
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(4, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates unit leader id 10")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("declared 0 traits but decoded 1")));
    }

    #[test]
    fn rejects_bad_active_ruler_ambiguity_flag() {
        let rows = vec![CharacterLaneRow::ActiveRuler(ActiveRulerRow {
            country_tag_db_id: 3,
            char_id: 2450,
            ruling_party: Some("communism"),
            name: Some("Iosif Stalin"),
            token: Some("SOV_iosif_stalin"),
            leader_vec_size: 2,
            ambiguous: false,
        })];
        let payload = encode_payload(0, 0, SCHEMA_ROOTS_VALIDATED, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("ambiguous flag disagrees")));
    }
}
