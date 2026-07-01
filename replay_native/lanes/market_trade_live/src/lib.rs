use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 43;
pub const DOMAIN_NAME: &str = "market_trade_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;

pub const MARKET_TRADE_HEADER_LEN: usize = 48;
pub const MARKET_TRADE_ROW_LEN: usize = 48;
pub const MARKET_TRADE_CONTRACT_TYPE: u32 = 81;
pub const MARKET_TRADE_MAX_TAG_DB_ID: u32 = 2_048;
pub const MARKET_TRADE_MAX_CONTRACT_ID: u32 = i32::MAX as u32;
pub const MARKET_TRADE_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const MARKET_TRADE_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const MARKET_TRADE_SCALE_RAW: u32 = 100_000;

pub const ROW_FLAG_BUYER_PTR_VALID: u32 = 1;
pub const ROW_FLAG_EQUIPMENT_SUBBUF_VALID: u32 = 1 << 1;
pub const ROW_FLAG_DELIVERY_SUBBUF_VALID: u32 = 1 << 2;
pub const ROW_FLAG_VARIANT_VALID: u32 = 1 << 3;
pub const ROW_FLAG_AMOUNT_VALID: u32 = 1 << 4;
pub const ROW_FLAGS_KNOWN: u32 = ROW_FLAG_BUYER_PTR_VALID
    | ROW_FLAG_EQUIPMENT_SUBBUF_VALID
    | ROW_FLAG_DELIVERY_SUBBUF_VALID
    | ROW_FLAG_VARIANT_VALID
    | ROW_FLAG_AMOUNT_VALID;

pub const FIELD_STATUS_INVALID: u32 = 0;
pub const FIELD_STATUS_VALID: u32 = 1;
pub const FIELD_STATUS_UNREADABLE: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTradeRow {
    pub contract_id: u32,
    pub contract_type: u32,
    pub flags: u32,
    pub seller_tag_db_id: u32,
    pub buyer_tag_db_id: u32,
    pub equipment_variant_id: u32,
    pub amount_raw: i64,
    pub equipment_subbuf_status: u32,
    pub delivery_subbuf_status: u32,
    pub buyer_ptr_status: u32,
}

impl MarketTradeRow {
    pub fn complete(contract_id: u32, seller_tag_db_id: u32, buyer_tag_db_id: u32) -> Self {
        Self {
            contract_id,
            contract_type: MARKET_TRADE_CONTRACT_TYPE,
            flags: ROW_FLAGS_KNOWN,
            seller_tag_db_id,
            buyer_tag_db_id,
            equipment_variant_id: 10_000 + contract_id,
            amount_raw: 25 * MARKET_TRADE_SCALE_RAW as i64,
            equipment_subbuf_status: FIELD_STATUS_VALID,
            delivery_subbuf_status: FIELD_STATUS_VALID,
            buyer_ptr_status: FIELD_STATUS_VALID,
        }
    }

    pub fn has_complete_extractable_fields(&self) -> bool {
        self.flags & ROW_FLAGS_KNOWN == ROW_FLAGS_KNOWN
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketTradePayloadSummary {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_contract_rows: u32,
    pub emitted_contract_rows: u32,
    pub omitted_candidate_rows: u32,
    pub buyer_ptr_valid_rows: u32,
    pub variant_valid_rows: u32,
    pub amount_valid_rows: u32,
    pub partial_field_rows: u32,
    pub scale_raw: u32,
    pub rows: Vec<MarketTradeRow>,
    pub parse_errors: Vec<String>,
}

impl MarketTradePayloadSummary {
    pub fn missing_extractable_rows(&self) -> u32 {
        self.partial_field_rows
    }

    pub fn ok(&self) -> bool {
        self.parse_errors.is_empty()
            && self.status_flags
                & (STATUS_WORKER_PANIC
                    | STATUS_BRIDGE_UNAVAILABLE
                    | STATUS_SOURCE_UNAVAILABLE
                    | STATUS_PARTIAL_DATA)
                == 0
            && self.partial_field_rows == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketTradeStreamSummary {
    pub total_bytes: usize,
    pub record_count: u64,
    pub domain_data_records: u64,
    pub frame_end_records: u64,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub unique_game_hours: usize,
    pub skipped_game_hours: Vec<i32>,
    pub status_flags_or: u32,
    pub row_count_total: u64,
    pub observed_contract_rows_total: u64,
    pub omitted_candidate_rows_total: u64,
    pub partial_field_rows_total: u64,
    pub payloads: Vec<MarketTradePayloadSummary>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTradeGate {
    pub ok: bool,
    pub problems: Vec<&'static str>,
    pub summary: MarketTradeStreamSummary,
}

pub fn encode_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_contract_rows: u32,
    changed_rows: u32,
    rows: &[MarketTradeRow],
) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let buyer_ptr_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_BUYER_PTR_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let variant_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_VARIANT_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let amount_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_AMOUNT_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let partial_field_rows = rows
        .iter()
        .filter(|row| !row.has_complete_extractable_fields())
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload =
        Vec::with_capacity(MARKET_TRADE_HEADER_LEN + rows.len() * MARKET_TRADE_ROW_LEN);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&changed_rows.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&observed_contract_rows.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&buyer_ptr_valid_rows.to_le_bytes());
    payload.extend_from_slice(&variant_valid_rows.to_le_bytes());
    payload.extend_from_slice(&amount_valid_rows.to_le_bytes());
    payload.extend_from_slice(&partial_field_rows.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.contract_id.to_le_bytes());
        payload.extend_from_slice(&row.contract_type.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.seller_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.buyer_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.equipment_variant_id.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&row.equipment_subbuf_status.to_le_bytes());
        payload.extend_from_slice(&row.delivery_subbuf_status.to_le_bytes());
        payload.extend_from_slice(&row.buyer_ptr_status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> MarketTradePayloadSummary {
    let mut summary = MarketTradePayloadSummary::default();
    if payload.len() < MARKET_TRADE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: market_trade_live payload is {} byte(s), expected at least {MARKET_TRADE_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    summary.elapsed_wall_ms = read_u64_le(payload, 0);
    summary.row_count = read_u32_le(payload, 8);
    summary.changed_rows = read_u32_le(payload, 12);
    summary.status_flags = read_u32_le(payload, 16);
    summary.observed_contract_rows = read_u32_le(payload, 20);
    summary.emitted_contract_rows = read_u32_le(payload, 24);
    summary.omitted_candidate_rows = summary
        .observed_contract_rows
        .saturating_sub(summary.emitted_contract_rows);
    summary.buyer_ptr_valid_rows = read_u32_le(payload, 28);
    summary.variant_valid_rows = read_u32_le(payload, 32);
    summary.amount_valid_rows = read_u32_le(payload, 36);
    summary.partial_field_rows = read_u32_le(payload, 40);
    summary.scale_raw = read_u32_le(payload, 44);

    if summary.changed_rows > summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: changed_rows {} > row_count {}",
            summary.changed_rows, summary.row_count
        ));
    }
    if summary.emitted_contract_rows != summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: emitted_contract_rows {} != row_count {}",
            summary.emitted_contract_rows, summary.row_count
        ));
    }
    if summary.observed_contract_rows < summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: observed_contract_rows {} < row_count {}",
            summary.observed_contract_rows, summary.row_count
        ));
    }
    if summary.scale_raw != MARKET_TRADE_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: scale_raw {} != {MARKET_TRADE_SCALE_RAW}",
            summary.scale_raw
        ));
    }
    for (name, count) in [
        ("buyer_ptr_valid_rows", summary.buyer_ptr_valid_rows),
        ("variant_valid_rows", summary.variant_valid_rows),
        ("amount_valid_rows", summary.amount_valid_rows),
        ("partial_field_rows", summary.partial_field_rows),
    ] {
        if count > summary.row_count {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} {count} > row_count {}",
                summary.row_count
            ));
        }
    }

    let expected_len = MARKET_TRADE_HEADER_LEN
        .saturating_add((summary.row_count as usize).saturating_mul(MARKET_TRADE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market_trade_live payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }

    let mut seen_contracts = BTreeSet::new();
    let mut computed_buyer = 0u32;
    let mut computed_variant = 0u32;
    let mut computed_amount = 0u32;
    let mut computed_partial = 0u32;
    let mut cursor = MARKET_TRADE_HEADER_LEN;
    for row_index in 0..summary.row_count {
        let row = MarketTradeRow {
            contract_id: read_u32_le(payload, cursor),
            contract_type: read_u32_le(payload, cursor + 4),
            flags: read_u32_le(payload, cursor + 8),
            seller_tag_db_id: read_u32_le(payload, cursor + 12),
            buyer_tag_db_id: read_u32_le(payload, cursor + 16),
            equipment_variant_id: read_u32_le(payload, cursor + 20),
            amount_raw: read_i64_le(payload, cursor + 24),
            equipment_subbuf_status: read_u32_le(payload, cursor + 32),
            delivery_subbuf_status: read_u32_le(payload, cursor + 36),
            buyer_ptr_status: read_u32_le(payload, cursor + 40),
        };
        let reserved = read_u32_le(payload, cursor + 44);
        cursor += MARKET_TRADE_ROW_LEN;

        let buyer_valid = row.flags & ROW_FLAG_BUYER_PTR_VALID != 0;
        let equipment_valid = row.flags & ROW_FLAG_EQUIPMENT_SUBBUF_VALID != 0;
        let delivery_valid = row.flags & ROW_FLAG_DELIVERY_SUBBUF_VALID != 0;
        let variant_valid = row.flags & ROW_FLAG_VARIANT_VALID != 0;
        let amount_valid = row.flags & ROW_FLAG_AMOUNT_VALID != 0;
        computed_buyer += u32::from(buyer_valid);
        computed_variant += u32::from(variant_valid);
        computed_amount += u32::from(amount_valid);
        computed_partial += u32::from(
            !(buyer_valid && equipment_valid && delivery_valid && variant_valid && amount_valid),
        );

        validate_row(
            offset,
            row_index as usize,
            &row,
            reserved,
            &mut summary.parse_errors,
        );
        if !seen_contracts.insert(row.contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} duplicates contract_id {}",
                row.contract_id
            ));
        }
        summary.rows.push(row);
    }

    for (name, expected, computed) in [
        (
            "buyer_ptr_valid_rows",
            summary.buyer_ptr_valid_rows,
            computed_buyer,
        ),
        (
            "variant_valid_rows",
            summary.variant_valid_rows,
            computed_variant,
        ),
        (
            "amount_valid_rows",
            summary.amount_valid_rows,
            computed_amount,
        ),
        (
            "partial_field_rows",
            summary.partial_field_rows,
            computed_partial,
        ),
    ] {
        if expected != computed {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} header {expected} != decoded {computed}"
            ));
        }
    }
    summary
}

fn validate_row(
    offset: usize,
    row_index: usize,
    row: &MarketTradeRow,
    reserved: u32,
    errors: &mut Vec<String>,
) {
    if row.contract_id == 0 || row.contract_id > MARKET_TRADE_MAX_CONTRACT_ID {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} invalid contract_id {}",
            row.contract_id
        ));
    }
    if row.contract_type != MARKET_TRADE_CONTRACT_TYPE {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} contract_type {} != {MARKET_TRADE_CONTRACT_TYPE}",
            row.contract_type
        ));
    }
    for (name, tag_db_id) in [
        ("seller_tag_db_id", row.seller_tag_db_id),
        ("buyer_tag_db_id", row.buyer_tag_db_id),
    ] {
        if tag_db_id == 0 || tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid {name} {tag_db_id}"
            ));
        }
    }
    if row.flags & !ROW_FLAGS_KNOWN != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} unknown flags 0x{:X}",
            row.flags & !ROW_FLAGS_KNOWN
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} reserved field is nonzero"
        ));
    }
    validate_status_pair(
        offset,
        row_index,
        "buyer_ptr",
        row.flags & ROW_FLAG_BUYER_PTR_VALID != 0,
        row.buyer_ptr_status,
        errors,
    );
    validate_status_pair(
        offset,
        row_index,
        "equipment_subbuf",
        row.flags & ROW_FLAG_EQUIPMENT_SUBBUF_VALID != 0,
        row.equipment_subbuf_status,
        errors,
    );
    validate_status_pair(
        offset,
        row_index,
        "delivery_subbuf",
        row.flags & ROW_FLAG_DELIVERY_SUBBUF_VALID != 0,
        row.delivery_subbuf_status,
        errors,
    );

    if row.flags & ROW_FLAG_VARIANT_VALID != 0 {
        if row.equipment_variant_id == 0 || row.equipment_variant_id > MARKET_TRADE_MAX_VARIANT_ID {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid equipment_variant_id {}",
                row.equipment_variant_id
            ));
        }
    } else if row.equipment_variant_id != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} has variant id without valid flag"
        ));
    }
    if row.flags & ROW_FLAG_AMOUNT_VALID != 0 {
        if row.amount_raw <= 0 || row.amount_raw > MARKET_TRADE_MAX_AMOUNT_RAW {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid amount_raw {}",
                row.amount_raw
            ));
        }
    } else if row.amount_raw != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} has amount without valid flag"
        ));
    }
}

fn validate_status_pair(
    offset: usize,
    row_index: usize,
    name: &str,
    valid_flag: bool,
    status: u32,
    errors: &mut Vec<String>,
) {
    if status > FIELD_STATUS_UNREADABLE {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} invalid {name}_status {status}"
        ));
    }
    if valid_flag != (status == FIELD_STATUS_VALID) {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} {name} flag/status mismatch"
        ));
    }
}

pub fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn summarize_compact_stream(raw: &[u8]) -> MarketTradeStreamSummary {
    let mut summary = MarketTradeStreamSummary {
        total_bytes: raw.len(),
        ..MarketTradeStreamSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence = None;
    let mut frame_hours = BTreeSet::new();
    let mut data_hours = BTreeSet::new();
    let mut per_hour_data = BTreeMap::<i32, u32>::new();

    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid compact record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported compact record version {version} header_len {header_len}"
            ));
            break;
        }
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let reserved = read_u32_le(header, 28);
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: compact record reserved field is nonzero"
            ));
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);

        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > raw.len().saturating_sub(offset) {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        summary.record_count = summary.record_count.saturating_add(1);

        match record_type {
            RECORD_TYPE_DOMAIN_DATA if domain_id == DOMAIN_ID => {
                summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                data_hours.insert(game_hour_raw);
                *per_hour_data.entry(game_hour_raw).or_default() += 1;
                let payload_summary = decode_payload(offset, payload);
                summary.status_flags_or |= payload_summary.status_flags;
                summary.row_count_total = summary
                    .row_count_total
                    .saturating_add(payload_summary.row_count as u64);
                summary.observed_contract_rows_total = summary
                    .observed_contract_rows_total
                    .saturating_add(payload_summary.observed_contract_rows as u64);
                summary.omitted_candidate_rows_total = summary
                    .omitted_candidate_rows_total
                    .saturating_add(payload_summary.omitted_candidate_rows as u64);
                summary.partial_field_rows_total = summary
                    .partial_field_rows_total
                    .saturating_add(payload_summary.partial_field_rows as u64);
                summary
                    .parse_errors
                    .extend(payload_summary.parse_errors.iter().cloned());
                summary.payloads.push(payload_summary);
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_records = summary.frame_end_records.saturating_add(1);
                frame_hours.insert(game_hour_raw);
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
            }
            RECORD_TYPE_DOMAIN_PERF | RECORD_TYPE_DOMAIN_DATA => {}
            other => summary.parse_errors.push(format!(
                "offset {offset}: unknown compact record type {other}"
            )),
        }
        offset += record_len;
    }

    let all_hours: BTreeSet<i32> = frame_hours.union(&data_hours).copied().collect();
    summary.first_game_hour_raw = all_hours.first().copied();
    summary.last_game_hour_raw = all_hours.last().copied();
    summary.unique_game_hours = all_hours.len();
    summary.skipped_game_hours = skipped_hours(&all_hours);
    for (hour, count) in per_hour_data {
        if count != 1 {
            summary.parse_errors.push(format!(
                "game_hour_raw {hour}: market_trade_live data record count {count} != 1"
            ));
        }
    }
    summary
}

pub fn gate_compact_stream(raw: &[u8], min_game_hours: usize) -> MarketTradeGate {
    let summary = summarize_compact_stream(raw);
    let mut problems = Vec::new();
    if !summary.parse_errors.is_empty() {
        problems.push("parse_errors");
    }
    if !summary.ordering_errors.is_empty() {
        problems.push("ordering_errors");
    }
    if summary.domain_data_records == 0 {
        problems.push("no_market_trade_data_records");
    }
    if summary.unique_game_hours < min_game_hours {
        problems.push("insufficient_game_hours");
    }
    if !summary.skipped_game_hours.is_empty() {
        problems.push("skipped_game_hours");
    }
    if summary.status_flags_or & STATUS_WORKER_PANIC != 0 {
        problems.push("worker_panic");
    }
    if summary.status_flags_or & STATUS_BRIDGE_UNAVAILABLE != 0 {
        problems.push("bridge_unavailable");
    }
    if summary.status_flags_or & STATUS_SOURCE_UNAVAILABLE != 0 {
        problems.push("source_unavailable");
    }
    if summary.status_flags_or & STATUS_PARTIAL_DATA != 0 {
        problems.push("partial_data_status");
    }
    if summary.partial_field_rows_total != 0 {
        problems.push("missing_extractable_market_trade_rows");
    }
    MarketTradeGate {
        ok: problems.is_empty(),
        problems,
        summary,
    }
}

pub fn skipped_hours(hours: &BTreeSet<i32>) -> Vec<i32> {
    let mut missing = Vec::new();
    let ordered: Vec<i32> = hours.iter().copied().collect();
    for pair in ordered.windows(2) {
        let prev = pair[0];
        let next = pair[1];
        if next > prev + 1 {
            missing.extend((prev + 1)..next);
        }
    }
    missing
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip_accepts_complete_purchase_contract_rows() {
        let rows = vec![
            MarketTradeRow::complete(726, 66, 17),
            MarketTradeRow::complete(785, 73, 33),
        ];
        let payload = encode_payload(1234, 0, 2, 2, &rows);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.row_count, 2);
        assert_eq!(decoded.observed_contract_rows, 2);
        assert_eq!(decoded.omitted_candidate_rows, 0);
        assert_eq!(decoded.partial_field_rows, 0);
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn omitted_candidates_are_accounted_but_not_partial() {
        let rows = vec![MarketTradeRow::complete(726, 66, 17)];
        let payload = encode_payload(1234, 0, 4, 1, &rows);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.omitted_candidate_rows, 3);
        assert_eq!(decoded.missing_extractable_rows(), 0);
    }

    #[test]
    fn partial_row_fails_gate_even_when_payload_decodes() {
        let mut row = MarketTradeRow::complete(785, 73, 368);
        row.flags &= !ROW_FLAG_BUYER_PTR_VALID;
        row.buyer_ptr_status = FIELD_STATUS_INVALID;
        let payload = encode_payload(1234, STATUS_PARTIAL_DATA, 1, 1, &[row]);

        let decoded = decode_payload(0, &payload);

        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
        assert!(!decoded.ok());
        assert_eq!(decoded.partial_field_rows, 1);
    }

    #[test]
    fn decoder_rejects_fabricated_variant_without_flag() {
        let mut row = MarketTradeRow::complete(1, 2, 3);
        row.flags &= !ROW_FLAG_VARIANT_VALID;
        row.equipment_variant_id = 99;
        let payload = encode_payload(0, 0, 1, 1, &[row]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|err| err.contains("variant id without valid flag")));
    }

    #[test]
    fn compact_stream_gate_requires_hourly_unskipped_data() {
        let payload_a = encode_payload(0, 0, 1, 1, &[MarketTradeRow::complete(1, 2, 3)]);
        let payload_b = encode_payload(1, 0, 1, 1, &[MarketTradeRow::complete(2, 2, 3)]);
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            10,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_a,
        );
        append_compact_record(&mut stream, 2, 10, 0, RECORD_TYPE_FRAME_END, &[]);
        append_compact_record(
            &mut stream,
            3,
            12,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_b,
        );
        append_compact_record(&mut stream, 4, 12, 0, RECORD_TYPE_FRAME_END, &[]);

        let gate = gate_compact_stream(&stream, 2);

        assert!(!gate.ok);
        assert!(gate.problems.contains(&"skipped_game_hours"));
        assert_eq!(gate.summary.skipped_game_hours, vec![11]);
    }

    #[test]
    fn compact_stream_gate_accepts_complete_consecutive_hours() {
        let mut stream = Vec::new();
        for (sequence, hour) in [(1, 20), (3, 21), (5, 22)] {
            let payload = encode_payload(
                sequence,
                0,
                1,
                1,
                &[MarketTradeRow::complete(sequence as u32, 2, 3)],
            );
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            append_compact_record(
                &mut stream,
                sequence + 1,
                hour,
                0,
                RECORD_TYPE_FRAME_END,
                &[],
            );
        }

        let gate = gate_compact_stream(&stream, 3);

        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.summary.domain_data_records, 3);
        assert_eq!(gate.summary.unique_game_hours, 3);
        assert_eq!(gate.summary.partial_field_rows_total, 0);
    }
}
