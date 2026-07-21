//! Domain-46 air ace roster: the `ACE1` trailer codec.
//!
//! The ace roster ships INSIDE the domain-46 `air_variant_archetypes` data
//! record, appended after the self-delimiting base payload (the MTD1
//! purchase-contract trailer precedent from domain 43). This preserves the
//! stream invariant of exactly one data record per domain per frame; trailer
//! presence is decided by trailing bytes + the `ACE1` magic, which is
//! unambiguous because the base payload's exact length is derived from its
//! own header (a legacy no-trailer record decodes exactly to its end).
//!
//! Rows are FULL-REPLACE per frame (no delta/tombstones): every valid roster
//! ace is re-emitted, and the fold replaces its whole ace map whenever a
//! trailer is present. All fields fail closed: `absent != 0` is enforced by
//! presence flags, and killer fields are only valid on a confirmed-dead ace
//! (their content on living aces was never validated).

use std::collections::{BTreeMap, BTreeSet};

pub const ACE_TRAILER_MAGIC: &[u8; 4] = b"ACE1";
pub const ACE_TRAILER_VERSION: u16 = 2;
pub const ACE_TRAILER_VERSION_V1: u16 = 1;
pub const ACE_ROSTER_AUTHORITY: &str = "exact_a729_cace_owner_roster";
pub const ACE_ROSTER_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const ACE_ROSTER_FUNCTION_MANIFEST_SHA256: &str =
    "f85162e5b2a531962e1080fb3add1867f4522a2aba7811d013a9f43c8e708720";
pub const ACE_ROSTER_ORDERING: &str = "country_ace_index_then_ace_id";
pub const ACE_TRAILER_HEADER_LEN: usize = 40;
pub const ACE_TRAILER_ROW_LEN: usize = 40;
pub const ACE_TRAILER_ROW_LEN_V1: usize = 32;

// Trailer-local status flags. These are NEVER fed into the global compact
// status interpretation (the record-level status stays the base archetype
// payload's); values are chosen to match the global semantics where one
// exists so a human reading a dump is not misled.
pub const ACE_STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const ACE_STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const ACE_STATUS_WING_ASSIGNMENT_BLOCKED: u32 = 1 << 13;
pub const ACE_KNOWN_STATUS_MASK: u32 =
    ACE_STATUS_SOURCE_UNAVAILABLE | ACE_STATUS_PARTIAL_DATA | ACE_STATUS_WING_ASSIGNMENT_BLOCKED;

pub const ROW_FLAG_NAME: u32 = 1 << 1;
pub const ROW_FLAG_SURNAME: u32 = 1 << 2;
pub const ROW_FLAG_CALLSIGN: u32 = 1 << 3;
pub const ROW_FLAG_WING_ID: u32 = 1 << 4;
/// The alive byte (+0x158) read back a validated boolean; without this flag
/// the row's death state is UNKNOWN (torn/out-of-domain read), never "alive".
pub const ROW_FLAG_ALIVE_KNOWN: u32 = 1 << 5;
/// Requires ROW_FLAG_ALIVE_KNOWN. Set = dead (alive byte 0).
pub const ROW_FLAG_DEAD: u32 = 1 << 6;
/// Requires ROW_FLAG_DEAD. Killer ace id (+0x164) present. Independent of
/// KILLER_COUNTRY: two D04 dead aces carry a killer country but no killer
/// ace id.
pub const ROW_FLAG_KILLER_ACE: u32 = 1 << 7;
/// Requires ROW_FLAG_DEAD. Killer country tag-db-id (+0x168) present.
pub const ROW_FLAG_KILLER_COUNTRY: u32 = 1 << 8;
/// The serializer-owned handled byte (`CAceEntry+0x16C`) was read as a
/// validated boolean. A trailer-v1 row leaves this field unknown.
pub const ROW_FLAG_HANDLED_KNOWN: u32 = 1 << 9;
/// Requires `ROW_FLAG_HANDLED_KNOWN`. Set = the death record was handled.
pub const ROW_FLAG_HANDLED: u32 = 1 << 10;
/// The serializer-owned raw kill type (`CAceEntry+0x15C`) was read. No enum
/// label is assigned because the retained corpus only proves raw value 1.
pub const ROW_FLAG_KILL_TYPE_KNOWN: u32 = 1 << 11;

pub const SOURCE_LIVE_COUNTRY_ACE_VECTOR: u32 = 1;
pub const SOURCE_LIVE_ACE_OBJECT_LAYOUT: u32 = 1 << 1;
pub const SOURCE_DEV_SAVE_LAYOUT_VALIDATION: u32 = 1 << 2;
pub const SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED: u32 = 1 << 3;
pub const SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT: u32 = 1 << 4;
pub const SOURCE_LIVE_ACE_DEATH_RECORD: u32 = 1 << 5;
pub const SOURCE_LIVE_ACE_LIFECYCLE_RECORD: u32 = 1 << 6;

pub const MAX_TAG_DB_ID: u32 = 4096;
pub const MAX_ACE_ID: u32 = i32::MAX as u32;
pub const MAX_COUNTRY_ACE_INDEX: u32 = 4096;
pub const MAX_WING_ID: u32 = i32::MAX as u32;
/// UTF-8 byte cap. Raw cp1252 strings are capped at 96 bytes at the read
/// layer; cp1252->UTF-8 can grow each byte to at most 3 bytes.
pub const MAX_STRING_LEN: usize = 96 * 3;
pub const MAX_ACE_ROWS: usize = 65_536;

const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_NAME
    | ROW_FLAG_SURNAME
    | ROW_FLAG_CALLSIGN
    | ROW_FLAG_WING_ID
    | ROW_FLAG_ALIVE_KNOWN
    | ROW_FLAG_DEAD
    | ROW_FLAG_KILLER_ACE
    | ROW_FLAG_KILLER_COUNTRY
    | ROW_FLAG_HANDLED_KNOWN
    | ROW_FLAG_HANDLED
    | ROW_FLAG_KILL_TYPE_KNOWN;
const KNOWN_ROW_FLAGS_V1: u32 = ROW_FLAG_NAME
    | ROW_FLAG_SURNAME
    | ROW_FLAG_CALLSIGN
    | ROW_FLAG_WING_ID
    | ROW_FLAG_ALIVE_KNOWN
    | ROW_FLAG_DEAD
    | ROW_FLAG_KILLER_ACE
    | ROW_FLAG_KILLER_COUNTRY;
const KNOWN_SOURCE_MASK: u32 = SOURCE_LIVE_COUNTRY_ACE_VECTOR
    | SOURCE_LIVE_ACE_OBJECT_LAYOUT
    | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
    | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED
    | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT
    | SOURCE_LIVE_ACE_DEATH_RECORD
    | SOURCE_LIVE_ACE_LIFECYCLE_RECORD;
const KNOWN_SOURCE_MASK_V1: u32 = SOURCE_LIVE_COUNTRY_ACE_VECTOR
    | SOURCE_LIVE_ACE_OBJECT_LAYOUT
    | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
    | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED
    | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT
    | SOURCE_LIVE_ACE_DEATH_RECORD;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirAceRosterHeader {
    pub status_flags: u32,
    pub observed_country_rows: u32,
    pub observed_ace_rows: u32,
    pub wing_assignment_rows: u32,
    pub dead_rows: u32,
    pub skipped_rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirAceRosterRow {
    pub tag_db_id: u32,
    pub ace_id: u32,
    pub country_ace_index: u32,
    pub wing_id: Option<u32>,
    pub alive: Option<bool>,
    pub handled: Option<bool>,
    pub kill_type_raw: Option<u32>,
    pub killer_ace_id: Option<u32>,
    pub killer_country_tag_db_id: Option<u32>,
    pub source_mask: u32,
    pub name: Option<String>,
    pub surname: Option<String>,
    pub callsign: Option<String>,
}

/// Mechanically derive the only lifecycle states proven by the raw
/// serializer-owned booleans. Contradictory or incomplete tuples remain
/// absent instead of being normalized.
pub fn ace_lifecycle_state(row: &AirAceRosterRow) -> Option<&'static str> {
    match (row.alive, row.handled) {
        (Some(true), Some(false)) => Some("alive"),
        (Some(false), Some(false)) => Some("dead_unhandled"),
        (Some(false), Some(true)) => Some("dead_handled"),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirAceRosterGateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

fn row_flags(row: &AirAceRosterRow) -> u32 {
    let mut flags = 0u32;
    if row.name.is_some() {
        flags |= ROW_FLAG_NAME;
    }
    if row.surname.is_some() {
        flags |= ROW_FLAG_SURNAME;
    }
    if row.callsign.is_some() {
        flags |= ROW_FLAG_CALLSIGN;
    }
    if row.wing_id.is_some() {
        flags |= ROW_FLAG_WING_ID;
    }
    if let Some(alive) = row.alive {
        flags |= ROW_FLAG_ALIVE_KNOWN;
        if !alive {
            flags |= ROW_FLAG_DEAD;
            if row.killer_ace_id.is_some() {
                flags |= ROW_FLAG_KILLER_ACE;
            }
            if row.killer_country_tag_db_id.is_some() {
                flags |= ROW_FLAG_KILLER_COUNTRY;
            }
        }
    }
    if let Some(handled) = row.handled {
        flags |= ROW_FLAG_HANDLED_KNOWN;
        if handled {
            flags |= ROW_FLAG_HANDLED;
        }
    }
    if row.kill_type_raw.is_some() {
        flags |= ROW_FLAG_KILL_TYPE_KNOWN;
    }
    flags
}

/// Encode the `ACE1` trailer bytes to append after a self-delimiting base
/// archetype payload. The encoder is strict the same way the decoder is:
/// killer fields on a non-dead row are DROPPED (never encoded), so a
/// fail-open reader bug upstream cannot produce a decodable fail-open wire
/// image.
pub fn encode_trailer(header: &AirAceRosterHeader, rows: &[AirAceRosterRow]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        ACE_TRAILER_HEADER_LEN
            .saturating_add(rows.len().saturating_mul(ACE_TRAILER_ROW_LEN))
            .saturating_add(rows.len().saturating_mul(32)),
    );
    out.extend_from_slice(ACE_TRAILER_MAGIC);
    out.extend_from_slice(&ACE_TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(&(ACE_TRAILER_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(ACE_TRAILER_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&header.status_flags.to_le_bytes());
    out.extend_from_slice(&header.observed_country_rows.to_le_bytes());
    out.extend_from_slice(&header.observed_ace_rows.to_le_bytes());
    out.extend_from_slice(&header.wing_assignment_rows.to_le_bytes());
    out.extend_from_slice(&header.dead_rows.to_le_bytes());
    out.extend_from_slice(&header.skipped_rows.to_le_bytes());

    for row in rows {
        let flags = row_flags(row);
        let dead = flags & ROW_FLAG_DEAD != 0;
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.ace_id.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&row.source_mask.to_le_bytes());
        out.extend_from_slice(&row.country_ace_index.to_le_bytes());
        out.extend_from_slice(&row.wing_id.unwrap_or(0).to_le_bytes());
        let killer_ace = if dead {
            row.killer_ace_id.unwrap_or(0)
        } else {
            0
        };
        let killer_country = if dead {
            row.killer_country_tag_db_id.unwrap_or(0)
        } else {
            0
        };
        out.extend_from_slice(&killer_ace.to_le_bytes());
        out.extend_from_slice(&killer_country.to_le_bytes());
        out.extend_from_slice(&row.kill_type_raw.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }

    for row in rows {
        append_compact_string(&mut out, row.name.as_deref());
        append_compact_string(&mut out, row.surname.as_deref());
        append_compact_string(&mut out, row.callsign.as_deref());
    }

    out
}

/// The decoded `ACE1` trailer: validated header + full-replace rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AceTrailerFrame {
    /// Exact wire version retained for the portable authority envelope. V1 is
    /// alive-only historical transport; v2 is the stable raw lifecycle tuple.
    pub version: u16,
    pub header: AirAceRosterHeader,
    pub rows: Vec<AirAceRosterRow>,
}

/// Strictly decode the `ACE1` trailer at `trailer_offset` within `payload`
/// into rows. Every wire-validation rule lives HERE (single source of
/// truth); the gate summary decoder and the world fold both consume this.
/// Any violation fails the whole trailer (`Err`), never a partial decode.
/// The trailer must consume `payload` to its exact end.
pub fn decode_trailer_rows(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<AceTrailerFrame, String> {
    let trailer = payload
        .get(trailer_offset..)
        .ok_or_else(|| format!("ace trailer offset {trailer_offset} is beyond payload end"))?;
    if trailer.len() < ACE_TRAILER_HEADER_LEN {
        return Err(format!(
            "ace trailer is {} byte(s), expected at least {ACE_TRAILER_HEADER_LEN}",
            trailer.len()
        ));
    }
    if &trailer[0..4] != ACE_TRAILER_MAGIC {
        return Err("ace trailer magic is invalid".to_string());
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved = read_u16_le(trailer, 10);
    let v1 = version == ACE_TRAILER_VERSION_V1 && row_len == ACE_TRAILER_ROW_LEN_V1;
    let v2 = version == ACE_TRAILER_VERSION && row_len == ACE_TRAILER_ROW_LEN;
    if (!v1 && !v2) || header_len != ACE_TRAILER_HEADER_LEN || reserved != 0 {
        return Err(format!(
            "ace trailer header shape is invalid (version {version}, header_len {header_len}, row_len {row_len}, reserved {reserved})"
        ));
    }
    let row_count = read_u32_le(trailer, 12) as usize;
    let header = AirAceRosterHeader {
        status_flags: read_u32_le(trailer, 16),
        observed_country_rows: read_u32_le(trailer, 20),
        observed_ace_rows: read_u32_le(trailer, 24),
        wing_assignment_rows: read_u32_le(trailer, 28),
        dead_rows: read_u32_le(trailer, 32),
        skipped_rows: read_u32_le(trailer, 36),
    };

    if header.status_flags & !ACE_KNOWN_STATUS_MASK != 0 {
        return Err(format!(
            "ace trailer has unknown status flags {:#x}",
            header.status_flags
        ));
    }
    if row_count > MAX_ACE_ROWS {
        return Err(format!(
            "ace trailer row_count {row_count} exceeds {MAX_ACE_ROWS}"
        ));
    }
    if row_count != header.observed_ace_rows as usize {
        return Err(format!(
            "ace trailer row_count {row_count} != observed_ace_rows {} (rows are full-replace)",
            header.observed_ace_rows
        ));
    }
    if header.wing_assignment_rows as usize > row_count || header.dead_rows as usize > row_count {
        return Err(format!(
            "ace trailer assignment/dead counters exceed row_count {row_count}"
        ));
    }
    // SOURCE_UNAVAILABLE is a whole-frame absence: it must carry zero rows
    // (absent, never zero-filled). PARTIAL (roster incomplete) still carries
    // the surviving rows for observability; the fold retains prior state on
    // both. `skipped_rows` is an informational counter and may be nonzero on
    // a complete frame (ambiguous-assignment-only drops), so it does NOT by
    // itself require PARTIAL.
    if header.status_flags & ACE_STATUS_SOURCE_UNAVAILABLE != 0 && row_count != 0 {
        return Err(format!(
            "ace trailer SOURCE_UNAVAILABLE carries {row_count} rows (must be empty)"
        ));
    }

    let rows_end = header_len.saturating_add(row_count.saturating_mul(row_len));
    if trailer.len() < rows_end {
        return Err(format!(
            "ace trailer is {} byte(s), expected at least {rows_end}",
            trailer.len()
        ));
    }

    let mut rows = Vec::with_capacity(row_count);
    let mut seen_aces = BTreeSet::new();
    let mut decoded_assignment_rows = 0usize;
    let mut decoded_dead_rows = 0usize;
    let mut row_cursor = header_len;
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, row_cursor);
        let ace_id = read_u32_le(trailer, row_cursor + 4);
        let flags = read_u32_le(trailer, row_cursor + 8);
        let source_mask = read_u32_le(trailer, row_cursor + 12);
        let country_ace_index = read_u32_le(trailer, row_cursor + 16);
        let wing_id = read_u32_le(trailer, row_cursor + 20);
        let killer_ace_id = read_u32_le(trailer, row_cursor + 24);
        let killer_country = read_u32_le(trailer, row_cursor + 28);
        let kill_type_raw = if v1 {
            0
        } else {
            read_u32_le(trailer, row_cursor + 32)
        };
        let row_reserved = if v1 {
            0
        } else {
            read_u32_le(trailer, row_cursor + 36)
        };
        row_cursor += row_len;

        if row_reserved != 0 {
            return Err(format!(
                "ace row {row_index} has nonzero reserved value {row_reserved:#x}"
            ));
        }
        let known_row_flags = if v1 {
            KNOWN_ROW_FLAGS_V1
        } else {
            KNOWN_ROW_FLAGS
        };
        if flags & !known_row_flags != 0 {
            return Err(format!("ace row {row_index} has unknown flags {flags:#x}"));
        }
        if source_mask == 0 {
            return Err(format!("ace row {row_index} has empty source_mask"));
        }
        let known_source_mask = if v1 {
            KNOWN_SOURCE_MASK_V1
        } else {
            KNOWN_SOURCE_MASK
        };
        if source_mask & !known_source_mask != 0 {
            return Err(format!(
                "ace row {row_index} has unknown source_mask {source_mask:#x}"
            ));
        }
        let required_identity_sources =
            SOURCE_LIVE_COUNTRY_ACE_VECTOR | SOURCE_LIVE_ACE_OBJECT_LAYOUT;
        if source_mask & required_identity_sources != required_identity_sources {
            return Err(format!(
                "ace row {row_index} lacks exact country-vector/object-layout identity sources"
            ));
        }
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            return Err(format!(
                "ace row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if ace_id == 0 || ace_id > MAX_ACE_ID {
            return Err(format!("ace row {row_index} has invalid ace_id {ace_id}"));
        }
        if !seen_aces.insert((tag_db_id, ace_id)) {
            return Err(format!(
                "ace row {row_index} duplicates tag_db_id {tag_db_id} ace_id {ace_id}"
            ));
        }
        if country_ace_index > MAX_COUNTRY_ACE_INDEX {
            return Err(format!(
                "ace row {row_index} has invalid country_ace_index {country_ace_index}"
            ));
        }
        let wing = if flags & ROW_FLAG_WING_ID != 0 {
            if wing_id == 0 || wing_id > MAX_WING_ID {
                return Err(format!("ace row {row_index} has invalid wing_id {wing_id}"));
            }
            decoded_assignment_rows = decoded_assignment_rows.saturating_add(1);
            Some(wing_id)
        } else {
            if wing_id != 0 {
                return Err(format!("ace row {row_index} has wing_id without flag"));
            }
            None
        };
        if wing.is_some()
            != (source_mask & SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT != 0)
        {
            return Err(format!(
                "ace row {row_index} wing assignment disagrees with source authority"
            ));
        }
        let frame_assignment_blocked =
            header.status_flags & ACE_STATUS_WING_ASSIGNMENT_BLOCKED != 0;
        if frame_assignment_blocked
            != (source_mask & SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED != 0)
        {
            return Err(format!(
                "ace row {row_index} assignment-blocked source disagrees with frame status"
            ));
        }

        let alive_known = flags & ROW_FLAG_ALIVE_KNOWN != 0;
        let dead = flags & ROW_FLAG_DEAD != 0;
        if dead && !alive_known {
            return Err(format!("ace row {row_index} has DEAD without ALIVE_KNOWN"));
        }
        if dead {
            decoded_dead_rows = decoded_dead_rows.saturating_add(1);
        }
        let killer_ace = if flags & ROW_FLAG_KILLER_ACE != 0 {
            if !dead {
                return Err(format!(
                    "ace row {row_index} has KILLER_ACE on a non-dead row"
                ));
            }
            if killer_ace_id == 0 || killer_ace_id > MAX_ACE_ID {
                return Err(format!(
                    "ace row {row_index} has invalid killer_ace_id {killer_ace_id}"
                ));
            }
            Some(killer_ace_id)
        } else {
            if killer_ace_id != 0 {
                return Err(format!(
                    "ace row {row_index} has killer_ace_id without flag"
                ));
            }
            None
        };
        let killer_country_id = if flags & ROW_FLAG_KILLER_COUNTRY != 0 {
            if !dead {
                return Err(format!(
                    "ace row {row_index} has KILLER_COUNTRY on a non-dead row"
                ));
            }
            if killer_country == 0 || killer_country > MAX_TAG_DB_ID {
                return Err(format!(
                    "ace row {row_index} has invalid killer_country {killer_country}"
                ));
            }
            Some(killer_country)
        } else {
            if killer_country != 0 {
                return Err(format!(
                    "ace row {row_index} has killer_country without flag"
                ));
            }
            None
        };
        if source_mask & SOURCE_LIVE_ACE_DEATH_RECORD != 0 && !alive_known {
            return Err(format!(
                "ace row {row_index} claims death-record source without ALIVE_KNOWN"
            ));
        }
        if alive_known && source_mask & SOURCE_LIVE_ACE_DEATH_RECORD == 0 {
            return Err(format!(
                "ace row {row_index} has ALIVE_KNOWN without death-record source"
            ));
        }
        let handled_known = flags & ROW_FLAG_HANDLED_KNOWN != 0;
        let handled = flags & ROW_FLAG_HANDLED != 0;
        if handled && !handled_known {
            return Err(format!(
                "ace row {row_index} has HANDLED without HANDLED_KNOWN"
            ));
        }
        if alive_known && !dead && handled {
            return Err(format!(
                "ace row {row_index} has contradictory alive=true handled=true tuple"
            ));
        }
        let kill_type_known = flags & ROW_FLAG_KILL_TYPE_KNOWN != 0;
        if !kill_type_known && kill_type_raw != 0 {
            return Err(format!(
                "ace row {row_index} has kill_type_raw without flag"
            ));
        }
        if alive_known && !dead && kill_type_known && kill_type_raw != 0 {
            return Err(format!(
                "ace row {row_index} has nonzero death-only kill_type_raw on a living ace"
            ));
        }
        // V2's lifecycle bytes are captured as one stable tuple.  Partial
        // presence would turn a torn read into apparently authoritative raw
        // state.  V1 is the only supported alive-only wire shape: it predates
        // handled/kill-type capture and is decoded with both new fields absent.
        if !v1 && (alive_known != handled_known || alive_known != kill_type_known) {
            return Err(format!(
                "ace row {row_index} has partial v2 lifecycle tuple presence"
            ));
        }
        if source_mask & SOURCE_LIVE_ACE_LIFECYCLE_RECORD != 0
            && (!alive_known || !handled_known || !kill_type_known)
        {
            return Err(format!(
                "ace row {row_index} claims lifecycle-record source without alive, handled, and kill_type presence"
            ));
        }
        if !v1
            && alive_known
            && source_mask & SOURCE_LIVE_ACE_LIFECYCLE_RECORD == 0
        {
            return Err(format!(
                "ace row {row_index} has a v2 lifecycle tuple without lifecycle-record source"
            ));
        }

        rows.push(AirAceRosterRow {
            tag_db_id,
            ace_id,
            country_ace_index,
            wing_id: wing,
            alive: alive_known.then_some(!dead),
            handled: handled_known.then_some(handled),
            kill_type_raw: kill_type_known.then_some(kill_type_raw),
            killer_ace_id: killer_ace,
            killer_country_tag_db_id: killer_country_id,
            source_mask,
            name: None,
            surname: None,
            callsign: None,
        });
    }

    let mut cursor = rows_end;
    for row_index in 0..row_count {
        let name = read_compact_string(trailer, &mut cursor, "ace name")?;
        let surname = read_compact_string(trailer, &mut cursor, "ace surname")?;
        let callsign = read_compact_string(trailer, &mut cursor, "ace callsign")?;
        let row_flags = read_u32_le(trailer, header_len + row_index * row_len + 8);
        validate_flagged_string(row_index, "name", name.as_deref(), row_flags, ROW_FLAG_NAME)?;
        validate_flagged_string(
            row_index,
            "surname",
            surname.as_deref(),
            row_flags,
            ROW_FLAG_SURNAME,
        )?;
        validate_flagged_string(
            row_index,
            "callsign",
            callsign.as_deref(),
            row_flags,
            ROW_FLAG_CALLSIGN,
        )?;
        let row = &mut rows[row_index];
        row.name = name;
        row.surname = surname;
        row.callsign = callsign;
    }
    if trailer_offset.saturating_add(cursor) != payload.len() {
        return Err(format!(
            "ace trailer has {} trailing byte(s)",
            payload.len().saturating_sub(trailer_offset + cursor)
        ));
    }
    if decoded_assignment_rows != header.wing_assignment_rows as usize {
        return Err(format!(
            "ace trailer wing_assignment_rows {} != decoded {decoded_assignment_rows}",
            header.wing_assignment_rows
        ));
    }
    if decoded_dead_rows != header.dead_rows as usize {
        return Err(format!(
            "ace trailer dead_rows {} != decoded {decoded_dead_rows}",
            header.dead_rows
        ));
    }

    Ok(AceTrailerFrame {
        version,
        header,
        rows,
    })
}

/// Decode + gate the `ACE1` trailer at `trailer_offset` within `payload`,
/// producing gate units. Thin wrapper over `decode_trailer_rows` (the single
/// validation source); a violation surfaces as one parse error.
pub fn decode_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
) -> AirAceRosterGateSummary {
    let mut summary = AirAceRosterGateSummary::default();
    let frame = match decode_trailer_rows(payload, trailer_offset) {
        Ok(frame) => frame,
        Err(error) => {
            summary
                .parse_errors
                .push(format!("offset {offset}: {error}"));
            return summary;
        }
    };
    summary.status_flags_or |= frame.header.status_flags;
    summary.data_rows_total = frame.rows.len() as u64;
    add_unit(&mut summary, "air_ace_roster_rows", frame.rows.len());
    add_unit(
        &mut summary,
        "air_ace_roster_observed_country_rows",
        frame.header.observed_country_rows as usize,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_observed_ace_rows",
        frame.header.observed_ace_rows as usize,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_wing_assignment_rows",
        frame.header.wing_assignment_rows as usize,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_dead_rows",
        frame.header.dead_rows as usize,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_skipped_rows",
        frame.header.skipped_rows as usize,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_identity_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.name.is_some() || row.surname.is_some() || row.callsign.is_some())
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_alive_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.alive == Some(true))
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_death_record_rows",
        frame.rows.iter().filter(|row| row.alive.is_some()).count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_handled_known_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.handled.is_some())
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_handled_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.handled == Some(true))
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_kill_type_known_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.kill_type_raw.is_some())
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_killer_ace_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.killer_ace_id.is_some())
            .count(),
    );
    add_unit(
        &mut summary,
        "air_ace_roster_killer_country_rows",
        frame
            .rows
            .iter()
            .filter(|row| row.killer_country_tag_db_id.is_some())
            .count(),
    );
    summary
}

fn validate_flagged_string(
    row_index: usize,
    label: &str,
    value: Option<&str>,
    flags: u32,
    flag: u32,
) -> Result<(), String> {
    match value {
        Some(text) => {
            if text.len() > MAX_STRING_LEN {
                return Err(format!(
                    "ace row {row_index} {label} string is too long ({})",
                    text.len()
                ));
            }
            if flags & flag == 0 {
                return Err(format!(
                    "ace row {row_index} has {label} string without flag"
                ));
            }
        }
        None => {
            if flags & flag != 0 {
                return Err(format!(
                    "ace row {row_index} has {label} flag with empty string"
                ));
            }
        }
    }
    Ok(())
}

fn read_compact_string(
    bytes: &[u8],
    cursor: &mut usize,
    label: &str,
) -> Result<Option<String>, String> {
    if bytes.len().saturating_sub(*cursor) < 2 {
        return Err(format!("{label} string is truncated before length"));
    }
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    if bytes.len().saturating_sub(*cursor) < len {
        return Err(format!(
            "{label} string length {len} exceeds remaining payload"
        ));
    }
    let body = &bytes[*cursor..*cursor + len];
    *cursor += len;
    if len == 0 {
        return Ok(None);
    }
    String::from_utf8(body.to_vec())
        .map(Some)
        .map_err(|_| format!("{label} string is not valid UTF-8"))
}

fn add_unit(summary: &mut AirAceRosterGateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn append_compact_string(out: &mut Vec<u8>, value: Option<&str>) {
    let bytes = value.unwrap_or("").as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..len]);
}

fn read_u16_le(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive_row(ace_id: u32) -> AirAceRosterRow {
        AirAceRosterRow {
            tag_db_id: 1,
            ace_id,
            country_ace_index: ace_id - 5000,
            wing_id: None,
            alive: Some(true),
            handled: Some(false),
            kill_type_raw: Some(0),
            killer_ace_id: None,
            killer_country_tag_db_id: None,
            source_mask: SOURCE_LIVE_COUNTRY_ACE_VECTOR
                | SOURCE_LIVE_ACE_OBJECT_LAYOUT
                | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
                | SOURCE_LIVE_ACE_DEATH_RECORD
                | SOURCE_LIVE_ACE_LIFECYCLE_RECORD,
            name: Some("Tobias".to_string()),
            surname: Some("Ochsenhirt".to_string()),
            callsign: Some("Fighter of Libau".to_string()),
        }
    }

    fn dead_row(ace_id: u32) -> AirAceRosterRow {
        AirAceRosterRow {
            alive: Some(false),
            handled: Some(true),
            kill_type_raw: Some(1),
            killer_ace_id: Some(6265),
            killer_country_tag_db_id: Some(92),
            ..alive_row(ace_id)
        }
    }

    fn header_for(rows: &[AirAceRosterRow]) -> AirAceRosterHeader {
        AirAceRosterHeader {
            status_flags: 0,
            observed_country_rows: 17,
            observed_ace_rows: rows.len() as u32,
            wing_assignment_rows: rows.iter().filter(|row| row.wing_id.is_some()).count() as u32,
            dead_rows: rows.iter().filter(|row| row.alive == Some(false)).count() as u32,
            skipped_rows: 0,
        }
    }

    #[test]
    fn round_trips_alive_and_dead_rows() {
        let mut with_wing = alive_row(5440);
        with_wing.wing_id = Some(290);
        with_wing.source_mask |= SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;
        let rows = vec![with_wing, dead_row(6265), alive_row(7001)];
        let trailer = encode_trailer(&header_for(&rows), &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 3);
        let units = &summary.data_units_total;
        assert_eq!(units.get("air_ace_roster_rows").copied(), Some(3));
        assert_eq!(
            units.get("air_ace_roster_wing_assignment_rows").copied(),
            Some(1)
        );
        assert_eq!(units.get("air_ace_roster_dead_rows").copied(), Some(1));
        assert_eq!(units.get("air_ace_roster_alive_rows").copied(), Some(2));
        assert_eq!(
            units.get("air_ace_roster_handled_known_rows").copied(),
            Some(3)
        );
        assert_eq!(units.get("air_ace_roster_handled_rows").copied(), Some(1));
        assert_eq!(
            units.get("air_ace_roster_kill_type_known_rows").copied(),
            Some(3)
        );
        assert_eq!(
            units.get("air_ace_roster_killer_ace_rows").copied(),
            Some(1)
        );
        assert_eq!(
            units.get("air_ace_roster_killer_country_rows").copied(),
            Some(1)
        );
        assert_eq!(units.get("air_ace_roster_identity_rows").copied(), Some(3));
    }

    #[test]
    fn dead_row_killer_fields_are_independent() {
        let mut no_killer_id = dead_row(6265);
        no_killer_id.killer_ace_id = None;
        let rows = vec![no_killer_id];
        let trailer = encode_trailer(&header_for(&rows), &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_killer_ace_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_killer_country_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn decodes_v1_rows_without_fabricating_lifecycle_fields() {
        let rows = vec![alive_row(5440)];
        let v2 = encode_trailer(&header_for(&rows), &rows);
        let mut v1 = v2[..ACE_TRAILER_HEADER_LEN].to_vec();
        v1[4..6].copy_from_slice(&ACE_TRAILER_VERSION_V1.to_le_bytes());
        v1[8..10].copy_from_slice(&(ACE_TRAILER_ROW_LEN_V1 as u16).to_le_bytes());
        let mut fixed =
            v2[ACE_TRAILER_HEADER_LEN..ACE_TRAILER_HEADER_LEN + ACE_TRAILER_ROW_LEN_V1].to_vec();
        let flags = read_u32_le(&fixed, 8)
            & !(ROW_FLAG_HANDLED_KNOWN | ROW_FLAG_HANDLED | ROW_FLAG_KILL_TYPE_KNOWN);
        fixed[8..12].copy_from_slice(&flags.to_le_bytes());
        let source = read_u32_le(&fixed, 12) & !SOURCE_LIVE_ACE_LIFECYCLE_RECORD;
        fixed[12..16].copy_from_slice(&source.to_le_bytes());
        v1.extend_from_slice(&fixed);
        v1.extend_from_slice(&v2[ACE_TRAILER_HEADER_LEN + ACE_TRAILER_ROW_LEN..]);

        let frame = decode_trailer_rows(&v1, 0).expect("v1 trailer remains decodable");
        assert_eq!(frame.rows[0].alive, Some(true));
        assert_eq!(frame.rows[0].handled, None);
        assert_eq!(frame.rows[0].kill_type_raw, None);
    }

    #[test]
    fn encoder_drops_killer_fields_on_living_rows() {
        let mut bad = alive_row(5440);
        // A buggy upstream reader claims killer fields on a LIVING ace: the
        // encoder must not let them reach the wire.
        bad.killer_ace_id = Some(6265);
        bad.killer_country_tag_db_id = Some(92);
        let rows = vec![bad];
        let trailer = encode_trailer(&header_for(&rows), &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_killer_ace_rows")
                .copied(),
            Some(0)
        );
    }

    #[test]
    fn decoder_rejects_contradictory_alive_and_handled_tuple() {
        let mut row = alive_row(5440);
        row.handled = Some(true);
        let bytes = encode_trailer(&header_for(std::slice::from_ref(&row)), &[row]);
        let error = decode_trailer_rows(&bytes, 0).expect_err("contradiction must fail closed");
        assert!(error.contains("alive=true handled=true"), "{error}");
    }

    #[test]
    fn decoder_rejects_partial_v2_lifecycle_tuple_presence() {
        let row = alive_row(5440);
        let mut bytes = encode_trailer(&header_for(std::slice::from_ref(&row)), &[row]);
        let flags_offset = ACE_TRAILER_HEADER_LEN + 8;
        let flags = read_u32_le(&bytes, flags_offset) & !ROW_FLAG_KILL_TYPE_KNOWN;
        bytes[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let source_offset = ACE_TRAILER_HEADER_LEN + 12;
        let source = read_u32_le(&bytes, source_offset) & !SOURCE_LIVE_ACE_LIFECYCLE_RECORD;
        bytes[source_offset..source_offset + 4].copy_from_slice(&source.to_le_bytes());

        let error = decode_trailer_rows(&bytes, 0).expect_err("partial tuple must fail closed");
        assert!(error.contains("partial v2 lifecycle tuple"), "{error}");
    }

    #[test]
    fn decoder_rejects_missing_identity_or_lifecycle_source_authority() {
        for clear_source in [
            SOURCE_LIVE_ACE_OBJECT_LAYOUT,
            SOURCE_LIVE_ACE_LIFECYCLE_RECORD,
        ] {
            let row = alive_row(5440);
            let mut bytes = encode_trailer(&header_for(std::slice::from_ref(&row)), &[row]);
            let source_offset = ACE_TRAILER_HEADER_LEN + 12;
            let source = read_u32_le(&bytes, source_offset) & !clear_source;
            bytes[source_offset..source_offset + 4].copy_from_slice(&source.to_le_bytes());
            assert!(
                decode_trailer_rows(&bytes, 0).is_err(),
                "clearing source bit {clear_source:#x} must fail closed"
            );
        }
    }

    #[test]
    fn decoder_rejects_death_only_kill_type_on_living_ace() {
        let mut row = alive_row(5440);
        row.kill_type_raw = Some(1);
        let bytes = encode_trailer(&header_for(std::slice::from_ref(&row)), &[row]);
        let error =
            decode_trailer_rows(&bytes, 0).expect_err("living nonzero kill type must fail");
        assert!(error.contains("death-only kill_type_raw"), "{error}");
    }

    #[test]
    fn rejects_killer_flag_on_non_dead_row() {
        let rows = vec![dead_row(6265)];
        let mut trailer = encode_trailer(&header_for(&rows), &rows);
        // Clear DEAD (bit 6) but keep the killer flags: must be rejected.
        let flags_offset = ACE_TRAILER_HEADER_LEN + 8;
        let mut flags = read_u32_le(&trailer, flags_offset);
        flags &= !ROW_FLAG_DEAD;
        trailer[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("KILLER_ACE on a non-dead row")));
    }

    #[test]
    fn rejects_dead_without_alive_known() {
        let rows = vec![alive_row(5440)];
        let mut trailer = encode_trailer(&header_for(&rows), &rows);
        let flags_offset = ACE_TRAILER_HEADER_LEN + 8;
        let mut flags = read_u32_le(&trailer, flags_offset);
        flags = (flags & !ROW_FLAG_ALIVE_KNOWN) | ROW_FLAG_DEAD;
        trailer[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("DEAD without ALIVE_KNOWN")));
    }

    #[test]
    fn rejects_wing_id_without_flag() {
        let rows = vec![alive_row(5440)];
        let mut trailer = encode_trailer(&header_for(&rows), &rows);
        let wing_offset = ACE_TRAILER_HEADER_LEN + 20;
        trailer[wing_offset..wing_offset + 4].copy_from_slice(&357u32.to_le_bytes());
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("wing_id without flag")));
    }

    #[test]
    fn source_unavailable_must_carry_zero_rows() {
        // SOURCE_UNAVAILABLE is whole-frame absence; carrying rows is a
        // fail-open contradiction (the fold would retain AND the frame claims
        // rows).
        let rows = vec![alive_row(5440)];
        let mut header = header_for(&rows);
        header.status_flags = ACE_STATUS_SOURCE_UNAVAILABLE;
        let trailer = encode_trailer(&header, &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("SOURCE_UNAVAILABLE carries")));
    }

    #[test]
    fn partial_frame_carries_surviving_rows_for_observability() {
        // A partial (incomplete-roster) frame still carries its surviving
        // rows; skipped_rows is informational and does NOT require any
        // particular row count relationship beyond the full-replace identity.
        let rows = vec![alive_row(5440)];
        let mut header = header_for(&rows);
        header.skipped_rows = 3;
        header.status_flags = ACE_STATUS_PARTIAL_DATA;
        let trailer = encode_trailer(&header, &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.status_flags_or, ACE_STATUS_PARTIAL_DATA);
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_skipped_rows")
                .copied(),
            Some(3)
        );
    }

    #[test]
    fn skipped_rows_without_partial_is_allowed_for_ambiguous_only_drops() {
        // Ambiguous-assignment-only drops count toward skipped_rows but do
        // NOT force PARTIAL (identity is complete). This must decode clean.
        let rows = vec![alive_row(5440)];
        let mut header = header_for(&rows);
        header.skipped_rows = 2;
        let trailer = encode_trailer(&header, &rows);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn rejects_trailing_bytes_after_trailer() {
        let rows = vec![alive_row(5440)];
        let mut trailer = encode_trailer(&header_for(&rows), &rows);
        trailer.push(0);
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("trailing byte")));
    }

    #[test]
    fn rejects_invalid_magic() {
        let rows = vec![alive_row(5440)];
        let mut trailer = encode_trailer(&header_for(&rows), &rows);
        trailer[0] = b'X';
        let summary = decode_trailer(0, &trailer, 0);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("magic is invalid")));
    }

    #[test]
    fn decodes_at_nonzero_trailer_offset() {
        let rows = vec![alive_row(5440), dead_row(6265)];
        let trailer = encode_trailer(&header_for(&rows), &rows);
        let mut payload = vec![0u8; 128];
        payload.extend_from_slice(&trailer);
        let summary = decode_trailer(7, &payload, 128);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 2);
    }
}
