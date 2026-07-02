# Domain 43 MTD1 Detail Slice Integration Packet

This packet ships the already-validated `MTD1` purchase-contract detail slice
through the shared compact stream. It intentionally does not ship `MTLL`,
resource market, stockpile, non-market contracts, route/path fields, or any
other market residual.

## Scope

- Domain: `43` (`market_trade_live`).
- Wire shape: keep the existing 48-byte header and 48-byte purchase-contract
  rows, then append one optional `MTD1` trailer.
- Fold home: existing schema 7b `diplomacy_world.trade_contracts` and projected
  per-country `diplomacy.lend_lease`.
- R2-only fields: raw progress/detail values not named by schema 7b stay under
  the existing `r2_market_trade_live` object.
- Failure semantics: any per-row detail read skip sets `PARTIAL_DATA`; the
  shared fold treats source-unavailable/partial/failure as rowless no-advance
  for this hour. A successful zero-row market snapshot remains a resolved empty
  list, not `null`.
- Ownership: all new fields belong to the domain 43 market accumulator only.
- Build profile: use `compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID`;
  do not add a local owner-tag maximum.

## Evidence

- Lane README documents `MTD1` as magic `MTD1`, version 1, header 32, row 64,
  keyed by `contract_id`, with anti-fabrication flags.
- Validated offsets:
  - `CPurchaseContract+0xC0`: `speed_raw`
  - `CPurchaseContract+0x25C`: `completed_raw`
  - `CPurchaseContract+0x1D0`: `days`
  - `CPurchaseContract+0x5C`: delivery-country `tag_db_id`
  - `CPurchaseContract+0x118/+0x11C`: assigned/total convoys
  - `CPurchaseContract+0x1C0`: `efficiency_raw`
  - `CPurchaseContract+0x1E0`: factory CIC progress raw
  - `CPurchaseContract+0x1E8`: equipment-transfer progress raw
- Same-tick proof path:
  `extractor/native/runs/market_trade_live_contract_same_tick_20260701_v3/`.
  The lane accepted 63 detail rows, with 63/63 price, amount, speed,
  completed, days, delivery-country, efficiency, and progress rows plus 25
  convoy rows; replay verify reported zero skipped/partial/parse rows.
- Master contract already marks the purchase-contract slice production done and
  the detail work lane-local proved. One master-contract wording note still
  lags the lane proof by saying efficiency/progress are unaccepted; this packet
  follows the lane README and v3 residual proof.

## Edit 1: `extractor/native/collector/src/lib.rs`

### Add constants next to the existing market-trade constants

```rust
const COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
const COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION: u16 = 1;
const COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN: u16 = 32;
const COMPACT_MARKET_TRADE_DETAIL_ROW_LEN: u16 = 64;

const COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID: u32 = 1;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
```

### Add the detail row type near `CompactMarketTradeRow`

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CompactMarketTradeDetailRow {
    contract_id: u32,
    flags: u32,
    price_raw: i64,
    amount_raw: i64,
    speed_raw: i32,
    completed_raw: i32,
    days: i32,
    delivery_country_tag_db_id: u32,
    convoys_assigned: i32,
    convoys_total: i32,
    efficiency_raw: i32,
    factory_cic_progress_raw: i32,
    equipment_transfer_progress_raw: i32,
}
```

### Extend `CompactMarketTradeRead`

```rust
struct CompactMarketTradeRead {
    rows: Vec<CompactMarketTradeRow>,
    detail_rows: Vec<CompactMarketTradeDetailRow>,
    detail_skipped_rows: u32,
    read_time_us: u64,
    changed_rows: u32,
    status_flags: u32,
    observed_contract_rows: u32,
    emitted_contract_rows: u32,
    buyer_ptr_valid_rows: u32,
    variant_valid_rows: u32,
    amount_valid_rows: u32,
    partial_field_rows: u32,
}
```

Every early `CompactMarketTradeRead` return must initialize:

```rust
detail_rows: Vec::new(),
detail_skipped_rows: 0,
```

### Add detail read helpers near `read_compact_market_trade_row`

```rust
fn compact_market_trade_valid_tag_db_id(tag_db_id: u32) -> bool {
    tag_db_id != 0
        && tag_db_id <= compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
}

fn compact_market_trade_detail_i32(
    flags: &mut u32,
    flag: u32,
    addr: usize,
    min: i32,
    max: i32,
) -> Option<i32> {
    let raw = read_i32_stable(addr)?;
    if (min..=max).contains(&raw) {
        *flags |= flag;
        Some(raw)
    } else {
        Some(0)
    }
}

fn read_compact_market_trade_detail_row(
    contract_ptr: usize,
    core: &CompactMarketTradeRow,
) -> Option<CompactMarketTradeDetailRow> {
    let mut flags = 0u32;

    let mut row = CompactMarketTradeDetailRow {
        contract_id: core.contract_id,
        ..CompactMarketTradeDetailRow::default()
    };

    if core.flags & COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID != 0
        && core.amount_raw > 0
        && core.amount_raw <= MAX_MARKET_TRADE_AMOUNT_RAW
    {
        row.amount_raw = core.amount_raw;
        flags |= COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID;
    }

    let price_raw = read_i64_stable(contract_ptr + 0x68)?;
    if price_raw > 0 && price_raw <= MAX_MARKET_TRADE_AMOUNT_RAW {
        row.price_raw = price_raw;
        flags |= COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID;
    }

    if let Some(speed_raw) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID,
        contract_ptr + 0xC0,
        0,
        1_000_000_000,
    ) {
        row.speed_raw = speed_raw;
    }
    if let Some(completed_raw) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID,
        contract_ptr + 0x25C,
        0,
        1,
    ) {
        row.completed_raw = completed_raw;
    }
    if let Some(days) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID,
        contract_ptr + 0x1D0,
        -1,
        100_000,
    ) {
        row.days = days;
    }

    let delivery_country_tag_db_id = read_u32_stable(contract_ptr + 0x5C)?;
    if compact_market_trade_valid_tag_db_id(delivery_country_tag_db_id) {
        row.delivery_country_tag_db_id = delivery_country_tag_db_id;
        flags |= COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID;
    }

    let convoys_assigned = read_i32_stable(contract_ptr + 0x118)?;
    let convoys_total = read_i32_stable(contract_ptr + 0x11C)?;
    if convoys_assigned >= 0 && convoys_total >= 0 && convoys_assigned <= convoys_total {
        row.convoys_assigned = convoys_assigned;
        row.convoys_total = convoys_total;
        flags |= COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID;
    }

    if let Some(efficiency_raw) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID,
        contract_ptr + 0x1C0,
        0,
        COMPACT_FIXED_SCALE_RAW as i32,
    ) {
        row.efficiency_raw = efficiency_raw;
    }
    if let Some(factory_cic_progress_raw) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID,
        contract_ptr + 0x1E0,
        0,
        COMPACT_FIXED_SCALE_RAW as i32,
    ) {
        row.factory_cic_progress_raw = factory_cic_progress_raw;
    }
    if let Some(equipment_transfer_progress_raw) = compact_market_trade_detail_i32(
        &mut flags,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID,
        contract_ptr + 0x1E8,
        0,
        COMPACT_FIXED_SCALE_RAW as i32,
    ) {
        row.equipment_transfer_progress_raw = equipment_transfer_progress_raw;
    }

    row.flags = flags;
    if row.flags == 0 {
        None
    } else {
        Some(row)
    }
}
```

The `price_raw` address above must be confirmed against the shared collector's
current price root before pasting. If shared price extraction already uses a
different rooted field, keep the rooted shared price field and do not invent a
new price address.

### Update `read_compact_market_trade`

Inside the `for contract_ptr in contract_ptrs` loop, keep a pointer map for
emitted core rows:

```rust
let mut contract_ptr_by_contract_id: BTreeMap<u32, usize> = BTreeMap::new();
```

When inserting a non-conflicting core row:

```rust
contract_ptr_by_contract_id.insert(row.contract_id, contract_ptr);
rows_by_contract_id.insert(row.contract_id, row);
```

After `let rows = rows_by_contract_id.into_values().collect::<Vec<_>>();`,
build detail rows only from emitted core rows:

```rust
let mut detail_rows = Vec::new();
let mut detail_skipped_rows = 0u32;
for row in &rows {
    let Some(contract_ptr) = contract_ptr_by_contract_id.get(&row.contract_id).copied() else {
        detail_skipped_rows = detail_skipped_rows.saturating_add(1);
        continue;
    };
    match read_compact_market_trade_detail_row(contract_ptr, row) {
        Some(detail) => detail_rows.push(detail),
        None => detail_skipped_rows = detail_skipped_rows.saturating_add(1),
    }
}
if detail_skipped_rows > 0 {
    status_flags |= COMPACT_STATUS_PARTIAL_DATA;
}
```

Then return:

```rust
CompactMarketTradeRead {
    rows,
    detail_rows,
    detail_skipped_rows,
    read_time_us: duration_micros_u64(read_started.elapsed()),
    changed_rows,
    status_flags,
    observed_contract_rows,
    emitted_contract_rows,
    buyer_ptr_valid_rows,
    variant_valid_rows,
    amount_valid_rows,
    partial_field_rows,
}
```

### Append the trailer in `compact_market_trade_data_payload`

Increase capacity:

```rust
let mut payload = Vec::with_capacity(
    48usize
        .saturating_add(read.rows.len().saturating_mul(48))
        .saturating_add(if read.detail_rows.is_empty() {
            0
        } else {
            COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN as usize
                + read.detail_rows.len() * COMPACT_MARKET_TRADE_DETAIL_ROW_LEN as usize
        }),
);
```

After the core rows:

```rust
if !read.detail_rows.is_empty() && read.detail_skipped_rows == 0 {
    append_compact_market_trade_detail_trailer(&mut payload, &read.detail_rows);
}
```

Add the encoder:

```rust
fn append_compact_market_trade_detail_trailer(
    payload: &mut Vec<u8>,
    detail_rows: &[CompactMarketTradeDetailRow],
) {
    let price_valid = detail_rows
        .iter()
        .filter(|row| row.flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let speed_valid = detail_rows
        .iter()
        .filter(|row| row.flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let delivery_route_valid = detail_rows
        .iter()
        .filter(|row| {
            row.flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID)
                != 0
        })
        .count()
        .min(u32::MAX as usize) as u32;
    let progress_valid = detail_rows
        .iter()
        .filter(|row| {
            row.flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
                != 0
        })
        .count()
        .min(u32::MAX as usize) as u32;

    payload.extend_from_slice(COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC);
    payload.extend_from_slice(&COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN.to_le_bytes());
    payload.extend_from_slice(&COMPACT_MARKET_TRADE_DETAIL_ROW_LEN.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&(detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&price_valid.to_le_bytes());
    payload.extend_from_slice(&speed_valid.to_le_bytes());
    payload.extend_from_slice(&delivery_route_valid.to_le_bytes());
    payload.extend_from_slice(&progress_valid.to_le_bytes());

    for row in detail_rows {
        payload.extend_from_slice(&row.contract_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.price_raw.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&row.speed_raw.to_le_bytes());
        payload.extend_from_slice(&row.completed_raw.to_le_bytes());
        payload.extend_from_slice(&row.days.to_le_bytes());
        payload.extend_from_slice(&row.delivery_country_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.convoys_assigned.to_le_bytes());
        payload.extend_from_slice(&row.convoys_total.to_le_bytes());
        payload.extend_from_slice(&row.efficiency_raw.to_le_bytes());
        payload.extend_from_slice(&row.factory_cic_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.equipment_transfer_progress_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
}
```

## Edit 2: `extractor/native/recorder_host/src/main.rs`

### Replace local tag max and add detail constants

```rust
const COMPACT_MARKET_TRADE_MAX_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;

const COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
const COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION: u16 = 1;
const COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN: usize = 32;
const COMPACT_MARKET_TRADE_DETAIL_ROW_LEN: usize = 64;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID: u32 = 1;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
const COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
const COMPACT_MARKET_TRADE_DETAIL_FLAGS_KNOWN: u32 =
    COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;
```

### In `decode_compact_market_trade_data_payload`, allow and parse the trailer

Replace the exact-length check with:

```rust
let core_end = COMPACT_MARKET_TRADE_HEADER_LEN
    .saturating_add(row_count.saturating_mul(COMPACT_MARKET_TRADE_ROW_LEN));
if payload.len() < core_end {
    summary.parse_errors.push(format!(
        "offset {offset}: market trade data payload is {} byte(s), expected at least {core_end}",
        payload.len()
    ));
    return;
}
```

After the core row loop and counter checks, add:

```rust
let detail_rows = if cursor < payload.len() {
    decode_compact_market_trade_detail_trailer(offset, payload, cursor, summary)
} else {
    0
};
add_compact_domain_data_unit(summary, 43, "market_trade_detail_rows", detail_rows);
```

Add the decoder helper:

```rust
fn decode_compact_market_trade_detail_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut CompactSummary,
) -> usize {
    if payload.get(trailer_offset..trailer_offset + 4)
        != Some(COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC.as_slice())
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade trailing bytes are not MTD1 detail trailer"
        ));
        return 0;
    }
    if payload.len() < trailer_offset + COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 trailer header is truncated"
        ));
        return 0;
    }

    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16_le(payload, trailer_offset + 10);
    let row_count = read_u32_le(payload, trailer_offset + 12) as usize;
    let price_valid_rows = read_u32_le(payload, trailer_offset + 16) as usize;
    let speed_valid_rows = read_u32_le(payload, trailer_offset + 20) as usize;
    let delivery_route_valid_rows = read_u32_le(payload, trailer_offset + 24) as usize;
    let progress_valid_rows = read_u32_le(payload, trailer_offset + 28) as usize;

    if version != COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 version {version} != {}",
            COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION
        ));
    }
    if header_len != COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN
        || row_len != COMPACT_MARKET_TRADE_DETAIL_ROW_LEN
        || reserved0 != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 header shape is invalid"
        ));
    }

    let expected_len = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 len {} != {expected_len}",
            payload.len()
        ));
        return row_count;
    }

    let mut seen_contracts = BTreeSet::new();
    let mut computed_price = 0usize;
    let mut computed_speed = 0usize;
    let mut computed_delivery_route = 0usize;
    let mut computed_progress = 0usize;
    let mut computed_completed = 0usize;
    let mut computed_days = 0usize;
    let mut computed_delivery_country = 0usize;
    let mut computed_convoys = 0usize;
    let mut computed_efficiency = 0usize;
    let mut cursor = trailer_offset + header_len;
    for row_index in 0..row_count {
        let contract_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let price_raw = read_i64_le(payload, cursor + 8);
        let amount_raw = read_i64_le(payload, cursor + 16);
        let speed_raw = read_i32_le(payload, cursor + 24);
        let completed_raw = read_i32_le(payload, cursor + 28);
        let days = read_i32_le(payload, cursor + 32);
        let delivery_country_tag_db_id = read_u32_le(payload, cursor + 36);
        let convoys_assigned = read_i32_le(payload, cursor + 40);
        let convoys_total = read_i32_le(payload, cursor + 44);
        let efficiency_raw = read_i32_le(payload, cursor + 48);
        let factory_cic_progress_raw = read_i32_le(payload, cursor + 52);
        let equipment_transfer_progress_raw = read_i32_le(payload, cursor + 56);
        let reserved = read_u32_le(payload, cursor + 60);
        cursor += row_len;

        if contract_id == 0 || contract_id > COMPACT_MARKET_TRADE_MAX_CONTRACT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid contract_id {contract_id}"
            ));
        } else if !seen_contracts.insert(contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} duplicates contract_id {contract_id}"
            ));
        }
        if flags & !COMPACT_MARKET_TRADE_DETAIL_FLAGS_KNOWN != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} reserved is nonzero"
            ));
        }

        validate_market_trade_detail_presence_i64(
            summary,
            offset,
            row_index,
            "price_raw",
            price_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0,
            1,
            COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW,
        );
        validate_market_trade_detail_presence_i64(
            summary,
            offset,
            row_index,
            "amount_raw",
            amount_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID != 0,
            1,
            COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "speed_raw",
            speed_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0,
            0,
            1_000_000_000,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "completed_raw",
            completed_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0,
            0,
            1,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "days",
            days,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0,
            -1,
            100_000,
        );
        if flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
            if delivery_country_tag_db_id == 0
                || delivery_country_tag_db_id > COMPACT_MARKET_TRADE_MAX_TAG_DB_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTD1 row {row_index} has invalid delivery_country_tag_db_id {delivery_country_tag_db_id}"
                ));
            }
        } else if delivery_country_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has delivery_country without flag"
            ));
        }
        if flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0 {
            if convoys_assigned < 0 || convoys_total < 0 || convoys_assigned > convoys_total {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTD1 row {row_index} has invalid convoy values"
                ));
            }
        } else if convoys_assigned != 0 || convoys_total != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has convoys without flag"
            ));
        }
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "efficiency_raw",
            efficiency_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "factory_cic_progress_raw",
            factory_cic_progress_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "equipment_transfer_progress_raw",
            equipment_transfer_progress_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );

        computed_price += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0);
        computed_speed += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0);
        computed_completed +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0);
        computed_days += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0);
        computed_delivery_country +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0);
        computed_convoys += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0);
        computed_efficiency +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0);
        computed_delivery_route += usize::from(
            flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID)
                != 0,
        );
        computed_progress += usize::from(
            flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
                != 0,
        );
    }

    for (name, expected, actual) in [
        ("price_valid_rows", price_valid_rows, computed_price),
        ("speed_valid_rows", speed_valid_rows, computed_speed),
        (
            "delivery_route_valid_rows",
            delivery_route_valid_rows,
            computed_delivery_route,
        ),
        ("progress_valid_rows", progress_valid_rows, computed_progress),
    ] {
        if expected != actual {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 {name} {expected} != computed {actual}"
            ));
        }
    }

    add_compact_domain_data_unit(summary, 43, "market_trade_detail_price_valid_rows", computed_price);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_speed_valid_rows", computed_speed);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_completed_valid_rows", computed_completed);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_days_valid_rows", computed_days);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_delivery_country_valid_rows", computed_delivery_country);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_convoy_valid_rows", computed_convoys);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_efficiency_valid_rows", computed_efficiency);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_delivery_route_valid_rows", computed_delivery_route);
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_progress_valid_rows", computed_progress);
    row_count
}
```

Add the two small validators beside the helper above:

```rust
fn validate_market_trade_detail_presence_i64(
    summary: &mut CompactSummary,
    offset: usize,
    row_index: usize,
    name: &str,
    value: i64,
    present: bool,
    min: i64,
    max: i64,
) {
    if present {
        if !(min..=max).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 row {row_index} has {name} without flag"
        ));
    }
}

fn validate_market_trade_detail_presence_i32(
    summary: &mut CompactSummary,
    offset: usize,
    row_index: usize,
    name: &str,
    value: i32,
    present: bool,
    min: i32,
    max: i32,
) {
    if present {
        if !(min..=max).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 row {row_index} has {name} without flag"
        ));
    }
}
```

## Edit 3: `extractor/native/recorder_host/src/world_fold.rs`

### Add detail constants near the market constants

```rust
const MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
const MARKET_TRADE_DETAIL_TRAILER_VERSION: u16 = 1;
const MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN: usize = 32;
const MARKET_TRADE_DETAIL_ROW_LEN: usize = 64;
const MARKET_TRADE_DETAIL_FLAG_PRICE_VALID: u32 = 1;
const MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
const MARKET_TRADE_DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
const MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
const MARKET_TRADE_DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
const MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
const MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
const MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
const MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
const MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
const MARKET_TRADE_DETAIL_FLAGS_KNOWN: u32 = MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
    | MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
    | MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
    | MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
    | MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
    | MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
    | MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
    | MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
    | MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
    | MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;
```

### Add the fold row type near other state structs

```rust
#[derive(Clone, Copy)]
struct MarketTradeDetailFoldRow {
    contract_id: u32,
    flags: u32,
    price_raw: i64,
    amount_raw: i64,
    speed_raw: i32,
    completed_raw: i32,
    days: i32,
    delivery_country_tag_db_id: u32,
    convoys_assigned: i32,
    convoys_total: i32,
    efficiency_raw: i32,
    factory_cic_progress_raw: i32,
    equipment_transfer_progress_raw: i32,
}
```

### Replace the exact-length check in `fold_market_trade`

```rust
let core_end =
    MARKET_TRADE_HEADER_LEN.saturating_add(row_count.saturating_mul(MARKET_TRADE_ROW_LEN));
if payload.len() < core_end {
    return Err(format!("market trade len {} < {core_end}", payload.len()));
}
```

After parsing core rows, parse optional detail rows:

```rust
let detail_rows = if cursor < payload.len() {
    parse_market_trade_detail_trailer(payload, cursor)?
} else {
    BTreeMap::new()
};
```

When creating each contract object, apply detail by `contract_id`:

```rust
if let Some(detail) = detail_rows.get(&contract_id) {
    apply_market_trade_detail_to_contract(&mut contract, *detail);
}
```

### Add the detail parser and applier

```rust
fn parse_market_trade_detail_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<BTreeMap<u32, MarketTradeDetailFoldRow>, String> {
    if payload.get(trailer_offset..trailer_offset + 4)
        != Some(MARKET_TRADE_DETAIL_TRAILER_MAGIC.as_slice())
    {
        return Err("market trade trailing bytes are not MTD1 detail trailer".into());
    }
    need(
        payload,
        trailer_offset + MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN,
        "market trade MTD1 header",
    )?;
    let version = read_u16(payload, trailer_offset + 4);
    let header_len = read_u16(payload, trailer_offset + 6) as usize;
    let row_len = read_u16(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16(payload, trailer_offset + 10);
    let row_count = read_u32(payload, trailer_offset + 12) as usize;
    if version != MARKET_TRADE_DETAIL_TRAILER_VERSION
        || header_len != MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN
        || row_len != MARKET_TRADE_DETAIL_ROW_LEN
        || reserved0 != 0
    {
        return Err("market trade MTD1 header shape is invalid".into());
    }
    let expected = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() != expected {
        return Err(format!("market trade MTD1 len {} != {expected}", payload.len()));
    }

    let mut rows = BTreeMap::new();
    let mut cursor = trailer_offset + header_len;
    for row_index in 0..row_count {
        let row = MarketTradeDetailFoldRow {
            contract_id: read_u32(payload, cursor),
            flags: read_u32(payload, cursor + 4),
            price_raw: read_i64(payload, cursor + 8),
            amount_raw: read_i64(payload, cursor + 16),
            speed_raw: read_i32(payload, cursor + 24),
            completed_raw: read_i32(payload, cursor + 28),
            days: read_i32(payload, cursor + 32),
            delivery_country_tag_db_id: read_u32(payload, cursor + 36),
            convoys_assigned: read_i32(payload, cursor + 40),
            convoys_total: read_i32(payload, cursor + 44),
            efficiency_raw: read_i32(payload, cursor + 48),
            factory_cic_progress_raw: read_i32(payload, cursor + 52),
            equipment_transfer_progress_raw: read_i32(payload, cursor + 56),
        };
        let reserved = read_u32(payload, cursor + 60);
        cursor += row_len;
        if row.contract_id == 0 {
            return Err(format!("market trade MTD1 row {row_index} has zero contract_id"));
        }
        if row.flags & !MARKET_TRADE_DETAIL_FLAGS_KNOWN != 0 {
            return Err(format!(
                "market trade MTD1 row {row_index} has unknown flags {:#x}",
                row.flags
            ));
        }
        if reserved != 0 {
            return Err(format!("market trade MTD1 row {row_index} reserved is nonzero"));
        }
        if rows.insert(row.contract_id, row).is_some() {
            return Err(format!(
                "market trade MTD1 row {row_index} duplicates contract_id {}",
                row.contract_id
            ));
        }
    }
    Ok(rows)
}

fn apply_market_trade_detail_to_contract(
    contract: &mut Map<String, Value>,
    detail: MarketTradeDetailFoldRow,
) {
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0 {
        contract.insert(
            "prices".into(),
            json!([{
                "amount": detail.price_raw as f64 / FIXED_SCALE,
                "amount_raw": detail.price_raw
            }]),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0 {
        contract.insert("speed".into(), json!(detail.speed_raw as f64 / FIXED_SCALE));
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
        contract.insert(
            "delivery_country".into(),
            json!(detail.delivery_country_tag_db_id.to_string()),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0 {
        contract.insert(
            "convoys".into(),
            json!({
                "assigned": detail.convoys_assigned,
                "total": detail.convoys_total,
            }),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0 {
        contract.insert(
            "delivery_efficiency".into(),
            json!(detail.efficiency_raw as f64 / FIXED_SCALE),
        );
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0 {
        contract.insert("days".into(), json!(detail.days));
    }
    if detail.flags & MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0 {
        contract.insert("completed".into(), json!(detail.completed_raw != 0));
    }

    let r2 = ensure_json_object_mut(contract, "r2_market_trade_live");
    r2.insert("detail_flags".into(), json!(detail.flags));
    r2.insert("amount_raw".into(), json!(detail.amount_raw));
    r2.insert("speed_raw".into(), json!(detail.speed_raw));
    r2.insert("completed_raw".into(), json!(detail.completed_raw));
    r2.insert("efficiency_raw".into(), json!(detail.efficiency_raw));
    r2.insert(
        "factory_cic_progress_raw".into(),
        json!(detail.factory_cic_progress_raw),
    );
    r2.insert(
        "equipment_transfer_progress_raw".into(),
        json!(detail.equipment_transfer_progress_raw),
    );
}
```

`resolve_contract_fields` already includes `delivery_country`, so no resolver
change is required if the detail applier stores it as a numeric string.

## Draft tests

### Collector unit test

```rust
#[test]
fn compact_market_trade_payload_appends_mtd1_detail_rows() {
    let flags = COMPACT_MARKET_TRADE_ROW_FLAG_BUYER_PTR_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_EQUIPMENT_SUBBUF_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_DELIVERY_SUBBUF_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_VARIANT_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID;
    let read = CompactMarketTradeRead {
        rows: vec![CompactMarketTradeRow {
            contract_id: 7,
            contract_type: PURCHASE_CONTRACT_TYPE,
            flags,
            seller_tag_db_id: 1,
            buyer_tag_db_id: 2,
            equipment_variant_id: 99,
            amount_raw: 250_000,
            equipment_subbuf_status: COMPACT_MARKET_TRADE_SUBBUF_STATUS_VALID,
            delivery_subbuf_status: COMPACT_MARKET_TRADE_SUBBUF_STATUS_VALID,
            buyer_ptr_status: COMPACT_MARKET_TRADE_PTR_STATUS_VALID,
        }],
        detail_rows: vec![CompactMarketTradeDetailRow {
            contract_id: 7,
            flags: COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
                | COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
                | COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
                | COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
                | COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
                | COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID,
            price_raw: 500_000,
            amount_raw: 250_000,
            speed_raw: 100_000,
            completed_raw: 0,
            days: 30,
            delivery_country_tag_db_id: 2,
            ..CompactMarketTradeDetailRow::default()
        }],
        detail_skipped_rows: 0,
        read_time_us: 0,
        changed_rows: 1,
        status_flags: 0,
        observed_contract_rows: 1,
        emitted_contract_rows: 1,
        buyer_ptr_valid_rows: 1,
        variant_valid_rows: 1,
        amount_valid_rows: 1,
        partial_field_rows: 0,
    };
    let payload = compact_market_trade_data_payload(&read, 123);
    assert_eq!(&payload[48 + 48..48 + 48 + 4], b"MTD1");
    assert_eq!(u32::from_le_bytes(payload[48 + 48 + 12..48 + 48 + 16].try_into().unwrap()), 1);
}
```

### Recorder decoder tests

```rust
#[test]
fn compact_summary_decodes_market_trade_mtd1_detail_rows() {
    let mut payload = test_market_trade_payload_with_one_core_row();
    append_test_mtd1_row(
        &mut payload,
        7,
        COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
            | COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
            | COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
            | COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
            | COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
            | COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID,
    );
    let mut summary = CompactSummary::default();
    decode_compact_market_trade_data_payload(0, &payload, &mut summary);
    assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
    assert_eq!(
        compact_domain_data_unit(&summary, 43, "market_trade_detail_rows"),
        1
    );
}

#[test]
fn compact_summary_rejects_unflagged_market_trade_mtd1_speed() {
    let mut payload = test_market_trade_payload_with_one_core_row();
    append_test_mtd1_row(&mut payload, 7, COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID);
    let row_start = COMPACT_MARKET_TRADE_HEADER_LEN + COMPACT_MARKET_TRADE_ROW_LEN
        + COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN;
    payload[row_start + 24..row_start + 28].copy_from_slice(&100_000i32.to_le_bytes());
    let mut summary = CompactSummary::default();
    decode_compact_market_trade_data_payload(0, &payload, &mut summary);
    assert!(summary
        .parse_errors
        .iter()
        .any(|err| err.contains("speed_raw without flag")));
}
```

### World fold test

```rust
#[test]
fn world_fold_market_trade_mtd1_populates_trade_contract_detail_fields() {
    let mut fold = WorldFold::default();
    fold.country_tags_by_db_id.insert(1, "GER".to_string());
    fold.country_tags_by_db_id.insert(2, "SWE".to_string());

    let mut payload = test_market_trade_payload_with_one_core_row();
    append_test_mtd1_row(
        &mut payload,
        7,
        MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
            | MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
            | MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
            | MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
            | MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
            | MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
            | MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
            | MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID,
    );

    fold.fold_market_trade(&payload).unwrap();
    let mut unresolved = BTreeSet::new();
    let mut contracts = fold.market_trade_contracts.clone().unwrap();
    fold.resolve_contract_list(&mut contracts, &mut unresolved);
    let contract = contracts[0].as_object().unwrap();
    assert_eq!(contract.get("delivery_country").unwrap(), "SWE");
    assert!(contract.get("prices").unwrap().is_array());
    assert!(contract.get("convoys").unwrap().is_object());
    assert!(contract
        .get("r2_market_trade_live")
        .unwrap()
        .get("factory_cic_progress_raw")
        .is_some());
}
```

## Validation commands

Run after applying the shared patch:

```powershell
cargo test --manifest-path extractor\native\collector\Cargo.toml compact_market_trade
cargo test --manifest-path extractor\native\recorder_host\Cargo.toml market_trade
cargo test --manifest-path extractor\native\lanes\market_trade_live\Cargo.toml
```

## Riskiest assumptions

- The shared collector price root must be confirmed before pasting the helper.
  The lane proves the trailer contract and price semantics; this packet leaves a
  guard note so the implementer does not invent a new `price_raw` address.
- The master contract has stale wording about efficiency/progress. The lane
  README and v3 proof support those fields, but this should be called out in
  review.
- `speed_raw` is folded as fixed-point `speed` plus raw R2. If downstream
  consumers expect a different unit, keep the raw R2 field stable and adjust
  only the first-class display value.
