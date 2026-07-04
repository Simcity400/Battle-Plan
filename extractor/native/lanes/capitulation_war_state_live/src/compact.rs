//! Domain 60 wire format: self-delimiting payload, strict exact-end decode.
//!
//! Layout (all little-endian):
//!   header (32 bytes):
//!     0x00 u64 elapsed_wall_ms
//!     0x08 u32 status_flags
//!     0x0C u32 schema_version (== 1)
//!     0x10 u32 country_row_count
//!     0x14 u32 war_row_count
//!     0x18 u32 fixed_scale_raw (== 100_000, legitimacy scale)
//!     0x1C u32 reserved (== 0)
//!   country rows (28 bytes each):
//!     u32 tag_db_id | u32 flags | i32 capitulated_date_raw |
//!     i32 last_surrender_date_raw | i32 faction_join_date_raw |
//!     i32 legitimacy_raw | i32 exile_army_leaders
//!   war rows (32 bytes each):
//!     u32 first_tag_db_id | u32 second_tag_db_id | i32 start_date_raw |
//!     u32 reserved (== 0) | i64 first_casualties | i64 second_casualties
//!
//! Date raws share the engine hour-counter encoding used by `game_hour_raw`
//! (`compact_contract::calendar`). The capitulation date fields use the
//! engine's own null sentinel `CGAMEDATE_NULL_RAW` (validated live: 422/437
//! countries carry it for `capitulated_date`); a war `start_date_raw` must be
//! a real in-window date (958/958 validated). Absent country rows mean "no
//! diplomacy sub-object resolved" — never zero-filled (absent != 0).

pub const DOMAIN_ID: u16 = 60;
pub const DOMAIN_NAME: &str = "capitulation_war_state_live";
pub const SCHEMA_VERSION: u32 = 1;
/// Legitimacy fixed-point scale (i32 raw x 1e5, FINAL_SWEEP §T1 8/8 exact).
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const HEADER_LEN: usize = 32;
pub const ROW_COUNTRY_LEN: usize = 28;
pub const ROW_WAR_LEN: usize = 32;

// Global status mirrors (record-header vocabulary, kept in-payload like the
// other lane snapshots so the fold can gate on the payload alone).
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
/// Lane badge (in-payload only): live war casualty fields are RUNNING
/// accumulators — byte-exact vs the save snapshot for frozen wars, but they
/// legitimately exceed the serialized per-country snapshot on active wars
/// (T1 Codex verdict). MANDATORY whenever war rows are present; the decoder
/// fails closed on unbadged war rows so the semantic can never silently
/// launder into "save-exact casualties".
pub const STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR: u32 = 1 << 16;

pub const KNOWN_STATUS_FLAGS: u32 = compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE
    | compact_contract::COMPACT_STATUS_PROFILE_MISMATCH
    | compact_contract::COMPACT_STATUS_NO_CLOCK
    | compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE
    | compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE
    | STATUS_WORKER_PANIC
    | compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | compact_contract::COMPACT_STATUS_CONTROL_INVALID
    | STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR;

/// Country row flag: the validated `capitulated` u8 read 1.
pub const COUNTRY_FLAG_CAPITULATED: u32 = 1;
pub const KNOWN_COUNTRY_FLAGS: u32 = COUNTRY_FLAG_CAPITULATED;

/// Engine CGameDate null sentinel (raw hours for 1.1.1.0). Validated live:
/// 422/437 `capitulated_date`, 427/437 `last_surrender_date`, 365/437
/// `faction_join_date` carry exactly this value on D04.
pub const CGAMEDATE_NULL_RAW: i32 = 43_808_760;

pub const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
/// Same guard the domain 53 war reader uses.
pub const MAX_CASUALTIES: i64 = 50_000_000;
/// Legitimacy is a 0..=100.0 percentage x 1e5 on the observed save; guard with
/// headroom for modded values without admitting garbage.
pub const MAX_LEGITIMACY_RAW: i32 = 1_000_000_000;
pub const MAX_EXILE_ARMY_LEADERS: i32 = 100_000;
pub const MAX_COUNTRY_ROWS: u32 = 8_192;
pub const MAX_WAR_ROWS: u32 = 8_192;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryCapitulationRow {
    pub tag_db_id: u32,
    pub capitulated: bool,
    /// Raw engine hour counter; `CGAMEDATE_NULL_RAW` == engine null.
    pub capitulated_date_raw: i32,
    pub last_surrender_date_raw: i32,
    pub faction_join_date_raw: i32,
    /// Fixed-point x `FIXED_SCALE_RAW`.
    pub legitimacy_raw: i32,
    pub exile_army_leaders: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarRow {
    pub first_tag_db_id: u32,
    pub second_tag_db_id: u32,
    /// Raw engine hour counter (in-window real date; 958/958 validated).
    pub start_date_raw: i32,
    /// Live running accumulators (see the mandatory status badge).
    pub first_casualties: i64,
    pub second_casualties: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapitulationWarSnapshot {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub countries: Vec<CountryCapitulationRow>,
    pub wars: Vec<WarRow>,
}

impl CapitulationWarSnapshot {
    pub fn row_count(&self) -> u32 {
        (self.countries.len().min(u32::MAX as usize) as u32)
            .saturating_add(self.wars.len().min(u32::MAX as usize) as u32)
    }
}

/// A capitulation-date-family raw is either the engine null sentinel or a real
/// in-window date.
pub fn date_raw_valid_nullable(raw: i32) -> bool {
    raw == CGAMEDATE_NULL_RAW || compact_contract::calendar::hour_index_from_raw(raw).is_some()
}

/// A war start date must be a real in-window date (no null observed live).
pub fn war_start_date_raw_valid(raw: i32) -> bool {
    compact_contract::calendar::hour_index_from_raw(raw).is_some()
}

pub fn encode_payload(snapshot: &CapitulationWarSnapshot) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        HEADER_LEN + snapshot.countries.len() * ROW_COUNTRY_LEN + snapshot.wars.len() * ROW_WAR_LEN,
    );
    payload.extend_from_slice(&snapshot.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&snapshot.status_flags.to_le_bytes());
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload
        .extend_from_slice(&(snapshot.countries.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.wars.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in &snapshot.countries {
        let mut flags = 0u32;
        if row.capitulated {
            flags |= COUNTRY_FLAG_CAPITULATED;
        }
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.capitulated_date_raw.to_le_bytes());
        payload.extend_from_slice(&row.last_surrender_date_raw.to_le_bytes());
        payload.extend_from_slice(&row.faction_join_date_raw.to_le_bytes());
        payload.extend_from_slice(&row.legitimacy_raw.to_le_bytes());
        payload.extend_from_slice(&row.exile_army_leaders.to_le_bytes());
    }
    for row in &snapshot.wars {
        payload.extend_from_slice(&row.first_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.second_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.start_date_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&row.first_casualties.to_le_bytes());
        payload.extend_from_slice(&row.second_casualties.to_le_bytes());
    }
    payload
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 bytes"))
}

fn read_i32(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 bytes"))
}

fn read_u64(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().expect("u64 bytes"))
}

fn read_i64(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(payload[offset..offset + 8].try_into().expect("i64 bytes"))
}

/// Strict full decode used by the recorder fold and by verification. Every
/// structural or semantic violation is an error — no partial acceptance.
pub fn decode_snapshot_payload(
    offset: usize,
    payload: &[u8],
) -> Result<CapitulationWarSnapshot, Vec<String>> {
    let mut errors = Vec::new();
    if payload.len() < HEADER_LEN {
        return Err(vec![format!(
            "offset {offset}: capitulation_war_state payload len {} < header {HEADER_LEN}",
            payload.len()
        )]);
    }
    let elapsed_wall_ms = read_u64(payload, 0x00);
    let status_flags = read_u32(payload, 0x08);
    let schema_version = read_u32(payload, 0x0C);
    let country_row_count = read_u32(payload, 0x10);
    let war_row_count = read_u32(payload, 0x14);
    let fixed_scale_raw = read_u32(payload, 0x18);
    let reserved = read_u32(payload, 0x1C);
    if schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "offset {offset}: capitulation_war_state schema_version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if fixed_scale_raw != FIXED_SCALE_RAW {
        errors.push(format!(
            "offset {offset}: capitulation_war_state fixed_scale_raw {fixed_scale_raw} != {FIXED_SCALE_RAW}"
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: capitulation_war_state header reserved {reserved} != 0"
        ));
    }
    if status_flags & !KNOWN_STATUS_FLAGS != 0 {
        errors.push(format!(
            "offset {offset}: capitulation_war_state unknown status flags {:#x}",
            status_flags & !KNOWN_STATUS_FLAGS
        ));
    }
    if country_row_count > MAX_COUNTRY_ROWS {
        errors.push(format!(
            "offset {offset}: capitulation_war_state country_row_count {country_row_count} > {MAX_COUNTRY_ROWS}"
        ));
    }
    if war_row_count > MAX_WAR_ROWS {
        errors.push(format!(
            "offset {offset}: capitulation_war_state war_row_count {war_row_count} > {MAX_WAR_ROWS}"
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let expected_len = HEADER_LEN
        + country_row_count as usize * ROW_COUNTRY_LEN
        + war_row_count as usize * ROW_WAR_LEN;
    if payload.len() != expected_len {
        return Err(vec![format!(
            "offset {offset}: capitulation_war_state payload len {} != expected {expected_len}",
            payload.len()
        )]);
    }
    if war_row_count > 0 && status_flags & STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR == 0 {
        return Err(vec![format!(
            "offset {offset}: capitulation_war_state war rows lack the mandatory running-accumulator badge"
        )]);
    }

    let mut cursor = HEADER_LEN;
    let mut countries = Vec::with_capacity(country_row_count as usize);
    for index in 0..country_row_count {
        let tag_db_id = read_u32(payload, cursor);
        let flags = read_u32(payload, cursor + 4);
        let capitulated_date_raw = read_i32(payload, cursor + 8);
        let last_surrender_date_raw = read_i32(payload, cursor + 12);
        let faction_join_date_raw = read_i32(payload, cursor + 16);
        let legitimacy_raw = read_i32(payload, cursor + 20);
        let exile_army_leaders = read_i32(payload, cursor + 24);
        cursor += ROW_COUNTRY_LEN;
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            errors.push(format!(
                "offset {offset}: country row {index} invalid tag_db_id {tag_db_id}"
            ));
            continue;
        }
        if flags & !KNOWN_COUNTRY_FLAGS != 0 {
            errors.push(format!(
                "offset {offset}: country row {index} unknown flags {:#x}",
                flags & !KNOWN_COUNTRY_FLAGS
            ));
            continue;
        }
        if !date_raw_valid_nullable(capitulated_date_raw)
            || !date_raw_valid_nullable(last_surrender_date_raw)
            || !date_raw_valid_nullable(faction_join_date_raw)
        {
            errors.push(format!(
                "offset {offset}: country row {index} date raw out of window"
            ));
            continue;
        }
        if !(0..=MAX_LEGITIMACY_RAW).contains(&legitimacy_raw) {
            errors.push(format!(
                "offset {offset}: country row {index} legitimacy_raw {legitimacy_raw} out of range"
            ));
            continue;
        }
        if !(0..=MAX_EXILE_ARMY_LEADERS).contains(&exile_army_leaders) {
            errors.push(format!(
                "offset {offset}: country row {index} exile_army_leaders {exile_army_leaders} out of range"
            ));
            continue;
        }
        countries.push(CountryCapitulationRow {
            tag_db_id,
            capitulated: flags & COUNTRY_FLAG_CAPITULATED != 0,
            capitulated_date_raw,
            last_surrender_date_raw,
            faction_join_date_raw,
            legitimacy_raw,
            exile_army_leaders,
        });
    }

    let mut wars = Vec::with_capacity(war_row_count as usize);
    for index in 0..war_row_count {
        let first_tag_db_id = read_u32(payload, cursor);
        let second_tag_db_id = read_u32(payload, cursor + 4);
        let start_date_raw = read_i32(payload, cursor + 8);
        let reserved = read_u32(payload, cursor + 12);
        let first_casualties = read_i64(payload, cursor + 16);
        let second_casualties = read_i64(payload, cursor + 24);
        cursor += ROW_WAR_LEN;
        if first_tag_db_id == 0
            || first_tag_db_id > MAX_TAG_DB_ID
            || second_tag_db_id == 0
            || second_tag_db_id > MAX_TAG_DB_ID
            || first_tag_db_id == second_tag_db_id
        {
            errors.push(format!(
                "offset {offset}: war row {index} invalid tag pair {first_tag_db_id}/{second_tag_db_id}"
            ));
            continue;
        }
        if reserved != 0 {
            errors.push(format!(
                "offset {offset}: war row {index} reserved {reserved} != 0"
            ));
            continue;
        }
        if !war_start_date_raw_valid(start_date_raw) {
            errors.push(format!(
                "offset {offset}: war row {index} start_date_raw {start_date_raw} out of window"
            ));
            continue;
        }
        if !(0..=MAX_CASUALTIES).contains(&first_casualties)
            || !(0..=MAX_CASUALTIES).contains(&second_casualties)
        {
            errors.push(format!(
                "offset {offset}: war row {index} casualties out of range"
            ));
            continue;
        }
        wars.push(WarRow {
            first_tag_db_id,
            second_tag_db_id,
            start_date_raw,
            first_casualties,
            second_casualties,
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(CapitulationWarSnapshot {
        elapsed_wall_ms,
        status_flags,
        countries,
        wars,
    })
}

/// Recorder compact-summary totals (pack_replay verify path).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaneSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub country_rows: u64,
    pub war_rows: u64,
    pub parse_errors: Vec<String>,
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> LaneSummary {
    match decode_snapshot_payload(offset, payload) {
        Ok(snapshot) => LaneSummary {
            status_flags_or: snapshot.status_flags,
            data_rows_total: u64::from(snapshot.row_count()),
            data_changed_rows_total: u64::from(snapshot.row_count()),
            country_rows: snapshot.countries.len() as u64,
            war_rows: snapshot.wars.len() as u64,
            parse_errors: Vec::new(),
        },
        Err(errors) => LaneSummary {
            parse_errors: errors,
            ..LaneSummary::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;

    fn real_date_raw(year: i32, doy: i32, hour: i32) -> i32 {
        DATE_EPOCH_CONST + (year * 365 + doy) * 24 + hour
    }

    fn sample_snapshot() -> CapitulationWarSnapshot {
        CapitulationWarSnapshot {
            elapsed_wall_ms: 7,
            status_flags: STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR,
            countries: vec![
                CountryCapitulationRow {
                    tag_db_id: 3,
                    capitulated: true,
                    // France 1940.6.18: month 6 day 18 -> doy 31+28+31+30+31+17.
                    capitulated_date_raw: real_date_raw(1940, 31 + 28 + 31 + 30 + 31 + 17, 2),
                    last_surrender_date_raw: CGAMEDATE_NULL_RAW,
                    faction_join_date_raw: CGAMEDATE_NULL_RAW,
                    legitimacy_raw: 10_000_000,
                    exile_army_leaders: 12,
                },
                CountryCapitulationRow {
                    tag_db_id: 47,
                    capitulated: false,
                    capitulated_date_raw: CGAMEDATE_NULL_RAW,
                    last_surrender_date_raw: CGAMEDATE_NULL_RAW,
                    faction_join_date_raw: real_date_raw(1936, 10, 5),
                    legitimacy_raw: 0,
                    exile_army_leaders: 0,
                },
            ],
            wars: vec![WarRow {
                first_tag_db_id: 47,
                second_tag_db_id: 3,
                start_date_raw: real_date_raw(1939, 244, 4),
                first_casualties: 1_110,
                second_casualties: 948,
            }],
        }
    }

    #[test]
    fn roundtrip_encode_decode() {
        let snapshot = sample_snapshot();
        let payload = encode_payload(&snapshot);
        assert_eq!(
            payload.len(),
            HEADER_LEN + 2 * ROW_COUNTRY_LEN + ROW_WAR_LEN
        );
        let decoded = decode_snapshot_payload(0, &payload).expect("decodes");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn empty_snapshot_roundtrips() {
        let snapshot = CapitulationWarSnapshot {
            elapsed_wall_ms: 1,
            status_flags: STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA,
            ..CapitulationWarSnapshot::default()
        };
        let payload = encode_payload(&snapshot);
        assert_eq!(payload.len(), HEADER_LEN);
        let decoded = decode_snapshot_payload(0, &payload).expect("decodes");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn war_rows_without_accumulator_badge_fail_closed() {
        let mut snapshot = sample_snapshot();
        snapshot.status_flags = 0;
        let payload = encode_payload(&snapshot);
        let errors = decode_snapshot_payload(0, &payload).expect_err("must reject");
        assert!(errors
            .iter()
            .any(|error| error.contains("running-accumulator badge")));
    }

    #[test]
    fn truncated_and_padded_payloads_fail() {
        let payload = encode_payload(&sample_snapshot());
        assert!(decode_snapshot_payload(0, &payload[..payload.len() - 1]).is_err());
        let mut padded = payload.clone();
        padded.push(0);
        assert!(decode_snapshot_payload(0, &padded).is_err());
    }

    #[test]
    fn unknown_country_flags_fail() {
        let mut snapshot = sample_snapshot();
        let payload = {
            snapshot.countries[0].capitulated = true;
            let mut payload = encode_payload(&snapshot);
            // Set an unknown flag bit in the first country row.
            let flags_offset = HEADER_LEN + 4;
            payload[flags_offset] |= 0x80;
            payload
        };
        let errors = decode_snapshot_payload(0, &payload).expect_err("must reject");
        assert!(errors.iter().any(|error| error.contains("unknown flags")));
    }

    #[test]
    fn out_of_window_dates_fail() {
        let mut snapshot = sample_snapshot();
        snapshot.countries[0].capitulated_date_raw = 123;
        let payload = encode_payload(&snapshot);
        assert!(decode_snapshot_payload(0, &payload).is_err());

        let mut snapshot = sample_snapshot();
        snapshot.wars[0].start_date_raw = CGAMEDATE_NULL_RAW;
        let payload = encode_payload(&snapshot);
        assert!(decode_snapshot_payload(0, &payload).is_err());
    }

    #[test]
    fn null_sentinel_dates_are_accepted_and_flag_roundtrips() {
        let snapshot = sample_snapshot();
        let payload = encode_payload(&snapshot);
        let decoded = decode_snapshot_payload(0, &payload).expect("decodes");
        assert!(decoded.countries[0].capitulated);
        assert!(!decoded.countries[1].capitulated);
        assert_eq!(
            decoded.countries[0].last_surrender_date_raw,
            CGAMEDATE_NULL_RAW
        );
        assert!(date_raw_valid_nullable(CGAMEDATE_NULL_RAW));
        assert!(!war_start_date_raw_valid(CGAMEDATE_NULL_RAW));
    }

    #[test]
    fn duplicate_or_reflexive_war_pairs_fail() {
        let mut snapshot = sample_snapshot();
        snapshot.wars[0].second_tag_db_id = snapshot.wars[0].first_tag_db_id;
        let payload = encode_payload(&snapshot);
        assert!(decode_snapshot_payload(0, &payload).is_err());
    }

    #[test]
    fn summary_counts_rows_by_family() {
        let payload = encode_payload(&sample_snapshot());
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty());
        assert_eq!(summary.country_rows, 2);
        assert_eq!(summary.war_rows, 1);
        assert_eq!(summary.data_rows_total, 3);
        assert_eq!(
            summary.status_flags_or & STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR,
            STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR
        );
    }
}
