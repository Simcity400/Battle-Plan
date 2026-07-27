//! Measure the wire format against the shipping compact stream on REAL
//! division dynamics — the measurement §13 says is missing.
//!
//! Input: a real `collector_stream.bin` from a d20-scheduled capture
//! (`runs/crashrate/minimal_t*`). The stream's own d20 DATA records are the
//! shipping format's cost for the domain: baseline + row-granular deltas,
//! 120-byte rows, exactly what the recorder stores today. This tool replays
//! those records into per-hour division states — the same fold
//! `world_fold::fold_divisions` performs — and re-encodes every hour with this
//! crate's grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **The whole stream is validated before any artifact is written.** Framing
//! and record semantics go through `compact_contract`'s parser and
//! `StatefulStreamValidator`. The bin re-implements the wire-format crate's
//! documented subset of `recorder_host::stream_fold::BoundaryTracker`
//! (`stream_fold.rs:277`-`:482`): DATA/PERF hour binding, monotonic closed
//! hours, one d20 record per hour, and clean FRAME_END/EOF boundaries.
//!
//! The five-check sweep additionally carries:
//!
//! - FRAME_END seal-field validation from
//!   `validate_frame_end_seal_fields` (`stream_fold.rs:46`-`:89`), before the
//!   open-hour match, plus the seal's distinct non-probe PERF-domain count
//!   cross-check (`:401`-`:419`);
//! - DOMAIN_PERF hour binding (`:328`-`:365`) and the DATA arm's pending-PERF
//!   rule (`:378`-`:386`);
//! - the status-independent d20 payload authority, run before failure-status
//!   exclusion. `decode_compact_divisions_data_payload`
//!   (`domain_data_authority.rs:4517`-`:5132`) checks the 32-byte header,
//!   scale/reserved/count relationships and exact legacy-or-extended length,
//!   then walks every row: identity/type/uniqueness, legacy/A15 compatibility,
//!   delete zero-fill, UI implication flags, province/movement/combat
//!   relationships, optional-field ranges and zero-packing, and order-link
//!   consistency. Unlike d63, `wire_format::d20` exposes no compact-payload
//!   boundary helpers, so those authority predicates are mirrored here; values
//!   already owned by `compact_contract` are called from that crate. The
//!   authority itself owns no additional fold-only frame rule: delta-before-
//!   baseline remains `world_fold::fold_divisions` behavior
//!   (`world_fold.rs:19022`-`:19029`) and is not invented as payload corruption;
//! - identity mode as a stated `StreamIdentityMode` parameter, matching
//!   production's caller-selected protocol (`stream_fold.rs:220`), never a
//!   sniff of the first record.
//!
//! Production also validates payloads for every *other* DOMAIN_DATA domain.
//! This single-domain measurement cannot do that without making `wire_format`
//! depend on `recorder_host`, so that is the one documented boundary-contract
//! check outside this harness's subset.
//!
//! Outputs, for the compression step (`scripts/bench_real_dynamics.py`):
//!   ship_d20.bin   — the d20 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision), which is the number the §13 layout model had to assume.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_STATUS_BRIDGE_UNAVAILABLE, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};

use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, key, KeyedState,
};
use wire_format::d20;
use wire_format::record::Record;
use wire_format::value::Value;

const D20: u16 = 20;

// Division table payload (world_fold.rs fold_divisions).
const TABLE_HEADER_LEN: usize = 32;
const ROW_LEN: usize = 120;
const LEGACY_ROW_LEN: usize = 104;
const FIXED_SCALE_RAW: u32 = 100_000;
const FIXED_SCALE: f64 = 100_000.0;
const DIVISION_TYPE: u32 = 4713;
const MAX_PROVINCE_ID: u32 = 1_000_000;
const MAX_COMBAT_REF_COUNT: u32 = 1_000;
const MAX_ORGANISATION_RAW: u32 = 100_000_000;
const MAX_STRENGTH_RAW: u32 = 1_000_000_000;
const MAX_SUPPLY_RAW: u32 = 100_000_000;
const MAX_SUPPLY_GAIN_RAW: u32 = 500_000;
const MAX_COUNTER_SUPPLY_STATUS_RAW: u32 = 100_000;
const MAX_DIG_IN_RAW: u32 = 100_000_000;
const MAX_DISRUPTED_SUPPLY_RAW: u32 = 1_000_000;
const MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
const MAX_CURRENT_MANPOWER: u32 = 10_000_000;
const MAX_CUMULATIVE_LOSSES: u32 = 100_000_000;

// Row flag bits (world_fold.rs).
const F_MOVING: u32 = 1 << 0;
const F_HAS_PROGRESS: u32 = 1 << 1;
const F_HAS_TARGET: u32 = 1 << 2;
const F_HAS_SOURCE: u32 = 1 << 3;
const F_IN_COMBAT: u32 = 1 << 4;
const F_HAS_OWNER: u32 = 1 << 5;
const F_HAS_ARMY: u32 = 1 << 6;
const F_HAS_EXPERIENCE: u32 = 1 << 7;
const F_HAS_MAX_SUPPLY: u32 = 1 << 8;
const F_HAS_CURRENT_SUPPLY: u32 = 1 << 9;
const F_HAS_SUPPLY_GAIN: u32 = 1 << 10;
const F_HAS_FUEL: u32 = 1 << 11;
const F_DELETED: u32 = 1 << 12;
const F_HAS_TEMPLATE: u32 = 1 << 13;
const F_ORDER_ARMY: u32 = 1 << 14;
const F_ORDER_ARMY_GROUP: u32 = 1 << 15;
const F_ORDER_UNRESOLVED: u32 = 1 << 16;
const F_HAS_DISRUPTED_SUPPLY: u32 = 1 << 17;
const F_HAS_STRATEGIC_REDEPLOYMENT: u32 = 1 << 18;
const F_HAS_CURRENT_MANPOWER: u32 = 1 << 19;
const F_HAS_CUMULATIVE_LOSSES: u32 = 1 << 20;
const F_HAS_MAX_ORGANISATION: u32 = 1 << 21;
const F_HAS_COUNTER_SUPPLY_STATUS: u32 = 1 << 22;
const F_HAS_DIG_IN: u32 = 1 << 23;
const F_HAS_DIG_IN_CAP: u32 = 1 << 24;
const F_TRAINING_OBSERVED: u32 = 1 << 25;
const F_TRAINING_VISIBLE: u32 = 1 << 26;
const F_DIG_IN_UI_OBSERVED: u32 = 1 << 27;
const F_DIG_IN_UI_VISIBLE: u32 = 1 << 28;
const F_DIG_IN_UI_CAPPED: u32 = 1 << 29;
const F_INVALID_ORDERS_OBSERVED: u32 = 1 << 30;
const F_INVALID_ORDERS_VISIBLE: u32 = 1 << 31;

const BLOCK_HOURS: usize = 48;
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;
const PROBE_DOMAIN_ID: u16 = 250;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[derive(Clone)]
struct ShipRow {
    bytes: [u8; ROW_LEN],
}

impl ShipRow {
    fn flags(&self) -> u32 {
        u32_at(&self.bytes, 8)
    }
    fn fixed(&self, off: usize) -> f64 {
        u32_at(&self.bytes, off) as f64 / FIXED_SCALE
    }
    fn opt_fixed(&self, flag: u32, off: usize) -> Value {
        if self.flags() & flag != 0 {
            Value::Continuous(self.fixed(off))
        } else {
            Value::Null
        }
    }
    fn opt_unsigned(&self, flag: u32, off: usize) -> Value {
        if self.flags() & flag != 0 {
            Value::Unsigned(u32_at(&self.bytes, off) as u64)
        } else {
            Value::Null
        }
    }

    /// The row's stored form in the new grammar, field for field. Presence
    /// flags map to the schema's four states the way `fold_divisions` reads
    /// them: flag clear on an optional field is Null; movement fields are
    /// Absent (not applicable) on a stationary division.
    fn to_record(&self, collection: &wire_format::schema::Collection) -> Record {
        let flags = self.flags();
        let moving = flags & F_MOVING != 0;
        let values: Vec<Value> = collection
            .value_fields()
            .map(|f| match f.field_id {
                d20::field::OWNER => {
                    if flags & F_HAS_OWNER != 0 {
                        Value::ContentId(u32_at(&self.bytes, 56) as u64)
                    } else {
                        Value::Null
                    }
                }
                d20::field::LOCATION_PROVINCE => Value::ContentId(u32_at(&self.bytes, 12) as u64),
                d20::field::TEMPLATE_ID => self.opt_unsigned(F_HAS_TEMPLATE, 52),
                d20::field::ORGANISATION => Value::Continuous(self.fixed(32)),
                d20::field::STRENGTH => Value::Continuous(self.fixed(40)),
                d20::field::MAX_ORGANISATION => self.opt_fixed(F_HAS_MAX_ORGANISATION, 104),
                d20::field::MOVING => Value::Bool(moving),
                d20::field::MOVE_TARGET_PROVINCE => {
                    if flags & F_HAS_TARGET != 0 {
                        Value::ContentId(u32_at(&self.bytes, 16) as u64)
                    } else {
                        Value::Absent
                    }
                }
                d20::field::MOVE_PROGRESS => {
                    if moving {
                        Value::Continuous(self.fixed(24))
                    } else {
                        Value::Absent
                    }
                }
                d20::field::IN_COMBAT => Value::Bool(flags & F_IN_COMBAT != 0),
                d20::field::EXPERIENCE => self.opt_fixed(F_HAS_EXPERIENCE, 64),
                d20::field::CURRENT_SUPPLY => self.opt_fixed(F_HAS_CURRENT_SUPPLY, 72),
                d20::field::MAX_SUPPLY => self.opt_fixed(F_HAS_MAX_SUPPLY, 68),
                d20::field::SUPPLY_GAIN => self.opt_fixed(F_HAS_SUPPLY_GAIN, 76),
                d20::field::FUEL => self.opt_fixed(F_HAS_FUEL, 80),
                d20::field::DIG_IN => self.opt_fixed(F_HAS_DIG_IN, 112),
                d20::field::DIG_IN_CAP => self.opt_fixed(F_HAS_DIG_IN_CAP, 116),
                d20::field::DISRUPTED_SUPPLY => self.opt_fixed(F_HAS_DISRUPTED_SUPPLY, 88),
                d20::field::STRATEGIC_REDEPLOYMENT => {
                    if flags & F_HAS_STRATEGIC_REDEPLOYMENT != 0 {
                        Value::Bool(u32_at(&self.bytes, 92) != 0)
                    } else {
                        Value::Null
                    }
                }
                d20::field::CURRENT_MANPOWER => self.opt_unsigned(F_HAS_CURRENT_MANPOWER, 96),
                d20::field::CUMULATIVE_LOSSES => self.opt_unsigned(F_HAS_CUMULATIVE_LOSSES, 100),
                d20::field::ARMY_ID => self.opt_unsigned(F_HAS_ARMY, 60),
                d20::field::ORDER_INSTANCE_ID => self.opt_unsigned(F_ORDER_ARMY, 84),
                d20::field::ORDER_LINK => {
                    if flags & F_ORDER_ARMY != 0 {
                        Value::Enum(0)
                    } else if flags & F_ORDER_ARMY_GROUP != 0 {
                        Value::Enum(1)
                    } else if flags & F_ORDER_UNRESOLVED != 0 {
                        Value::Enum(2)
                    } else {
                        Value::Null
                    }
                }
                _ => Value::Null,
            })
            .collect();
        Record::new(values)
    }
}

/// One d20 DATA record kept by the pass-1 walk: its game hour, whether the
/// record is an incremental update, its rows, and its byte span in the stream.
type ScannedHour = (i32, bool, Vec<ShipRow>, Range<usize>);

struct StreamScan {
    hours: Vec<ScannedHour>,
    failure_records: usize,
}

#[derive(Clone, Copy)]
struct DivisionPayloadHeader {
    row_count: usize,
    changed_rows: usize,
    status_flags: u32,
    scale_raw: u32,
    root_rows: usize,
    reserved: u32,
    row_len: usize,
}

/// Split the compact divisions payload at the boundary production uses before
/// its status early exit (`world_fold.rs:18997`-`:19014`). `wire_format::d20`
/// exposes the target schema, but no compact-payload parser.
fn split_division_payload(
    payload: &[u8],
    at_byte: usize,
) -> Result<(DivisionPayloadHeader, &[u8]), String> {
    if payload.len() < TABLE_HEADER_LEN {
        return Err(format!(
            "d20 payload at byte {at_byte} is {} byte(s), expected at least {TABLE_HEADER_LEN}",
            payload.len()
        ));
    }
    let row_count = u32_at(payload, 8) as usize;
    let legacy_expected = TABLE_HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(LEGACY_ROW_LEN)
                .ok_or_else(|| format!("d20 payload at byte {at_byte}: legacy length overflow"))?,
        )
        .ok_or_else(|| format!("d20 payload at byte {at_byte}: legacy length overflow"))?;
    let extended_expected =
        TABLE_HEADER_LEN
            .checked_add(row_count.checked_mul(ROW_LEN).ok_or_else(|| {
                format!("d20 payload at byte {at_byte}: extended length overflow")
            })?)
            .ok_or_else(|| format!("d20 payload at byte {at_byte}: extended length overflow"))?;
    let row_len = if payload.len() == extended_expected {
        ROW_LEN
    } else if payload.len() == legacy_expected {
        LEGACY_ROW_LEN
    } else {
        return Err(format!(
            "d20 payload at byte {at_byte} is {} byte(s), expected legacy \
             {legacy_expected} or extended {extended_expected}",
            payload.len()
        ));
    };
    Ok((
        DivisionPayloadHeader {
            row_count,
            changed_rows: u32_at(payload, 12) as usize,
            status_flags: u32_at(payload, 16),
            scale_raw: u32_at(payload, 20),
            root_rows: u32_at(payload, 24) as usize,
            reserved: u32_at(payload, 28),
            row_len,
        },
        &payload[TABLE_HEADER_LEN..],
    ))
}

fn check_optional_u32(
    present: bool,
    value: u32,
    min: u32,
    max: u32,
    name: &str,
    row_index: usize,
    at_byte: usize,
) -> Result<(), String> {
    if present {
        if value < min || value > max {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: {name} {value} is outside \
                 {min}..={max}"
            ));
        }
    } else if value != 0 {
        return Err(format!(
            "d20 row {row_index} at byte {at_byte}: {name} is {value} without its flag"
        ));
    }
    Ok(())
}

/// Replay d20's production payload authority
/// (`domain_data_authority.rs:4517`-`:5132`) over every record, including
/// failure-status records. The minimum/exact-length split already ran above;
/// all remaining authority predicates live here.
fn check_authority_payload(
    header: DivisionPayloadHeader,
    body: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "d20 payload at byte {at_byte}: scale {} != expected {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    if header.reserved != 0 {
        return Err(format!(
            "d20 payload at byte {at_byte}: reserved field is nonzero"
        ));
    }
    if header.changed_rows > header.row_count {
        return Err(format!(
            "d20 payload at byte {at_byte}: changed_rows {} > row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    let delta = header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
    if delta && header.changed_rows != header.row_count {
        return Err(format!(
            "d20 payload at byte {at_byte}: delta changed_rows {} != row_count {}",
            header.changed_rows, header.row_count
        ));
    }
    if !delta && header.row_count > header.root_rows {
        return Err(format!(
            "d20 payload at byte {at_byte}: row_count {} > root_rows {}",
            header.row_count, header.root_rows
        ));
    }
    if header.status_flags & COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0 && header.root_rows == 0 {
        return Err(format!(
            "d20 payload at byte {at_byte}: root source unavailable"
        ));
    }

    let extended = header.row_len == ROW_LEN;
    let a15_flags = F_HAS_MAX_ORGANISATION
        | F_HAS_COUNTER_SUPPLY_STATUS
        | F_HAS_DIG_IN
        | F_HAS_DIG_IN_CAP
        | F_TRAINING_OBSERVED
        | F_TRAINING_VISIBLE
        | F_DIG_IN_UI_OBSERVED
        | F_DIG_IN_UI_VISIBLE
        | F_DIG_IN_UI_CAPPED
        | F_INVALID_ORDERS_OBSERVED
        | F_INVALID_ORDERS_VISIBLE;
    let mut seen_divisions = BTreeSet::new();
    for (row_index, row) in body.chunks_exact(header.row_len).enumerate() {
        let div_id = u32_at(row, 0);
        let div_type = u32_at(row, 4);
        let flags = u32_at(row, 8);
        if div_id == 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: zero div_id"
            ));
        }
        if !seen_divisions.insert(div_id) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: duplicate div_id {div_id}"
            ));
        }
        if div_type != DIVISION_TYPE {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: div_type {div_type} != {DIVISION_TYPE}"
            ));
        }
        // The authority's explicit flag allow-list currently assigns every one
        // of the 32 bits (`domain_data_authority.rs:4655`-`:4677`), so there is
        // no unassigned row-flag bit to reject.
        if !extended && flags & a15_flags != 0 {
            return Err(format!(
                "d20 legacy row {row_index} at byte {at_byte} carries extended A15 flags"
            ));
        }

        let deleted = flags & F_DELETED != 0;
        if deleted {
            if flags != F_DELETED {
                return Err(format!(
                    "d20 delete row {row_index} at byte {at_byte} has non-delete flags"
                ));
            }
            if row[12..].iter().any(|byte| *byte != 0) {
                return Err(format!(
                    "d20 delete row {row_index} at byte {at_byte} is not zero-filled"
                ));
            }
            continue;
        }

        let moving = flags & F_MOVING != 0;
        let has_progress = flags & F_HAS_PROGRESS != 0;
        let has_target = flags & F_HAS_TARGET != 0;
        let has_source = flags & F_HAS_SOURCE != 0;
        let in_combat = flags & F_IN_COMBAT != 0;
        let has_template = flags & F_HAS_TEMPLATE != 0;
        let has_owner = flags & F_HAS_OWNER != 0;
        let has_army = flags & F_HAS_ARMY != 0;
        let has_experience = flags & F_HAS_EXPERIENCE != 0;
        let has_max_supply = flags & F_HAS_MAX_SUPPLY != 0;
        let has_current_supply = flags & F_HAS_CURRENT_SUPPLY != 0;
        let has_supply_gain = flags & F_HAS_SUPPLY_GAIN != 0;
        let has_fuel = flags & F_HAS_FUEL != 0;
        let has_disrupted_supply = flags & F_HAS_DISRUPTED_SUPPLY != 0;
        let has_strategic_redeployment = flags & F_HAS_STRATEGIC_REDEPLOYMENT != 0;
        let has_current_manpower = flags & F_HAS_CURRENT_MANPOWER != 0;
        let has_cumulative_losses = flags & F_HAS_CUMULATIVE_LOSSES != 0;
        let has_max_organisation = flags & F_HAS_MAX_ORGANISATION != 0;
        let has_counter_supply_status = flags & F_HAS_COUNTER_SUPPLY_STATUS != 0;
        let has_dig_in = flags & F_HAS_DIG_IN != 0;
        let has_dig_in_cap = flags & F_HAS_DIG_IN_CAP != 0;
        let training_observed = flags & F_TRAINING_OBSERVED != 0;
        let training_visible = flags & F_TRAINING_VISIBLE != 0;
        let dig_in_ui_observed = flags & F_DIG_IN_UI_OBSERVED != 0;
        let dig_in_ui_visible = flags & F_DIG_IN_UI_VISIBLE != 0;
        let dig_in_ui_capped = flags & F_DIG_IN_UI_CAPPED != 0;
        let invalid_orders_observed = flags & F_INVALID_ORDERS_OBSERVED != 0;
        let invalid_orders_visible = flags & F_INVALID_ORDERS_VISIBLE != 0;

        if training_visible && !training_observed {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: visible training without observation"
            ));
        }
        if invalid_orders_visible && !invalid_orders_observed {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: visible invalid-orders without observation"
            ));
        }
        if dig_in_ui_visible && !dig_in_ui_observed {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: visible dig-in without observation"
            ));
        }
        if dig_in_ui_capped && (!dig_in_ui_observed || !dig_in_ui_visible) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: capped dig-in without visible observation"
            ));
        }

        let location = u32_at(row, 12);
        let move_target = u32_at(row, 16);
        let move_source = u32_at(row, 20);
        let movement_progress = u32_at(row, 24);
        let progress_bucket = u32_at(row, 28);
        let organisation_raw = u32_at(row, 32);
        let strength_raw = u32_at(row, 40);
        let combat_ref_count = u32_at(row, 48);
        let template_id = u32_at(row, 52);
        let owner_tag_db_id = u32_at(row, 56);
        let army_id = u32_at(row, 60);
        let experience_raw = u32_at(row, 64);
        let max_supply_raw = u32_at(row, 68);
        let current_supply_raw = u32_at(row, 72);
        let supply_gain_raw = u32_at(row, 76);
        let fuel_raw = u32_at(row, 80);
        let order_instance_id = u32_at(row, 84);
        let disrupted_supply_raw = u32_at(row, 88);
        let strategic_redeployment_raw = u32_at(row, 92);
        let current_manpower = u32_at(row, 96);
        let cumulative_losses = u32_at(row, 100);
        let max_organisation_raw = if extended { u32_at(row, 104) } else { 0 };
        let counter_supply_status_raw = if extended { u32_at(row, 108) } else { 0 };
        let dig_in_raw = if extended { u32_at(row, 112) } else { 0 };
        let dig_in_cap_raw = if extended { u32_at(row, 116) } else { 0 };

        let valid_province = |value: u32| value > 0 && value <= MAX_PROVINCE_ID;
        if !valid_province(location) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: invalid location {location}"
            ));
        }
        if has_target && !valid_province(move_target) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: invalid move_target {move_target}"
            ));
        }
        if has_source && !valid_province(move_source) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: invalid move_source {move_source}"
            ));
        }
        if !moving && (has_progress || has_target || has_source) {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: movement fields while not moving"
            ));
        }
        if has_progress && movement_progress == 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: zero movement progress"
            ));
        }
        if !has_progress && movement_progress != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: progress without flag"
            ));
        }
        if !has_target && move_target != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: target without flag"
            ));
        }
        if !has_source && move_source != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: source without flag"
            ));
        }
        if !has_progress && progress_bucket != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: progress bucket without flag"
            ));
        }
        if organisation_raw > MAX_ORGANISATION_RAW {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: organisation exceeds max"
            ));
        }
        if strength_raw > MAX_STRENGTH_RAW {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: strength exceeds max"
            ));
        }
        if combat_ref_count > MAX_COMBAT_REF_COUNT {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: combat refs exceed max"
            ));
        }
        if !in_combat && combat_ref_count != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: combat refs while not in combat"
            ));
        }
        if in_combat && combat_ref_count == 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: in combat without refs"
            ));
        }

        check_optional_u32(
            has_template,
            template_id,
            1,
            i32::MAX as u32,
            "template_id",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_owner,
            owner_tag_db_id,
            1,
            compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
            "owner_tag_db_id",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_army,
            army_id,
            1,
            i32::MAX as u32,
            "army_id",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_experience,
            experience_raw,
            0,
            u32::MAX,
            "experience_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_max_supply,
            max_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "max_supply_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_current_supply,
            current_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "current_supply_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_supply_gain,
            supply_gain_raw,
            0,
            MAX_SUPPLY_GAIN_RAW,
            "supply_gain_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_fuel,
            fuel_raw,
            0,
            u32::MAX,
            "fuel_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_disrupted_supply,
            disrupted_supply_raw,
            0,
            MAX_DISRUPTED_SUPPLY_RAW,
            "disrupted_supply_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_strategic_redeployment,
            strategic_redeployment_raw,
            0,
            1,
            "strategic_redeployment_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_current_manpower,
            current_manpower,
            0,
            MAX_CURRENT_MANPOWER,
            "current_manpower",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_cumulative_losses,
            cumulative_losses,
            0,
            MAX_CUMULATIVE_LOSSES,
            "cumulative_losses",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_max_organisation,
            max_organisation_raw,
            100,
            MAX_ORGANISATION_RAW,
            "max_organisation_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_counter_supply_status,
            counter_supply_status_raw,
            0,
            MAX_COUNTER_SUPPLY_STATUS_RAW,
            "counter_supply_status_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_dig_in,
            dig_in_raw,
            0,
            MAX_DIG_IN_RAW,
            "dig_in_raw",
            row_index,
            at_byte,
        )?;
        check_optional_u32(
            has_dig_in_cap,
            dig_in_cap_raw,
            0,
            MAX_DIG_IN_RAW,
            "dig_in_cap_raw",
            row_index,
            at_byte,
        )?;

        let has_order_army = flags & F_ORDER_ARMY != 0;
        let has_order_group = flags & F_ORDER_ARMY_GROUP != 0;
        let has_order_unresolved = flags & F_ORDER_UNRESOLVED != 0;
        if has_order_army as u32 + has_order_group as u32 + has_order_unresolved as u32 > 1 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: conflicting order-link flags"
            ));
        }
        if has_order_army {
            if order_instance_id == 0 || order_instance_id > MAX_ORDER_INSTANCE_ID {
                return Err(format!(
                    "d20 row {row_index} at byte {at_byte}: invalid order instance id \
                     {order_instance_id}"
                ));
            }
        } else if order_instance_id != 0 {
            return Err(format!(
                "d20 row {row_index} at byte {at_byte}: order instance id without army-order flag"
            ));
        }
    }
    Ok(())
}

/// FRAME_END seal validation copied from production's
/// `validate_frame_end_seal_fields` (`stream_fold.rs:46`-`:89`).
fn check_frame_end_seal(payload: &[u8], at_byte: usize) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: payload length {} is not \
             {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted_domains = u32::from_le_bytes(payload[16..20].try_into().expect("4 bytes"));
    let scheduled_domains = u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes"));
    let status_flags = u32::from_le_bytes(payload[24..28].try_into().expect("4 bytes"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("4 bytes"));
    if reserved != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero \
             ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count \
             {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = status_flags & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain fatal bits {fatal:#010x}"
        ));
    }
    Ok((emitted_domains, scheduled_domains))
}

/// Pass 1: validate the whole compact stream and keep each usable d20 DATA
/// record. Every input-content failure is an `Err`; no partial measurement is
/// returned and no artifact has been created yet.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };
    let mut pos = 0usize;
    let mut hours: Vec<ScannedHour> = Vec::new();
    let mut failure_records = 0usize;
    let mut skipped_legacy = 0usize;
    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    let mut last_d20_hour: Option<i32> = None;

    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            return Err(format!(
                "stream ends mid-record at byte {pos}: {} byte(s) present, record needs {}",
                stream.len() - pos,
                record_end - pos
            ));
        }
        let payload = &stream[payload_start..record_end];
        validator
            .validate_record(&header, payload)
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;

        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID
                    && !non_probe_perf_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "duplicate DOMAIN_PERF for domain {} in hour {} at byte {pos}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while \
                             hour {open} is open",
                            header.game_hour_raw
                        ));
                    }
                    Some(_) => {}
                    None => {
                        if let Some(last_closed) = last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                return Err(format!(
                                    "non-monotonic DOMAIN_PERF at byte {pos}: closed hour \
                                     {last_closed}, then {}",
                                    header.game_hour_raw
                                ));
                            }
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "PERF hour conflict at byte {pos}: pending hour {pending}, \
                                     then {}",
                                    header.game_hour_raw
                                ));
                            }
                            Some(_) => {}
                            None => pending_perf_hour = Some(header.game_hour_raw),
                        }
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    return Err(format!(
                        "DOMAIN_DATA at byte {pos} carries hour {} while hour {open} is open",
                        header.game_hour_raw
                    ));
                }
                Some(_) => {}
                None => {
                    if let Some(pending) = pending_perf_hour {
                        if pending != header.game_hour_raw {
                            return Err(format!(
                                "DATA hour conflict at byte {pos}: pending PERF hour \
                                 {pending}, then DATA hour {}",
                                header.game_hour_raw
                            ));
                        }
                    }
                    if let Some(last_closed) = last_closed_hour {
                        if header.game_hour_raw <= last_closed {
                            return Err(format!(
                                "DOMAIN_DATA at byte {pos} reopens hour {} after hour \
                                 {last_closed} closed",
                                header.game_hour_raw
                            ));
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                let observed_perf_domains =
                    non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed_perf_domains != emitted_domains
                    || observed_perf_domains != scheduled_domains
                {
                    return Err(format!(
                        "FRAME_END domain counts at byte {pos} claim emitted={emitted_domains} \
                         scheduled={scheduled_domains}, but observed {observed_perf_domains} \
                         unique non-probe DOMAIN_PERF domains"
                    ));
                }
                match open_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} while hour {open} is open",
                            header.game_hour_raw
                        ));
                    }
                    None => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} without an open \
                             DOMAIN_DATA hour",
                            header.game_hour_raw
                        ));
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                    }
                }
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D20 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            let (division_header, body) = split_division_payload(payload, pos)?;
            if let Some(previous) = last_d20_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d20 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d20_hour = Some(header.game_hour_raw);

            let unknown_status = compact_status_unknown_flags_v1(division_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d20 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(division_header, body, pos)?;

            if compact_status_failure_flags_after_expected_partial_tolerance(
                D20,
                division_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else if division_header.row_len == LEGACY_ROW_LEN {
                skipped_legacy += 1;
            } else {
                let rows = body
                    .chunks_exact(ROW_LEN)
                    .map(|row| ShipRow {
                        bytes: row.try_into().expect("validated extended row"),
                    })
                    .collect();
                hours.push((
                    header.game_hour_raw,
                    division_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0,
                    rows,
                    pos..record_end,
                ));
            }
        }
        pos = record_end;
    }

    if let Some(open) = open_hour {
        return Err(format!("game hour {open} left open at EOF"));
    }
    if last_closed_hour.is_none() {
        return Err("stream closes no game hour (no FRAME_END records)".to_string());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err(format!(
            "stream ends after record type {}; final record must be a canonical FRAME_END",
            last_record_type.map_or_else(|| "none".to_string(), |kind| kind.to_string())
        ));
    }
    if skipped_legacy != 0 {
        return Err(format!(
            "stream carries {skipped_legacy} usable d20 record(s) with legacy rows; \
             this measurement maps only the extended row"
        ));
    }
    if hours.is_empty() {
        return Err("stream carries no usable d20 DATA records".to_string());
    }
    Ok(StreamScan {
        hours,
        failure_records,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_dynamics <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    let _failure_records = scan.failure_records;
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: fold to per-hour states; encode both arms. ----------------
    let schema = d20::schema();
    let collection = &schema.collections[0];

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut world: BTreeMap<String, ShipRow> = BTreeMap::new();
    let mut prior: Option<KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut divisions_total = 0usize;
    let mut non_contiguous = 0usize;
    let mut last_hour: Option<i32> = None;

    for (game_hour, delta, rows, ship_span) in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[ship_span.clone()]);
        ship_offsets.push(ship.len());

        if let Some(prev) = last_hour {
            if *game_hour != prev + 1 {
                non_contiguous += 1;
            }
        }
        last_hour = Some(*game_hour);

        // Fold exactly as world_fold does: baseline replaces, delta upserts.
        if !*delta {
            world.clear();
        }
        for row in rows {
            let uid = format!("{}:{}", u32_at(&row.bytes, 0), u32_at(&row.bytes, 4));
            if row.flags() & F_DELETED != 0 {
                world.remove(&uid);
            } else {
                world.insert(uid, row.clone());
            }
        }

        // The hour's full state in the new grammar.
        let mut state: KeyedState = BTreeMap::new();
        for row in world.values() {
            state.insert(
                key(&[u32_at(&row.bytes, 0) as u64, u32_at(&row.bytes, 4) as u64]),
                row.to_record(collection),
            );
        }
        divisions_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(&state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
        } else {
            let prev = prior.as_ref().unwrap();
            let d = diff(collection, prev, &state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                tombstones_total += d.tombstones.len();
                encode_delta(&schema, std::slice::from_ref(prev), &[Some(d)], &mut wire)
                    .expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's
        // OWN reconstructed prior, exactly as a conforming reader would, and
        // require the result to be stored-equal to this hour's state. This is
        // what makes the size claim a claim about decodable bytes.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(&schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().unwrap();
        assert_eq!(replayed.len(), state.len(), "hour {hours_seen}: row count");
        let residual = diff(collection, replayed, &state);
        assert!(
            residual.is_empty(),
            "hour {hours_seen}: reader state is not stored-equal to the fold"
        );

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d20.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d20.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!("hours                {hours_seen} ({non_contiguous} non-contiguous steps)");
    println!(
        "divisions avg        {:.1}",
        divisions_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d20.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        assert_eq!(out.len(), COMPACT_RECORD_HEADER_LEN);
        out.extend_from_slice(payload);
        out
    }

    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn frame_end(sequence: u64, game_hour: i32) -> Vec<u8> {
        frame_end_sealed(sequence, game_hour, 0, 0, 0, 0)
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// A minimal valid 120-byte extended division row.
    fn division_row(div_id: u32, owner: u32) -> Vec<u8> {
        let mut row = vec![0u8; ROW_LEN];
        row[0..4].copy_from_slice(&div_id.to_le_bytes());
        row[4..8].copy_from_slice(&DIVISION_TYPE.to_le_bytes());
        row[8..12].copy_from_slice(&F_HAS_OWNER.to_le_bytes());
        row[12..16].copy_from_slice(&1u32.to_le_bytes()); // valid location
        row[56..60].copy_from_slice(&owner.to_le_bytes());
        row
    }

    fn d20_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = vec![0u8; TABLE_HEADER_LEN];
        p[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p[12..16].copy_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p[16..20].copy_from_slice(&status_flags.to_le_bytes());
        p[20..24].copy_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
        p[24..28].copy_from_slice(&(rows.len() as u32).to_le_bytes()); // root_rows
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    fn d20_record(sequence: u64, game_hour: i32, status: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d20_payload(status, rows),
        )
    }

    fn good_stream() -> Vec<u8> {
        let mut stream = Vec::new();
        stream.extend(d20_record(1, 100, 0, &[division_row(1, 42)]));
        stream.extend(frame_end(2, 100));
        stream.extend(d20_record(
            3,
            101,
            COMPACT_STATUS_DELTA_DATA,
            &[division_row(1, 43)],
        ));
        stream.extend(frame_end(4, 101));
        stream
    }

    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let header = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        pos
    }

    #[test]
    fn a_well_formed_stream_yields_its_d20_hours() {
        let scan = scan_stream(&good_stream(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].0, 100);
        assert!(!scan.hours[0].1);
        assert_eq!(scan.hours[1].0, 101);
        assert!(scan.hours[1].1);
        assert_eq!(scan.failure_records, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                for rows in [Vec::new(), vec![division_row(1, 42)]] {
                    let mut stream = Vec::new();
                    stream.extend(d20_record(1, 100, status | extra, &rows));
                    stream.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&stream, StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        rows.len()
                    );
                }
            }
        }

        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut stream = Vec::new();
            stream.extend(d20_record(1, 100, informational, &[division_row(1, 42)]));
            stream.extend(frame_end(2, 100));
            let scan = scan_stream(&stream, StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational status {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1);
            assert_eq!(scan.failure_records, 0);
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(scan_stream(&full[..cut], StreamIdentityMode::Legacy).is_err());
        }
        for (records, expected) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            assert_eq!(
                scan_stream(&full[..cut], StreamIdentityMode::Legacy)
                    .expect("clean boundary")
                    .hours
                    .len(),
                expected
            );
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=4).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if !boundaries.contains(&cut) {
                assert!(
                    scan_stream(&full[..cut], StreamIdentityMode::Legacy).is_err(),
                    "mid-record cut at {cut}"
                );
            }
        }
        assert!(scan_stream(&[], StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for record_index in 0..4 {
            let at = boundary(&good_stream(), record_index);
            for (offset, patch) in [
                (0usize, vec![b'X']),
                (4, 2u16.to_le_bytes().to_vec()),
                (6, 33u16.to_le_bytes().to_vec()),
            ] {
                let mut stream = good_stream();
                stream[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for record_index in 0..4 {
            let at = boundary(&good_stream(), record_index);
            for wrong in [0u64, 9] {
                let mut stream = good_stream();
                stream[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut mismatched = Vec::new();
        mismatched.extend(d20_record(1, 100, 0, &[division_row(1, 42)]));
        mismatched.extend(frame_end(2, 999));
        assert!(scan_stream(&mismatched, StreamIdentityMode::Legacy).is_err());

        assert!(scan_stream(&frame_end(1, 100), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut stream = Vec::new();
        stream.extend(d20_record(1, 101, 0, &[division_row(1, 42)]));
        stream.extend(frame_end(2, 101));
        stream.extend(d20_record(3, 100, 0, &[division_row(1, 42)]));
        stream.extend(frame_end(4, 100));
        assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d20_record_in_one_hour_is_rejected() {
        for (first, second) in [(0u32, 0u32), (0, 1), (1, 0), (1, 1)] {
            let mut stream = Vec::new();
            stream.extend(d20_record(1, 100, first, &[division_row(1, 42)]));
            stream.extend(d20_record(2, 100, second, &[division_row(2, 42)]));
            stream.extend(frame_end(3, 100));
            assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut stream = Vec::new();
            stream.extend(d20_record(1, 100, 1 << bit, &[division_row(1, 42)]));
            stream.extend(frame_end(2, 100));
            stream.extend(d20_record(3, 101, 0, &[division_row(2, 42)]));
            stream.extend(frame_end(4, 101));
            let scan =
                scan_stream(&stream, StreamIdentityMode::Legacy).expect("well-formed stream");
            assert_eq!(scan.failure_records, 1, "failure bit {bit}");
            assert_eq!(scan.hours.len(), 1, "failure bit {bit}");
            assert_eq!(scan.hours[0].0, 101, "failure bit {bit}");
        }
    }

    #[test]
    fn a_stream_with_no_d20_records_is_refused_not_reported_as_zero() {
        let mut stream = Vec::new();
        stream.extend(record(
            1,
            100,
            63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        stream.extend(frame_end(2, 100));
        assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d20_payload_is_refused_not_skipped() {
        let mut short = Vec::new();
        short.extend(record(
            1,
            100,
            D20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; TABLE_HEADER_LEN - 1],
        ));
        short.extend(frame_end(2, 100));
        assert!(scan_stream(&short, StreamIdentityMode::Legacy).is_err());

        let mut bad_length = d20_payload(0, &[division_row(1, 42)]);
        bad_length.pop();
        let mut stream = Vec::new();
        stream.extend(record(
            1,
            100,
            D20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &bad_length,
        ));
        stream.extend(frame_end(2, 100));
        assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn the_d20_payload_authority_matches_production_predicates() {
        let mut bad_payloads: Vec<(&str, Vec<u8>)> = Vec::new();

        let mut scale = d20_payload(0, &[division_row(1, 42)]);
        scale[20..24].copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("scale", scale));

        let mut reserved = d20_payload(0, &[division_row(1, 42)]);
        reserved[28..32].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(("reserved", reserved));

        let mut changed = d20_payload(0, &[division_row(1, 42)]);
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        bad_payloads.push(("changed_rows", changed));

        let mut root = d20_payload(0, &[division_row(1, 42)]);
        root[24..28].copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("root_rows", root));

        let mut zero_id = d20_payload(0, &[division_row(1, 42)]);
        zero_id[TABLE_HEADER_LEN..TABLE_HEADER_LEN + 4].copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("zero div_id", zero_id));

        let row = division_row(1, 42);
        bad_payloads.push(("duplicate div_id", d20_payload(0, &[row.clone(), row])));

        let mut bad_type = d20_payload(0, &[division_row(1, 42)]);
        bad_type[TABLE_HEADER_LEN + 4..TABLE_HEADER_LEN + 8].copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("div_type", bad_type));

        let mut bad_location = d20_payload(0, &[division_row(1, 42)]);
        bad_location[TABLE_HEADER_LEN + 12..TABLE_HEADER_LEN + 16]
            .copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("location", bad_location));

        let mut movement = d20_payload(0, &[division_row(1, 42)]);
        movement[TABLE_HEADER_LEN + 8..TABLE_HEADER_LEN + 12]
            .copy_from_slice(&F_HAS_PROGRESS.to_le_bytes());
        movement[TABLE_HEADER_LEN + 24..TABLE_HEADER_LEN + 28].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(("movement relationship", movement));

        let mut combat = d20_payload(0, &[division_row(1, 42)]);
        combat[TABLE_HEADER_LEN + 8..TABLE_HEADER_LEN + 12]
            .copy_from_slice(&(F_HAS_OWNER | F_IN_COMBAT).to_le_bytes());
        bad_payloads.push(("combat relationship", combat));

        let mut optional = d20_payload(0, &[division_row(1, 42)]);
        optional[TABLE_HEADER_LEN + 76..TABLE_HEADER_LEN + 80].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(("optional zero-pack", optional));

        let mut ui = d20_payload(0, &[division_row(1, 42)]);
        ui[TABLE_HEADER_LEN + 8..TABLE_HEADER_LEN + 12]
            .copy_from_slice(&(F_HAS_OWNER | F_TRAINING_VISIBLE).to_le_bytes());
        bad_payloads.push(("UI implication", ui));

        let mut order = d20_payload(0, &[division_row(1, 42)]);
        order[TABLE_HEADER_LEN + 8..TABLE_HEADER_LEN + 12]
            .copy_from_slice(&(F_HAS_OWNER | F_ORDER_ARMY | F_ORDER_ARMY_GROUP).to_le_bytes());
        bad_payloads.push(("order relationship", order));

        let mut delete = d20_payload(0, &[division_row(1, 42)]);
        delete[TABLE_HEADER_LEN + 8..TABLE_HEADER_LEN + 12]
            .copy_from_slice(&F_DELETED.to_le_bytes());
        bad_payloads.push(("delete zero-fill", delete));

        for (label, payload) in bad_payloads {
            let mut stream = Vec::new();
            stream.extend(record(
                1,
                100,
                D20,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            stream.extend(frame_end(2, 100));
            assert!(
                scan_stream(&stream, StreamIdentityMode::Legacy).is_err(),
                "{label}"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = || d20_record(1, 100, 0, &[division_row(1, 42)]);
        let mut good = data();
        good.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&good, StreamIdentityMode::Legacy).is_ok());

        let mut bad_seals: Vec<(String, Vec<u8>)> = vec![
            ("reserved".into(), frame_end_sealed(2, 100, 0, 0, 0, 1)),
            (
                "count mismatch".into(),
                frame_end_sealed(2, 100, 0, 1, 0, 0),
            ),
        ];
        for bit in 14..32u32 {
            bad_seals.push((
                format!("unknown bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push((
                format!("fatal bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for (label, seal) in bad_seals {
            let mut stream = data();
            stream.extend(seal);
            assert!(
                scan_stream(&stream, StreamIdentityMode::Legacy).is_err(),
                "{label}"
            );
        }

        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut stream = data();
            stream.extend(record(
                2,
                100,
                compact_contract::COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut stream = data();
            stream.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_ok());
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut good = Vec::new();
        good.extend(perf(1, 100, D20));
        good.extend(perf(2, 100, 63));
        good.extend(perf(3, 100, PROBE_DOMAIN_ID));
        good.extend(d20_record(4, 100, 0, &[division_row(1, 42)]));
        good.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&good, StreamIdentityMode::Legacy).is_ok());

        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(d20_record(3, 100, 0, &[division_row(1, 42)]));
        probe_twice.extend(frame_end(4, 100));
        assert!(scan_stream(&probe_twice, StreamIdentityMode::Legacy).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 63));
        duplicate.extend(perf(2, 100, 63));
        duplicate.extend(d20_record(3, 100, 0, &[division_row(1, 42)]));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, StreamIdentityMode::Legacy).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(d20_record(1, 100, 0, &[division_row(1, 42)]));
        interleaved.extend(perf(2, 999, 63));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, StreamIdentityMode::Legacy).is_err());

        let mut stale = Vec::new();
        stale.extend(d20_record(1, 100, 0, &[division_row(1, 42)]));
        stale.extend(frame_end(2, 100));
        stale.extend(perf(3, 100, 63));
        stale.extend(d20_record(4, 101, 0, &[division_row(2, 42)]));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, StreamIdentityMode::Legacy).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 63));
        conflict.extend(perf(2, 101, D20));
        conflict.extend(d20_record(3, 100, 0, &[division_row(1, 42)]));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, StreamIdentityMode::Legacy).is_err());

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut stream = Vec::new();
            stream.extend(perf(1, 100, D20));
            stream.extend(perf(2, 100, 63));
            stream.extend(d20_record(3, 100, 0, &[division_row(1, 42)]));
            stream.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 63));
        two_hours.extend(d20_record(2, 100, 0, &[division_row(1, 42)]));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 63));
        two_hours.extend(d20_record(5, 101, 0, &[division_row(2, 42)]));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert!(scan_stream(&two_hours, StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut stream = Vec::new();
            stream.extend(perf(1, perf_hour, 63));
            stream.extend(d20_record(2, data_hour, 0, &[division_row(1, 42)]));
            stream.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&stream, StreamIdentityMode::Legacy).is_err());
        }

        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 63));
        agreeing.extend(d20_record(2, 100, 0, &[division_row(1, 42)]));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d20 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut malformed = d20_payload(1 << bit, &[division_row(1, 42)]);
            malformed[20..24].copy_from_slice(&0u32.to_le_bytes());
            let mut stream = Vec::new();
            stream.extend(record(
                1,
                100,
                D20,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &malformed,
            ));
            stream.extend(frame_end(2, 100));
            stream.extend(d20_record(3, 101, 0, &[division_row(2, 42)]));
            stream.extend(frame_end(4, 101));
            assert!(
                scan_stream(&stream, StreamIdentityMode::Legacy).is_err(),
                "failure bit {bit}"
            );
        }

        let mut stream = Vec::new();
        stream.extend(d20_record(1, 100, 1, &[division_row(1, 42)]));
        stream.extend(frame_end(2, 100));
        stream.extend(d20_record(3, 101, 0, &[division_row(2, 42)]));
        stream.extend(frame_end(4, 101));
        let scan = scan_stream(&stream, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // Delta-before-baseline is fold state, not payload malformedness. On an
        // already excluded record the fold returns before consulting it, so the
        // authority must not invent a refusal.
        let mut stream = Vec::new();
        stream.extend(d20_record(
            1,
            100,
            1 | COMPACT_STATUS_DELTA_DATA,
            &[division_row(1, 42)],
        ));
        stream.extend(frame_end(2, 100));
        stream.extend(d20_record(3, 101, 0, &[division_row(2, 42)]));
        stream.extend(frame_end(4, 101));
        let scan = scan_stream(&stream, StreamIdentityMode::Legacy)
            .expect("fold-only delta state is outside the payload authority");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(&decapitated, StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&decapitated, StreamIdentityMode::Legacy).is_ok());

        let mut identity_stream = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity_stream.extend(good_stream());
        assert!(scan_stream(&identity_stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        let mut state = 0xD20D_1A15_5EED_u64;
        for len in 0..256usize {
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes, StreamIdentityMode::Legacy);
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut stream = good.clone();
            stream[i] ^= 0xFF;
            let _ = scan_stream(&stream, StreamIdentityMode::Legacy);
        }
    }
}
