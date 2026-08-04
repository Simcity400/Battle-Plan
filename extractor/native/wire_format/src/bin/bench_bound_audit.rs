//! Live-capture audit of the plausibility bounds that discard rows.
//!
//! `bound_trips::bound_exceeded_u32`/`gate_u32` DROP the row they reject, and
//! the drop is invisible in the fold. A bound that never trips is therefore
//! *unaudited*, not confirmed: the only way to tell "correct" from "so tight
//! nothing survives" or "so loose garbage is admitted" is to look at what the
//! bounded field actually holds in a real capture.
//!
//! This walks a raw H4CR collector stream and reports, per bound:
//!
//! * every `bt_*` bound-trip diagnostic unit the collector emitted on the
//!   reserved probe domain (250) — count and last-observed raw sample;
//! * the observed distribution of each bounded field on the wire (max, and how
//!   much headroom the bound leaves), for the domains whose row layout carries
//!   one: d20 divisions, d44 supply logistics, d52 occupation.
//!
//! Read-only: it opens the stream, and writes nothing anywhere.
//!
//! ```text
//! cargo run -p wire_format --bin bench_bound_audit -- <collector_stream.bin>
//! ```

use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufReader, Read};

use compact_contract::{
    COMPACT_RECORD_HEADER_LEN as HEADER_LEN, COMPACT_RECORD_MAGIC as MAGIC,
    COMPACT_RECORD_VERSION as STREAM_RECORD_VERSION,
};
const RT_DOMAIN_PERF: u16 = 1;
const RT_DOMAIN_DATA: u16 = 3;
const PROBE_DOMAIN_ID: u16 = 250;
// Taken from the contract rather than respelled: an audit that guesses the
// trailer flag or magic reports "no bound ever tripped" for a stream full of
// trips, which is exactly the false negative this tool exists to prevent.
use compact_contract::record_semantics::{
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC as PERF_DIAGNOSTIC_MAGIC,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER as PERF_DIAGNOSTIC_TRAILER_FLAG,
};

const D20: u16 = 20;
const D44: u16 = 44;
const D52: u16 = 52;

// d20 presence flags (domain20_compact.rs). Optional fields declare presence
// with these, not by being nonzero.
const D20_ROW_FLAG_OWNER: u32 = 1 << 5;
const D20_ROW_FLAG_MAX_SUPPLY: u32 = 1 << 8;
const D20_ROW_FLAG_CURRENT_SUPPLY: u32 = 1 << 9;
const D20_ROW_FLAG_SUPPLY_GAIN: u32 = 1 << 10;

const D20_HEADER_LEN: usize = 32;
const D20_ROW_LEN: usize = 120;
const D20_LEGACY_ROW_LEN: usize = 104;
const D44_HEADER_LEN: usize = 52;
const D44_HEADER_LEN_LEGACY: usize = 48;
const D44_ROW_LEN: usize = 40;
const D52_HEADER_LEN: usize = 52;
const D52_ROW_LEN: usize = 64;

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn i32le(b: &[u8], at: usize) -> i32 {
    u32le(b, at) as i32
}
fn u64le(b: &[u8], at: usize) -> u64 {
    let mut out = [0u8; 8];
    out.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(out)
}

/// One bounded wire field's observed distribution.
#[derive(Default)]
struct FieldStats {
    /// Values considered (present-and-nonzero for optional fields).
    observed: u64,
    max: Option<i64>,
    min: Option<i64>,
    /// How many observations landed at or above the bound.
    at_or_over_bound: u64,
    /// Second-largest DISTINCT value, so one outlier cannot hide the body.
    ///
    /// Tracked as its own `Option` rather than seeded from the first
    /// observation: seeding it makes it unable to fall, so the sequence
    /// `10, 9` reported `second_max = 10` — a duplicate of the maximum
    /// masquerading as the runner-up.
    second_max: Option<i64>,
}

impl FieldStats {
    fn record(&mut self, value: i64, bound: i64) {
        self.observed = self.observed.saturating_add(1);
        self.min = Some(self.min.map_or(value, |min| min.min(value)));
        match self.max {
            None => self.max = Some(value),
            Some(max) if value > max => {
                self.second_max = Some(max);
                self.max = Some(value);
            }
            Some(max) if value < max => {
                self.second_max = Some(self.second_max.map_or(value, |s| s.max(value)));
            }
            // Equal to the current maximum: a repeat is not a runner-up.
            Some(_) => {}
        }
        if value >= bound {
            self.at_or_over_bound = self.at_or_over_bound.saturating_add(1);
        }
    }
}

struct Audit {
    /// bound name -> (bound value, stats)
    fields: BTreeMap<&'static str, (i64, FieldStats)>,
    /// bt_* unit name -> (summed count, last sample seen)
    bound_trip_units: BTreeMap<String, (u64, u64)>,
    /// every other probe-domain unit name, so a renamed unit is not silently
    /// reported as "the bound never tripped".
    probe_unit_names: BTreeMap<String, u64>,
    records: BTreeMap<u16, u64>,
    data_records: BTreeMap<u16, u64>,
    record_types: BTreeMap<u16, u64>,
    /// domain -> (records this tool could NOT scan, up to three distinct
    /// reasons). Anything in here means the field statistics below cover a
    /// SUBSET of the records, which the report must say out loud.
    unscanned: BTreeMap<u16, (u64, Vec<String>)>,
    parse_notes: Vec<String>,
}

impl Audit {
    fn new(bounds: &[(&'static str, i64)]) -> Self {
        let mut fields = BTreeMap::new();
        for (name, bound) in bounds {
            fields.insert(*name, (*bound, FieldStats::default()));
        }
        Audit {
            fields,
            bound_trip_units: BTreeMap::new(),
            probe_unit_names: BTreeMap::new(),
            records: BTreeMap::new(),
            data_records: BTreeMap::new(),
            record_types: BTreeMap::new(),
            unscanned: BTreeMap::new(),
            parse_notes: Vec::new(),
        }
    }

    fn record(&mut self, field: &'static str, value: i64) {
        if let Some((bound, stats)) = self.fields.get_mut(field) {
            let bound = *bound;
            stats.record(value, bound);
        }
    }
}

fn main() {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: bench_bound_audit <collector_stream.bin>");
            std::process::exit(2);
        }
    };
    let file = File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let mut reader = BufReader::with_capacity(1 << 22, file);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    eprintln!("stream: {path} ({} bytes)", bytes.len());

    // (wire field, the bound the producer/consumer applies to it)
    let mut audit = Audit::new(&[
        // d20 divisions
        ("d20.max_supply_raw / MAX_SUPPLY_RAW", 100_000_000),
        ("d20.current_supply_raw / MAX_SUPPLY_RAW", 100_000_000),
        ("d20.supply_gain_raw / MAX_SUPPLY_GAIN_RAW", 500_000),
        ("d20.location / d20 MAX_PROVINCE_ID", 1_000_000),
        ("d20.owner_tag_db_id / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        // d44 supply logistics
        ("d44.province.rail / MAX_BUILDING_LEVEL", 1_000),
        ("d44.province.supply_node / MAX_BUILDING_LEVEL", 1_000),
        ("d44.province.naval_hub / MAX_BUILDING_LEVEL", 1_000),
        ("d44.province.infrastructure / MAX_BUILDING_LEVEL", 1_000),
        ("d44.province.building_count / MAX_BUILDING_LEVEL", 1_000),
        ("d44.province.entity_id / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.division.join_province / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.division.current_supply / MAX_SUPPLY_RAW", 100_000_000),
        ("d44.division.max_supply / MAX_SUPPLY_RAW", 100_000_000),
        ("d44.division.supply_gain / MAX_SUPPLY_GAIN_RAW", 500_000),
        ("d44.division.owner / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        ("d44.csupply.entity_id / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.country_disrupted.value / MAX_SUPPLY_RAW", 100_000_000),
        ("d44.country_disrupted.province / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.country_disrupted.owner / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        ("d44.homebase_candidate.province / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.homebase_candidate.owner / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        ("d44.fhb1.province / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.fhb1.owner / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        ("d44.atr1.province / SUPPLY_MAX_PROVINCE_ID", 65_535),
        ("d44.atr1.owner / MAX_COMPACT_OWNER_TAG_DB_ID", 2_048),
        // d52 occupation
        ("d52.owner_tag_db_id / MAX_TAG_DB_ID", 1_024),
        ("d52.controller_tag_db_id / MAX_TAG_DB_ID", 1_024),
        ("d52.occupied_country_tag_db_id / MAX_TAG_DB_ID", 1_024),
        ("d52.collab.owner_tag_db_id / MAX_TAG_DB_ID", 1_024),
        ("d52.collab.target_tag_db_id / MAX_TAG_DB_ID", 1_024),
        ("d52.gar1.controller_tag_db_id / MAX_TAG_DB_ID", 1_024),
    ]);

    let mut offset = 0usize;
    while offset.saturating_add(HEADER_LEN) <= bytes.len() {
        if &bytes[offset..offset + 4] != MAGIC {
            audit
                .parse_notes
                .push(format!("offset {offset}: lost H4CR framing"));
            break;
        }
        let version = u16le(&bytes, offset + 4);
        let header_len = u16le(&bytes, offset + 6) as usize;
        let domain_id = u16le(&bytes, offset + 20);
        let record_type = u16le(&bytes, offset + 22);
        let payload_len = u32le(&bytes, offset + 24) as usize;
        let record_flags = u32le(&bytes, offset + 28);
        // Checked throughout: `payload_len` is a stream-supplied u32 and
        // `offset + header_len + payload_len` can wrap on a 32-bit build, which
        // would turn a hostile length into an in-bounds slice.
        let Some(record_end) = offset
            .checked_add(header_len)
            .and_then(|start| start.checked_add(payload_len))
        else {
            audit
                .parse_notes
                .push(format!("offset {offset}: record length overflows usize"));
            break;
        };
        if header_len < HEADER_LEN || record_end > bytes.len() {
            audit
                .parse_notes
                .push(format!("offset {offset}: truncated record"));
            break;
        }
        if version != STREAM_RECORD_VERSION {
            audit.parse_notes.push(format!(
                "offset {offset}: record version {version} is not {STREAM_RECORD_VERSION}"
            ));
            break;
        }
        let payload = &bytes[offset + header_len..record_end];
        *audit.record_types.entry(record_type).or_insert(0) += 1;
        *audit.records.entry(domain_id).or_insert(0) += 1;
        if record_type == RT_DOMAIN_DATA {
            *audit.data_records.entry(domain_id).or_insert(0) += 1;
        }

        // A scanner that cannot recognise a record's layout MUST say so. The
        // record has already been counted, so a silent `return` would let the
        // report present statistics from a subset of the records as if they
        // covered all of them — which is how this tool once reported "no bound
        // ever tripped" for a stream it had not looked in.
        let outcome = match record_type {
            RT_DOMAIN_PERF if domain_id == PROBE_DOMAIN_ID => {
                scan_probe_units(record_flags, payload, &mut audit)
            }
            RT_DOMAIN_DATA => match domain_id {
                D20 => scan_d20(payload, &mut audit),
                D44 => scan_d44(payload, &mut audit),
                D52 => scan_d52(payload, &mut audit),
                _ => Ok(()),
            },
            _ => Ok(()),
        };
        if let Err(reason) = outcome {
            let entry = audit.unscanned.entry(domain_id).or_default();
            entry.0 = entry.0.saturating_add(1);
            if entry.1.len() < 3 && !entry.1.contains(&reason) {
                entry.1.push(reason);
            }
        }
        offset = record_end;
    }

    report(&audit, offset, bytes.len());
}

/// Deletion marker on a d44 identity field (`domain44_contract::ROW_ID_DELETED`).
/// A tombstone row still carries its real id in the low bits; recording the
/// word unmasked reports a maximum around 0x80000000 for a field whose real
/// maximum is a province id.
const D44_ROW_ID_DELETED: u32 = 1 << 31;

const D52_COLLABORATION_MAGIC: &[u8; 4] = b"COL1";
const D52_COLLABORATION_HEADER_LEN: usize = 32;
const D52_COLLABORATION_ROW_LEN: usize = 16;
const D52_GARRISON_ALERT_MAGIC: &[u8; 4] = b"GAR1";
const D52_GARRISON_ALERT_HEADER_LEN: usize = 40;
const D52_GARRISON_ALERT_ROW_LEN: usize = 16;

fn scan_probe_units(record_flags: u32, payload: &[u8], audit: &mut Audit) -> Result<(), String> {
    if record_flags & PERF_DIAGNOSTIC_TRAILER_FLAG == 0 {
        // No trailer declared: nothing to read, and nothing missed.
        return Ok(());
    }
    let mut cursor = 40usize;
    if payload.len().saturating_sub(cursor) < 8 {
        return Err("perf diagnostic trailer shorter than its own header".into());
    }
    if &payload[cursor..cursor + 4] != PERF_DIAGNOSTIC_MAGIC {
        return Err("perf diagnostic trailer magic is not PDU1".into());
    }
    cursor += 4;
    let _version = u16le(payload, cursor);
    cursor += 2;
    let unit_count = u16le(payload, cursor) as usize;
    cursor += 2;
    for index in 0..unit_count {
        if payload.len().saturating_sub(cursor) < 12 {
            return Err(format!("perf diagnostic unit {index} header truncated"));
        }
        let name_len = u16le(payload, cursor) as usize;
        let value = u64le(payload, cursor + 4);
        cursor += 12;
        if payload.len().saturating_sub(cursor) < name_len {
            return Err(format!("perf diagnostic unit {index} name truncated"));
        }
        let name = String::from_utf8_lossy(&payload[cursor..cursor + name_len]).into_owned();
        cursor += name_len;
        if let Some(stem) = name.strip_prefix("bt_") {
            let entry = audit.bound_trip_units.entry(name.clone()).or_insert((0, 0));
            if stem.ends_with("_sample") {
                entry.1 = value;
            } else {
                entry.0 = entry.0.saturating_add(value);
            }
        }
        *audit.probe_unit_names.entry(name).or_insert(0) += 1;
    }
    Ok(())
}

fn scan_d20(payload: &[u8], audit: &mut Audit) -> Result<(), String> {
    if payload.len() < D20_HEADER_LEN {
        return Err("d20 payload shorter than its header".into());
    }
    let row_count = u32le(payload, 8) as usize;
    let extended_expected = D20_HEADER_LEN
        .checked_add(row_count.checked_mul(D20_ROW_LEN).ok_or("d20 row span overflows")?)
        .ok_or("d20 payload span overflows")?;
    let legacy_expected = D20_HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(D20_LEGACY_ROW_LEN)
                .ok_or("d20 legacy row span overflows")?,
        )
        .ok_or("d20 legacy payload span overflows")?;
    let row_len = if payload.len() == extended_expected {
        D20_ROW_LEN
    } else if payload.len() == legacy_expected {
        D20_LEGACY_ROW_LEN
    } else {
        return Err(format!(
            "d20 layout unrecognised: payload {} row_count {row_count}",
            payload.len()
        ));
    };
    let mut cursor = D20_HEADER_LEN;
    for _ in 0..row_count {
        if cursor + row_len > payload.len() {
            return Err("d20 row runs past the payload".into());
        }
        // d20's optional fields declare presence with a ROW FLAG, not with a
        // nonzero value. Treating zero as absence discarded 22,801 present
        // current-supply and 243,456 present supply-gain observations in t15 -
        // a real measurement of a division sitting at exactly zero supply.
        let flags = u32le(payload, cursor + 8);
        for (field, value, present) in [
            (
                "d20.location / d20 MAX_PROVINCE_ID",
                u32le(payload, cursor + 12),
                u32le(payload, cursor + 12) != 0,
            ),
            (
                "d20.owner_tag_db_id / MAX_COMPACT_OWNER_TAG_DB_ID",
                u32le(payload, cursor + 56),
                flags & D20_ROW_FLAG_OWNER != 0,
            ),
            (
                "d20.max_supply_raw / MAX_SUPPLY_RAW",
                u32le(payload, cursor + 68),
                flags & D20_ROW_FLAG_MAX_SUPPLY != 0,
            ),
            (
                "d20.current_supply_raw / MAX_SUPPLY_RAW",
                u32le(payload, cursor + 72),
                flags & D20_ROW_FLAG_CURRENT_SUPPLY != 0,
            ),
            (
                "d20.supply_gain_raw / MAX_SUPPLY_GAIN_RAW",
                u32le(payload, cursor + 76),
                flags & D20_ROW_FLAG_SUPPLY_GAIN != 0,
            ),
        ] {
            if present {
                audit.record(field, i64::from(value));
            }
        }
        cursor += row_len;
    }
    Ok(())
}

fn scan_d44(payload: &[u8], audit: &mut Audit) -> Result<(), String> {
    if payload.len() < D44_HEADER_LEN_LEGACY {
        return Err("d44 payload shorter than its legacy header".into());
    }
    let row_count = u32le(payload, 8) as usize;
    // The payload carries optional trailing sections after the rows, so the
    // header length is settled by the header's own row counters rather than by
    // dividing the remaining bytes.
    let sum_without_pressure = (u32le(payload, 24) as usize)
        .saturating_add(u32le(payload, 28) as usize)
        .saturating_add(u32le(payload, 32) as usize)
        .saturating_add(u32le(payload, 44) as usize);
    let header_len = if payload.len() >= D44_HEADER_LEN
        && sum_without_pressure.saturating_add(u32le(payload, 48) as usize) == row_count
    {
        D44_HEADER_LEN
    } else if sum_without_pressure == row_count {
        D44_HEADER_LEN_LEGACY
    } else {
        return Err(format!(
            "d44 header length undecidable: payload {} row_count {row_count}",
            payload.len()
        ));
    };
    let rows_end = header_len
        .checked_add(row_count.checked_mul(D44_ROW_LEN).ok_or("d44 row span overflows")?)
        .ok_or("d44 payload span overflows")?;
    if payload.len() < rows_end {
        return Err(format!(
            "d44 rows run past the payload: payload {} row_count {row_count}",
            payload.len()
        ));
    }
    let mut cursor = header_len;
    for _ in 0..row_count {
        let row_kind = u32le(payload, cursor);
        let entity_raw = u32le(payload, cursor + 4);
        let tombstone = entity_raw & D44_ROW_ID_DELETED != 0;
        let entity_id = entity_raw & !D44_ROW_ID_DELETED;
        let join_id = u32le(payload, cursor + 8);
        let owner = u32le(payload, cursor + 12);
        let a = i32le(payload, cursor + 20);
        let b = i32le(payload, cursor + 24);
        let c = i32le(payload, cursor + 28);
        let d = i32le(payload, cursor + 32);
        let aux = i32le(payload, cursor + 36);
        cursor += D44_ROW_LEN;
        // A tombstone carries an identity and nothing else: its scalar fields
        // are required to be zero, so recording them would add fake zero
        // observations to the value distributions.
        match row_kind {
            // Province logistics.
            1 => {
                audit.record("d44.province.entity_id / SUPPLY_MAX_PROVINCE_ID", i64::from(entity_id));
                if !tombstone {
                    audit.record("d44.province.rail / MAX_BUILDING_LEVEL", i64::from(a));
                    audit.record("d44.province.supply_node / MAX_BUILDING_LEVEL", i64::from(b));
                    audit.record("d44.province.naval_hub / MAX_BUILDING_LEVEL", i64::from(c));
                    audit.record("d44.province.infrastructure / MAX_BUILDING_LEVEL", i64::from(d));
                    audit.record("d44.province.building_count / MAX_BUILDING_LEVEL", i64::from(aux));
                }
            }
            // Division supply.
            2 => {
                if !tombstone {
                    // A tombstone's join/owner/scalars are all required to be
                    // zero, so recording them adds fake zero observations: nine
                    // division tombstones in t15 made the join-province count
                    // 912,112 against 912,103 live division rows.
                    audit.record(
                        "d44.division.join_province / SUPPLY_MAX_PROVINCE_ID",
                        i64::from(join_id),
                    );
                    if owner != 0 {
                        audit.record(
                            "d44.division.owner / MAX_COMPACT_OWNER_TAG_DB_ID",
                            i64::from(owner),
                        );
                    }
                    audit.record("d44.division.current_supply / MAX_SUPPLY_RAW", i64::from(a));
                    audit.record("d44.division.max_supply / MAX_SUPPLY_RAW", i64::from(b));
                    audit.record("d44.division.supply_gain / MAX_SUPPLY_GAIN_RAW", i64::from(c));
                }
            }
            // CSupply province input.
            4 => {
                audit.record("d44.csupply.entity_id / SUPPLY_MAX_PROVINCE_ID", i64::from(entity_id));
            }
            // Country disrupted supply. Its province id is the field the
            // collector's own province-id ceiling guards, and it is the kind
            // that actually uses the deletion marker.
            5 => {
                audit.record(
                    "d44.country_disrupted.province / SUPPLY_MAX_PROVINCE_ID",
                    i64::from(entity_id),
                );
                if owner != 0 {
                    audit.record(
                        "d44.country_disrupted.owner / MAX_COMPACT_OWNER_TAG_DB_ID",
                        i64::from(owner),
                    );
                }
                if !tombstone {
                    audit.record("d44.country_disrupted.value / MAX_SUPPLY_RAW", i64::from(a));
                }
            }
            // Foreign-homebase pressure candidate: also a province id under the
            // same ceiling, reached through the FHB read path.
            6 => {
                audit.record(
                    "d44.homebase_candidate.province / SUPPLY_MAX_PROVINCE_ID",
                    i64::from(entity_id),
                );
                if owner != 0 {
                    audit.record(
                        "d44.homebase_candidate.owner / MAX_COMPACT_OWNER_TAG_DB_ID",
                        i64::from(owner),
                    );
                }
            }
            // Kind 3 is a computed badge and carries no bounded identity.
            3 => {}
            other => {
                return Err(format!("d44 row kind {other} is not in this tool's vocabulary"));
            }
        }
    }
    // Trailing sections. FHB1 and ATR1 carry province and owner-tag fields
    // guarded by the very ceiling this audit is about - t15 holds 12,071 FHB1
    // rows and 641,975 ATR1 rows - so stopping at the base rows reported "every
    // record decoded in full" while auditing none of them.
    scan_d44_trailers(&payload[rows_end..], audit)
}

/// Additive D44 trailer chain: optional FHB1, optional ATR1, then optional
/// ARC1/PST1 whose codecs live in the lane crate. Sections are positional, so
/// the walk is a fixed sequence rather than a magic-dispatch loop.
fn scan_d44_trailers(trailer: &[u8], audit: &mut Audit) -> Result<(), String> {
    let mut cursor = 0usize;
    if trailer.len().saturating_sub(cursor) >= 4 && &trailer[cursor..cursor + 4] == b"FHB1" {
        if trailer.len().saturating_sub(cursor) < 16 {
            return Err("d44 FHB1 header truncated".into());
        }
        if u32le(trailer, cursor + 4) != 1 || u32le(trailer, cursor + 12) != 0 {
            return Err("d44 FHB1 header version/reserved invalid".into());
        }
        let row_count = u32le(trailer, cursor + 8) as usize;
        cursor += 16;
        for index in 0..row_count {
            // Fixed 80-byte row plus a variable-length source string whose
            // length is the last word of the fixed part.
            if trailer.len().saturating_sub(cursor) < 80 {
                return Err(format!("d44 FHB1 row {index} truncated"));
            }
            let owner = u32le(trailer, cursor);
            let province = u32le(trailer, cursor + 4);
            let source_len = u32le(trailer, cursor + 76) as usize;
            // Unlike ATR1 and the base division rows, an FHB1 tombstone KEEPS
            // its owner and province: they are its identity, and the reference
            // decoder range-checks them before it looks at the deletion bit.
            // Skipping them dropped six bounded identities in t15 while the
            // record still reported as fully audited.
            audit.record(
                "d44.fhb1.province / SUPPLY_MAX_PROVINCE_ID",
                i64::from(province),
            );
            audit.record(
                "d44.fhb1.owner / MAX_COMPACT_OWNER_TAG_DB_ID",
                i64::from(owner),
            );
            cursor = cursor
                .checked_add(80)
                .and_then(|c| c.checked_add(source_len))
                .ok_or("d44 FHB1 row span overflows")?;
            if cursor > trailer.len() {
                return Err(format!("d44 FHB1 row {index} source runs past the payload"));
            }
        }
    }
    if trailer.len().saturating_sub(cursor) >= 4 && &trailer[cursor..cursor + 4] == b"ATR1" {
        if trailer.len().saturating_sub(cursor) < 16 {
            return Err("d44 ATR1 header truncated".into());
        }
        if u32le(trailer, cursor + 4) != 1 || u32le(trailer, cursor + 12) != 0 {
            return Err("d44 ATR1 header version/reserved invalid".into());
        }
        let row_count = u32le(trailer, cursor + 8) as usize;
        cursor += 16;
        for index in 0..row_count {
            if trailer.len().saturating_sub(cursor) < 64 {
                return Err(format!("d44 ATR1 row {index} truncated"));
            }
            let province = u32le(trailer, cursor + 4);
            let owner = u32le(trailer, cursor + 8);
            let deleted = u32le(trailer, cursor + 12) & 1 != 0;
            if !deleted {
                audit.record(
                    "d44.atr1.province / SUPPLY_MAX_PROVINCE_ID",
                    i64::from(province),
                );
                audit.record(
                    "d44.atr1.owner / MAX_COMPACT_OWNER_TAG_DB_ID",
                    i64::from(owner),
                );
            }
            cursor += 64;
        }
    }
    if cursor < trailer.len() {
        // ARC1/PST1 and anything else: their codecs are not reimplemented here.
        // Saying so is the point - the alternative is reporting the record as
        // fully audited when part of it was never read.
        let magic = trailer
            .get(cursor..cursor + 4)
            .map(|m| String::from_utf8_lossy(m).into_owned())
            .unwrap_or_else(|| "<short>".to_string());
        return Err(format!(
            "d44 trailing section {magic:?} is not in this tool's vocabulary"
        ));
    }
    Ok(())
}

fn scan_d52(payload: &[u8], audit: &mut Audit) -> Result<(), String> {
    if payload.len() < D52_HEADER_LEN {
        return Err("d52 payload shorter than its header".into());
    }
    let row_count = u32le(payload, 8) as usize;
    let rows_end = D52_HEADER_LEN
        .checked_add(row_count.checked_mul(D52_ROW_LEN).ok_or("d52 row span overflows")?)
        .ok_or("d52 payload span overflows")?;
    if payload.len() < rows_end {
        return Err(format!(
            "d52 rows run past the payload: payload {} row_count {row_count}",
            payload.len()
        ));
    }
    let mut cursor = D52_HEADER_LEN;
    for _ in 0..row_count {
        for (field, value) in [
            ("d52.owner_tag_db_id / MAX_TAG_DB_ID", u32le(payload, cursor + 4)),
            (
                "d52.controller_tag_db_id / MAX_TAG_DB_ID",
                u32le(payload, cursor + 8),
            ),
            (
                "d52.occupied_country_tag_db_id / MAX_TAG_DB_ID",
                u32le(payload, cursor + 12),
            ),
        ] {
            if value != 0 {
                audit.record(field, i64::from(value));
            }
        }
        cursor += D52_ROW_LEN;
    }
    // Trailing extension sections. The collaboration target is one of the
    // fields whose refusal this change re-attributed, so omitting it would
    // under-report exactly what the audit is for.
    scan_d52_trailers(&payload[rows_end..], audit)
}

/// D52's trailer grammar is positional and fixed: at most one GAR1, then at
/// most one COL1. A magic-only dispatch loop accepts wrong schema versions,
/// COL1 without its preceding GAR1, and repeated or reordered sections, and
/// then reports the record fully audited.
fn scan_d52_trailers(trailer: &[u8], audit: &mut Audit) -> Result<(), String> {
    let mut cursor = 0usize;
    let mut garrison_consumed = false;
    if trailer.len().saturating_sub(cursor) >= 4
        && &trailer[cursor..cursor + 4] == D52_GARRISON_ALERT_MAGIC
    {
        if trailer.len().saturating_sub(cursor) < D52_GARRISON_ALERT_HEADER_LEN {
            return Err("d52 GAR1 section shorter than its header".into());
        }
        if u16le(trailer, cursor + 4) != 1
            || u16le(trailer, cursor + 6) as usize != D52_GARRISON_ALERT_HEADER_LEN
        {
            return Err("d52 GAR1 schema version / header length invalid".into());
        }
        // GAR1 header: magic(4) version(2) header_len(2) game_hour(4)
        // controller(4) flags(4) missing_manpower(i64, 8) count(4) scale(4)
        // reserved(4) = 40. The count is at 28, NOT at 12 where COL1 keeps its
        // own; reading 12 as the count skipped 234 records in t15.
        let controller = u32le(trailer, cursor + 12);
        if controller != 0 {
            audit.record(
                "d52.gar1.controller_tag_db_id / MAX_TAG_DB_ID",
                i64::from(controller),
            );
        }
        let row_count = u32le(trailer, cursor + 28) as usize;
        let section_len = D52_GARRISON_ALERT_HEADER_LEN
            .checked_add(
                row_count
                    .checked_mul(D52_GARRISON_ALERT_ROW_LEN)
                    .ok_or("d52 GAR1 row span overflows")?,
            )
            .ok_or("d52 GAR1 section span overflows")?;
        if trailer.len().saturating_sub(cursor) < section_len {
            return Err("d52 GAR1 rows run past the payload".into());
        }
        cursor += section_len;
        garrison_consumed = true;
    }
    // COL1 is reachable only AFTER a consumed GAR1. Testing it independently
    // accepts a COL1-only trailer, which the reference decoder never produces
    // and never reads - and then reports the record fully audited.
    if garrison_consumed
        && trailer.len().saturating_sub(cursor) >= 4
        && &trailer[cursor..cursor + 4] == D52_COLLABORATION_MAGIC
    {
        if trailer.len().saturating_sub(cursor) < D52_COLLABORATION_HEADER_LEN {
            return Err("d52 COL1 section shorter than its header".into());
        }
        if u16le(trailer, cursor + 4) != 1
            || u16le(trailer, cursor + 6) as usize != D52_COLLABORATION_HEADER_LEN
        {
            return Err("d52 COL1 schema version / header length invalid".into());
        }
        // COL1 header: magic(4) version(2) header_len(2) flags(4) count(4)
        // observed(4) anomalies(4) scale(4) reserved(4) = 32.
        let row_count = u32le(trailer, cursor + 12) as usize;
        let section_len = D52_COLLABORATION_HEADER_LEN
            .checked_add(
                row_count
                    .checked_mul(D52_COLLABORATION_ROW_LEN)
                    .ok_or("d52 COL1 row span overflows")?,
            )
            .ok_or("d52 COL1 section span overflows")?;
        if trailer.len().saturating_sub(cursor) < section_len {
            return Err("d52 COL1 rows run past the payload".into());
        }
        for index in 0..row_count {
            let at = cursor + D52_COLLABORATION_HEADER_LEN + index * D52_COLLABORATION_ROW_LEN;
            for (field, value) in [
                ("d52.collab.owner_tag_db_id / MAX_TAG_DB_ID", u32le(trailer, at)),
                (
                    "d52.collab.target_tag_db_id / MAX_TAG_DB_ID",
                    u32le(trailer, at + 4),
                ),
            ] {
                if value != 0 {
                    audit.record(field, i64::from(value));
                }
            }
        }
        cursor += section_len;
    }
    if cursor < trailer.len() {
        let magic = trailer
            .get(cursor..cursor + 4)
            .map(|m| String::from_utf8_lossy(m).into_owned())
            .unwrap_or_else(|| "<short>".to_string());
        return Err(format!(
            "d52 trailing section {magic:?} is outside the GAR1-then-COL1 grammar"
        ));
    }
    Ok(())
}

fn report(audit: &Audit, consumed: usize, total: usize) {
    println!("== framing ==");
    println!("consumed {consumed} / {total} bytes");
    for note in &audit.parse_notes {
        println!("NOTE {note}");
    }
    println!("record types: {:?}", audit.record_types);
    println!("domains present (any record type): {} distinct", audit.records.len());
    for probe in [D20, D44, D52] {
        println!(
            "  domain {probe}: {} record(s) total, {} DOMAIN_DATA record(s)",
            audit.records.get(&probe).copied().unwrap_or(0),
            audit.data_records.get(&probe).copied().unwrap_or(0)
        );
    }

    println!();
    println!("== bound-trip diagnostic units observed on probe domain 250 ==");
    if audit.bound_trip_units.is_empty() {
        println!("(none: no bt_* unit was emitted anywhere in this capture)");
    }
    for (name, (count, sample)) in &audit.bound_trip_units {
        println!("{name:>44}  summed={count:<12} last_raw_sample={sample}");
    }
    // Positive control for the negative above. `so_*` status-origin units are
    // emitted by the SAME collector call, in the same trailer, with the same
    // (name, value) shape as `bt_*`. Seeing them proves this decoder can read
    // that trailer, so "no bt_* unit" is a measurement rather than a decoder
    // that silently read nothing.
    println!(
        "probe-domain unit vocabulary seen: {} distinct ({} of them so_* status origins)",
        audit.probe_unit_names.len(),
        audit
            .probe_unit_names
            .keys()
            .filter(|name| name.starts_with("so_"))
            .count()
    );
    let sample: Vec<&str> = audit
        .probe_unit_names
        .keys()
        .map(String::as_str)
        .take(10)
        .collect();
    println!("first probe unit names: {sample:?}");

    println!();
    println!("== records this tool could NOT scan ==");
    if audit.unscanned.is_empty() {
        println!("(none: every record of every audited domain was decoded in full)");
    }
    for (domain, (count, reasons)) in &audit.unscanned {
        println!("  domain {domain}: {count} record(s) skipped");
        for reason in reasons {
            println!("      {reason}");
        }
    }
    let incomplete: Vec<u16> = audit.unscanned.keys().copied().collect();
    if !incomplete.is_empty() {
        println!(
            "WARNING: the field table below covers a SUBSET of domains {incomplete:?} -              treat their maxima as lower bounds, not measurements"
        );
    }

    println!();
    println!("== bounded wire fields, observed vs bound ==");
    println!(
        "{:<52} {:>12} {:>12} {:>14} {:>14} {:>10} {:>8}",
        "field / bound", "bound", "observed", "max", "2nd max", ">=bound", "headroom"
    );
    for (name, (bound, stats)) in &audit.fields {
        let Some(max) = stats.max else {
            println!(
                "{name:<52} {bound:>12} {:>12} {:>14} {:>14} {:>10} {:>8}",
                0, "-", "-", "-", "-"
            );
            continue;
        };
        let headroom = if max > 0 {
            format!("{:.0}x", *bound as f64 / max as f64)
        } else {
            "inf".to_string()
        };
        let second = stats
            .second_max
            .map_or_else(|| "-".to_string(), |value| value.to_string());
        println!(
            "{name:<52} {bound:>12} {:>12} {max:>14} {second:>14} {:>10} {headroom:>8}",
            stats.observed, stats.at_or_over_bound
        );
    }
}
