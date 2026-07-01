# Domain 44 CSupply Shared Registration Patch

This is the lane-local apply plan for registering the proven
`CSupplySystem+0x10` rows into the one shared collector and recorder.

Do not create a second collector or DLL. Apply these edits only to the existing
domain 44 path.

Post-registration hardening note: the first shared live run proved these rows
decode, but the scheduled read was too slow because the helper below used
heap-wide vtable discovery inside the domain read. `CSUPPLY_ROOT_HARDENING_PATCH.md`
supersedes that discovery path with the validated direct root
`CSupplySystem = u64[CGameState+0x3D8]`.

## Collector: `replay_native/collector/src/lib.rs`

### Constants

At the existing compact supply constants around line `3030`, add the row kind,
flags, CSupply RVAs, and bounded row constants:

```rust
const COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4;
const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17;
const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18;
const CGAMESTATE_CSUPPLY_SYSTEM: usize = 0x3D8;
const CSUPPLY_SYSTEM_VTABLE0_RVA: usize = 0x295D0A8;
const CSUPPLY_SYSTEM_VTABLE1_RVA: usize = 0x295D0D0;
const CSUPPLY_SYSTEM_PROVINCE_INPUT_A: usize = 0x10;
const COMPACT_SUPPLY_CSUPPLY_INPUT_STRIDE: usize = 8;
const COMPACT_SUPPLY_CSUPPLY_INPUT_COUNT: u32 = COMPACT_SUPPLY_MAX_PROVINCE_ID + 1;
const COMPACT_SUPPLY_CSUPPLY_INPUT_MAX_RAW: i32 = COMPACT_FIXED_SCALE_RAW as i32;
```

### Read struct and panic fallback

Add this field to `CompactSupplyLogisticsAttritionRead`:

```rust
csupply_input_rows: u32,
```

Set it to zero in `compact_supply_logistics_attrition_worker_panic_read`:

```rust
csupply_input_rows: 0,
```

### Domain read function

In `read_compact_supply_logistics_attrition`, append CSupply rows after division
rows and before badge rows:

```rust
let mut rows = province_rows;
let province_row_count = rows.len().min(u32::MAX as usize) as u32;
let division_row_count = division_rows.len().min(u32::MAX as usize) as u32;
rows.append(&mut division_rows);

let mut csupply_input_rows =
    compact_supply_logistics_attrition_csupply_input_rows(
        state.base,
        state.date_ptr as usize,
        tracker,
    );
let csupply_input_row_count = csupply_input_rows.len().min(u32::MAX as usize) as u32;
rows.append(&mut csupply_input_rows);

let mut badge_rows = compact_supply_logistics_attrition_badge_rows();
let badge_row_count = badge_rows.len().min(u32::MAX as usize) as u32;
rows.append(&mut badge_rows);
```

Then set the field in the returned `CompactSupplyLogisticsAttritionRead`:

```rust
csupply_input_rows: csupply_input_row_count,
```

### Collector helper

Insert the direct-root helper from `CSUPPLY_ROOT_HARDENING_PATCH.md` beside the
existing domain 44 supply helpers, before
`compact_supply_logistics_attrition_badge_rows`. The older heap-scan helper below
is retained only as historical registration context and must not run inside
scheduled recording:

```rust
fn compact_supply_logistics_attrition_csupply_input_rows(
    module_base: usize,
) -> Vec<CompactSupplyLogisticsAttritionRow> {
    let Some(vtable0) = module_base.checked_add(CSUPPLY_SYSTEM_VTABLE0_RVA) else {
        return Vec::new();
    };
    let Some(vtable1) = module_base.checked_add(CSUPPLY_SYSTEM_VTABLE1_RVA) else {
        return Vec::new();
    };

    for object_ptr in scan_qword_hits(vtable0, HEAP_LO, HEAP_HI) {
        if read_u64(object_ptr.saturating_add(8)).map(|value| value as usize) != Some(vtable1) {
            continue;
        }
        if let Some(rows) =
            compact_supply_logistics_attrition_csupply_input_rows_from_object(object_ptr)
        {
            return rows;
        }
    }

    Vec::new()
}

fn compact_supply_logistics_attrition_csupply_input_rows_from_object(
    object_ptr: usize,
) -> Option<Vec<CompactSupplyLogisticsAttritionRow>> {
    let array = read_u64(object_ptr.checked_add(CSUPPLY_SYSTEM_PROVINCE_INPUT_A)?)? as usize;
    let count = read_u32(object_ptr.checked_add(CSUPPLY_SYSTEM_PROVINCE_INPUT_A + 8)?)?;
    if count != COMPACT_SUPPLY_CSUPPLY_INPUT_COUNT {
        return None;
    }
    if !is_heap_pointer_aligned(array, 8) {
        return None;
    }

    let byte_len = (count as usize).checked_mul(COMPACT_SUPPLY_CSUPPLY_INPUT_STRIDE)?;
    let blob = read_bytes(array, byte_len)?;
    if blob.len() != byte_len {
        return None;
    }

    let mut rows = Vec::with_capacity(count.saturating_sub(1) as usize);
    for province_id in 1..count {
        let offset = (province_id as usize).checked_mul(COMPACT_SUPPLY_CSUPPLY_INPUT_STRIDE)?;
        let Some(raw) = read_i32_from_slice(&blob, offset) else {
            continue;
        };
        if !(0..=COMPACT_SUPPLY_CSUPPLY_INPUT_MAX_RAW).contains(&raw) {
            continue;
        }
        rows.push(CompactSupplyLogisticsAttritionRow {
            row_kind: COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT,
            entity_id: province_id,
            join_id: 0,
            owner_tag_db_id: 0,
            flags: COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
                | COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
            value_a_raw: raw,
            value_b_raw: 0,
            value_c_raw: 0,
            value_d_raw: 0,
            aux_raw: 0,
        });
    }

    Some(rows)
}
```

This helper intentionally emits zero rows when the root, array shape, or bounds
fail. That preserves fail-closed behavior and leaves the mandatory CSupply badge
row in the stream.

### Payload header

In `compact_supply_logistics_attrition_data_payload`, replace the final header
reserved zero:

```rust
payload.extend_from_slice(&0u32.to_le_bytes());
```

with:

```rust
payload.extend_from_slice(&read.csupply_input_rows.to_le_bytes());
```

### Collector test edit

In `compact_supply_logistics_attrition_payload_packs_rows_and_badges`, add one
row kind `4` before appending badge rows:

```rust
CompactSupplyLogisticsAttritionRow {
    row_kind: COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT,
    entity_id: 3544,
    join_id: 0,
    owner_tag_db_id: 0,
    flags: COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
        | COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
    value_a_raw: 72_500,
    value_b_raw: 0,
    value_c_raw: 0,
    value_d_raw: 0,
    aux_raw: 0,
},
```

Update the test read counters from `7` to `8`, set
`csupply_input_rows: 1`, and assert header bytes `44..48` equal `1`.

## Recorder: `replay_native/recorder_host/src/main.rs`

### Constants

At the compact supply constants around line `145`, add:

```rust
const COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4;
const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17;
const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18;
const COMPACT_SUPPLY_MAX_CSUPPLY_INPUT_RAW: i32 = COMPACT_FIXED_SCALE_RAW as i32;
```

Add the two CSupply flags to `COMPACT_SUPPLY_ROW_VALID_FLAGS`.

### Header decode

In `decode_compact_supply_logistics_attrition_data_payload`, replace:

```rust
let reserved0 = read_u32_le(payload, 44);
```

with:

```rust
let csupply_input_rows = read_u32_le(payload, 44) as usize;
```

Delete the reserved-zero parse error block. Change the row-count invariant to:

```rust
if province_rows
    .saturating_add(division_rows)
    .saturating_add(badge_rows)
    .saturating_add(csupply_input_rows)
    != row_count
{
    summary.parse_errors.push(format!(
        "offset {offset}: supply logistics row counters {province_rows}+{division_rows}+{badge_rows}+{csupply_input_rows} != row_count {row_count}"
    ));
}
```

Add the header-count summary unit beside the existing supply units:

```rust
add_compact_domain_data_unit(summary, 44, "supply_csupply_input_rows", csupply_input_rows);
```

### Row loop decode

Add state before the loop:

```rust
let mut decoded_csupply_input_rows = 0usize;
let mut seen_csupply_input_provinces = BTreeSet::new();
```

Add this match arm before the badge arm:

```rust
COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT => {
    decoded_csupply_input_rows = decoded_csupply_input_rows.saturating_add(1);
    if entity_id == 0 || entity_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply row {row_index} has invalid province_id {entity_id}"
        ));
    } else if !seen_csupply_input_provinces.insert(entity_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply row {row_index} duplicates province_id {entity_id}"
        ));
    }
    if join_id != 0 || owner_tag_db_id != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply row {row_index} has nonzero identity join fields"
        ));
    }
    let expected_flags = COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
        | COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY;
    if flags != expected_flags {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply row {row_index} has invalid flags {flags}"
        ));
    }
    validate_supply_logistics_nonnegative_value(
        offset,
        summary,
        row_index,
        "csupply_input_raw",
        value_a_raw,
        COMPACT_SUPPLY_MAX_CSUPPLY_INPUT_RAW,
    );
    if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply row {row_index} has nonzero reserved value fields"
        ));
    }
}
```

After the existing decoded counter checks, add:

```rust
if decoded_csupply_input_rows != csupply_input_rows {
    summary.parse_errors.push(format!(
        "offset {offset}: supply logistics csupply_input_rows {csupply_input_rows} != decoded {decoded_csupply_input_rows}"
    ));
}
```

Add the decoded summary unit at the end:

```rust
add_compact_domain_data_unit(
    summary,
    44,
    "supply_csupply_efficiency_rows",
    decoded_csupply_input_rows,
);
```

### Recorder tests

In `test_supply_logistics_attrition_data_payload`, add one row kind `4` after
the division row and before badge rows:

```rust
(
    COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT,
    3544u32,
    0u32,
    0u32,
    COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
        | COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
    72_500i32,
    0i32,
    0i32,
    0i32,
    0i32,
),
```

Change the header writer at bytes `44..48` from `0u32` to `1u32`.

Update the existing domain 44 coverage assertions:

```rust
assert_eq!(supply.unit_count_total, 9);
assert_eq!(supply.data_rows_total, 8);
assert_eq!(supply.data_changed_rows_total, 8);
assert_eq!(
    supply.data_units_total.get("supply_csupply_input_rows").copied(),
    Some(1)
);
assert_eq!(
    supply
        .data_units_total
        .get("supply_csupply_efficiency_rows")
        .copied(),
    Some(1)
);
```

Add one negative test:

```rust
#[test]
fn compact_supply_logistics_bad_csupply_row_fails_decode() {
    let mut payload = test_supply_logistics_attrition_data_payload(0);
    let csupply_cursor = COMPACT_SUPPLY_LOGISTICS_HEADER_LEN + 2 * COMPACT_SUPPLY_LOGISTICS_ROW_LEN;
    payload[csupply_cursor + 16..csupply_cursor + 20]
        .copy_from_slice(&COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY.to_le_bytes());
    let mut summary = CompactSummary::default();

    decode_compact_supply_logistics_attrition_data_payload(0, &payload, &mut summary);

    assert!(summary
        .parse_errors
        .iter()
        .any(|err| err.contains("csupply row 2 has invalid flags")));
}
```

## Required shared proof after applying

```powershell
cargo test --manifest-path replay_native\recorder_host\Cargo.toml
cargo test --manifest-path replay_native\snapshot\Cargo.toml
```

Then run the shared injected collector at the normal cadence and require:

- domain `44` compact rows include row kind `4`;
- shared recorder reports `supply_csupply_input_rows > 0`;
- shared recorder reports `supply_csupply_efficiency_rows > 0`;
- `.battleplan` verifies;
- hourly-or-lower frame cadence at speed five/current equivalent;
- zero skipped game hours;
- no source/worker/bridge/partial failures.
