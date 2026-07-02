# Domain 45 Country-Target Decision Rows Integration Packet

This packet ships only the already-validated country-target rows for active
targeted timed decisions through the shared compact stream. It closes the known
gap where the lane-local `DecisionTargetRow` extension exists but the active
shared recorder decoder still ignores it.

## Scope

- Domain: `45` (`focus_research_decisions_depth_live`).
- Row family: country-target rows derived from
  `CDecisionStatus+0xE8 -> active targeted timed records`.
- Validated target field: record vtable `0x27D5038`, `record+0x20`
  country-target `tag_db_id`.
- Out of scope, fail closed: numeric state/province target ids, cooldowns,
  days/state body fields, body-layout fields, and unmatched transition rows.
- Fold home: schema 3.7 does not define `countries[*].decisions.targets`, so
  fold rows under `countries[*].decisions._r2_depth_targets`.
- Failure semantics: structural per-row read skips for this row family set
  `PARTIAL_DATA`, and partial/source-unavailable frames emit no target rows.
  Rows that are not country-target rows are out-of-scope omissions, not partial
  data.
- Ownership: extend only domain 45 focus/research/decision-depth state.
- Build profile: use `compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID`;
  do not add a local owner-tag maximum.

## Evidence

- Master contract row: `CDecisionStatus+0xE8`, vtable `0x27D5038`,
  `record+0x20` country-target `tag_db_id`, 1061 exact save/live matched rows.
- Lane README: full live targeted timed sample had 3384 rows; the validated
  country-target subset was 1061 exact matched rows across 180 tokens. Numeric
  rows and cooldown/body fields remain open.
- Lane-local compact gate already has `DecisionTargetRow` with fixed fields
  `owner_tag_db_id, target_id, flags, source_kind, decision`; chunk 3 did not
  expose it through the active shared recorder decoder.

## Edit 1: `extractor/native/collector/src/lib.rs`

### Add constants near the FRD constants

```rust
const COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN: usize = 16;
const COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
const COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
const COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
const COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS: u32 =
    COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG
        | COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20;

const TARGETED_TIMED_DECISION_VTABLE_RVA: usize = 0x27D5038;
const TARGETED_TIMED_DECISION_DEF: usize = 0x10;
const TARGETED_TIMED_DECISION_COUNTRY_TARGET: usize = 0x20;
```

Do not add numeric target flags to the shared production slice. If the lane's
numeric work is later accepted, it must be a separate widening.

### Add state and row types near the existing decision structs

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
struct CompactFrdDecisionTargetRow {
    tag_db_id: u32,
    target_id: u32,
    flags: u32,
    source_kind: u32,
    decision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompactFrdTargetedTimedDecisionState {
    decision: String,
    target_tag_db_id: u32,
}
```

Extend the existing structs:

```rust
struct CompactFrdDepthRead {
    focus_events: Vec<CompactFrdFocusEventRow>,
    research_eta: Vec<CompactFrdResearchEtaRow>,
    decision_status: Vec<CompactFrdDecisionStatusRow>,
    decision_events: Vec<CompactFrdDecisionEventRow>,
    decision_targets: Vec<CompactFrdDecisionTargetRow>,
    badges: Vec<CompactFrdBadgeRow>,
    read_time_us: u64,
    changed_rows: u32,
    status_flags: u32,
    depth_status_flags: u32,
    observed_focus_rows: u32,
    observed_research_slot_rows: u32,
    observed_decision_rows: u32,
    observed_timed_decision_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompactFrdDecisionCountryState {
    tag_db_id: u32,
    active_timed_count: u32,
    targeted_count: u32,
    targeted_timed_count: u32,
    decisions_taken_bucket_count: u32,
    re_enable_count: u32,
    timed_live: Vec<CompactFrdTimedDecisionState>,
    targeted_timed_live: Vec<CompactFrdTargetedTimedDecisionState>,
}

#[derive(Clone)]
struct CountryDecisionsRow {
    status_present: Option<BTreeMap<String, u32>>,
    timed_live: Option<Vec<TimedDecisionRow>>,
    targeted_timed_live: Option<Vec<TargetedTimedDecisionRow>>,
}

#[derive(Clone)]
struct TargetedTimedDecisionRow {
    decision: String,
    target_tag_db_id: u32,
}
```

Update `CountryDecisionsRow::has_any`:

```rust
fn has_any(&self) -> bool {
    self.status_present.is_some()
        || self.timed_live.is_some()
        || self.targeted_timed_live.is_some()
}
```

Every `CompactFrdDepthRead` initializer must include:

```rust
decision_targets: Vec::new(),
```

Every `CompactFrdDecisionCountryState` initializer must include:

```rust
targeted_timed_live,
```

## Collector read path

### Extend `read_country_decisions`

```rust
fn read_country_decisions(
    object_ptr: usize,
    country_vtable: usize,
    module_base: usize,
) -> Option<CountryDecisionsRow> {
    let (status_ptr, timed_decision_vtable) =
        validated_decision_status_ptr(object_ptr, country_vtable)?;

    let status_present = decision_status_present(status_ptr);
    let timed_live = read_active_timed_decisions(status_ptr, timed_decision_vtable);
    let targeted_timed_live =
        read_active_targeted_timed_country_targets(status_ptr, module_base);
    let row = CountryDecisionsRow {
        status_present,
        timed_live,
        targeted_timed_live,
    };
    if row.has_any() {
        Some(row)
    } else {
        None
    }
}
```

Update callers to pass `state.base`.

### Add the country-target reader

```rust
fn compact_frd_valid_target_tag_db_id(tag_db_id: u32) -> bool {
    tag_db_id != 0
        && tag_db_id <= compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
}

fn read_active_targeted_timed_country_targets(
    status_ptr: usize,
    module_base: usize,
) -> Option<Vec<TargetedTimedDecisionRow>> {
    let targeted_vtable = module_base.checked_add(TARGETED_TIMED_DECISION_VTABLE_RVA)?;
    let header_addr = status_ptr.checked_add(DECISION_STATUS_TARGETED_TIMED)?;
    let (data, size, cap) = read_stable_vector_header(header_addr, 0, 0x0C, 0x08)?;
    if size > cap || cap > MAX_DECISION_STATUS_LIST || size as usize > MAX_TIMED_DECISIONS {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    if !is_map_control_heap_pointer(data) {
        return None;
    }

    let mut out = Vec::new();
    for index in 0..size as usize {
        let rec_ptr = read_u64_stable(data + index * 8)? as usize;
        if !is_map_control_heap_pointer(rec_ptr) {
            return None;
        }
        if read_u64_stable(rec_ptr).map(|value| value as usize) != Some(targeted_vtable) {
            return None;
        }
        let def_ptr = read_u64_stable(rec_ptr + TARGETED_TIMED_DECISION_DEF)? as usize;
        if !is_map_control_heap_pointer(def_ptr) {
            return None;
        }
        let decision = cached_decision_token(def_ptr)?;
        let target_tag_db_id =
            read_u32_stable(rec_ptr + TARGETED_TIMED_DECISION_COUNTRY_TARGET)?;
        if !compact_frd_valid_target_tag_db_id(target_tag_db_id) {
            // Numeric or otherwise unvalidated target shape. It is intentionally
            // outside this slice, so omit it without fabricating a partial row.
            continue;
        }
        out.push(TargetedTimedDecisionRow {
            decision,
            target_tag_db_id,
        });
    }
    Some(out)
}
```

If the shared collector cannot distinguish a structural read failure from a
non-country target at this helper boundary, prefer returning `None`, and have
the caller set `PARTIAL_DATA` and emit no target rows. Do not emit guessed
country targets.

### Extend `compact_frd_apply_decisions_row`

After `timed_live` is built:

```rust
let targeted_timed_live = decisions
    .targeted_timed_live
    .as_ref()
    .map(|rows| {
        rows.iter()
            .map(|targeted| CompactFrdTargetedTimedDecisionState {
                decision: targeted.decision.clone(),
                target_tag_db_id: targeted.target_tag_db_id,
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
```

Add `targeted_timed_live` to `CompactFrdDecisionCountryState`, then after the
status row is pushed:

```rust
for targeted in &state.targeted_timed_live {
    read.decision_targets.push(CompactFrdDecisionTargetRow {
        tag_db_id: state.tag_db_id,
        target_id: targeted.target_tag_db_id,
        flags: COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS,
        source_kind: COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
        decision: Some(targeted.decision.clone()),
    });
}
```

The status count remains the live `active_targeted_timed_decision` count. The
target row count is the validated country subset and may be lower.

### Extend debug JSON only if present

In `country_decisions_json`:

```rust
if let Some(targeted_timed_live) = decisions.targeted_timed_live.as_ref() {
    let parts: Vec<String> = targeted_timed_live
        .iter()
        .map(|row| {
            format!(
                "{{\"decision\":{},\"target_tag_db_id\":{}}}",
                json_str(&row.decision),
                row.target_tag_db_id
            )
        })
        .collect();
    fields.push(format!("\"targeted_timed_live\":[{}]", parts.join(",")));
}
```

## Compact payload migration

The active shared domain 45 header still starts with `elapsed_wall_ms`. The
lane-local extension starts with `decision_target_count`. Ship this only as a
coordinated collector/recorder/fold change.

Replace the header write in `compact_frd_depth_data_payload` with:

```rust
payload.extend_from_slice(
    &(read.decision_targets.len().min(u32::MAX as usize) as u32).to_le_bytes(),
);
payload.extend_from_slice(&0u32.to_le_bytes()); // schema_flags_reserved_zero
payload.extend_from_slice(&read.depth_status_flags.to_le_bytes());
payload.extend_from_slice(
    &(read.focus_events.len().min(u32::MAX as usize) as u32).to_le_bytes(),
);
payload.extend_from_slice(
    &(read.research_eta.len().min(u32::MAX as usize) as u32).to_le_bytes(),
);
payload.extend_from_slice(
    &(read.decision_status.len().min(u32::MAX as usize) as u32).to_le_bytes(),
);
payload.extend_from_slice(
    &(read.decision_events.len().min(u32::MAX as usize) as u32).to_le_bytes(),
);
payload.extend_from_slice(&read.observed_focus_rows.to_le_bytes());
payload.extend_from_slice(&read.observed_research_slot_rows.to_le_bytes());
payload.extend_from_slice(&read.observed_decision_rows.to_le_bytes());
payload.extend_from_slice(&read.observed_timed_decision_rows.to_le_bytes());
payload.extend_from_slice(&(read.badges.len().min(u32::MAX as usize) as u32).to_le_bytes());
```

Append target rows after decision event rows and before badge rows:

```rust
for row in &read.decision_targets {
    payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
    payload.extend_from_slice(&row.target_id.to_le_bytes());
    payload.extend_from_slice(&row.flags.to_le_bytes());
    payload.extend_from_slice(&row.source_kind.to_le_bytes());
    append_compact_string(&mut payload, row.decision.as_deref());
}
```

Update capacity to include:

```rust
.saturating_add(
    read.decision_targets
        .len()
        .saturating_mul(COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN),
)
```

## Edit 2: `extractor/native/recorder_host/src/main.rs`

### Add constants

```rust
const COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN: usize = 16;
const COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
const COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
const COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
const COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS: u32 =
    COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG
        | COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20;
const COMPACT_FRD_MAX_TARGET_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
```

### Replace the domain 45 header parser

At the top of `decode_compact_frd_depth_data_payload`:

```rust
let decision_target_count = read_u32_le(payload, 0) as usize;
let schema_flags_reserved_zero = read_u32_le(payload, 4);
let depth_status_flags = read_u32_le(payload, 8);
let focus_count = read_u32_le(payload, 12) as usize;
let research_count = read_u32_le(payload, 16) as usize;
let decision_status_count = read_u32_le(payload, 20) as usize;
let decision_event_count = read_u32_le(payload, 24) as usize;
let observed_focus_rows = read_u32_le(payload, 28);
let observed_research_slot_rows = read_u32_le(payload, 32) as usize;
let observed_decision_rows = read_u32_le(payload, 36);
let observed_timed_decision_rows = read_u32_le(payload, 40) as usize;
let badge_count = read_u32_le(payload, 44) as usize;
if schema_flags_reserved_zero != 0 {
    summary.parse_errors.push(format!(
        "offset {offset}: focus/research/decisions depth schema_flags_reserved_zero is nonzero"
    ));
}
let emitted_rows = focus_count
    .saturating_add(research_count)
    .saturating_add(decision_status_count)
    .saturating_add(decision_event_count)
    .saturating_add(decision_target_count)
    .saturating_add(badge_count);
```

Add data units:

```rust
add_compact_domain_data_unit(
    summary,
    COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
    "decision_target_rows",
    decision_target_count,
);
```

### Parse target rows before badges

After the decision event row loop and before badge parsing:

```rust
let mut targeted_timed_status_count_by_tag = BTreeMap::new();
for row in &decoded_decision_status_rows {
    targeted_timed_status_count_by_tag.insert(row.tag_db_id, row.targeted_timed_count);
}

let mut decision_target_rows_by_tag: BTreeMap<u32, u32> = BTreeMap::new();
let mut seen_decision_targets = BTreeSet::new();
let mut country_target_rows = 0usize;
for row_index in 0..decision_target_count {
    if let Err(err) = require_compact_remaining(
        payload,
        cursor,
        COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN,
        "decision target row",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return;
    }
    let tag_db_id = read_u32_le(payload, cursor);
    let target_id = read_u32_le(payload, cursor + 4);
    let flags = read_u32_le(payload, cursor + 8);
    let source_kind = read_u32_le(payload, cursor + 12);
    cursor += COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN;
    let decision = match read_compact_string(payload, &mut cursor, "decision target token")
    {
        Ok(decision) => decision,
        Err(err) => {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    };

    if tag_db_id == 0 || tag_db_id > COMPACT_FRD_MAX_TARGET_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} has invalid owner tag_db_id {tag_db_id}"
        ));
    }
    if flags != COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} has non-country or unknown flags {flags:#x}"
        ));
    }
    if target_id == 0 || target_id > COMPACT_FRD_MAX_TARGET_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} has invalid country target id {target_id}"
        ));
    }
    if source_kind != COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} has invalid source_kind {source_kind}"
        ));
    }
    if decision.is_empty() {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} has no decision token"
        ));
    }
    if !seen_decision_targets.insert((tag_db_id, target_id, source_kind, decision.clone())) {
        summary.parse_errors.push(format!(
            "offset {offset}: decision target row {row_index} duplicates owner/target/source key"
        ));
    }
    country_target_rows = country_target_rows.saturating_add(1);
    let count = decision_target_rows_by_tag.entry(tag_db_id).or_insert(0);
    *count = count.saturating_add(1);
}
for (tag_db_id, target_rows) in decision_target_rows_by_tag {
    match targeted_timed_status_count_by_tag.get(&tag_db_id).copied() {
        Some(status_count) if target_rows <= status_count => {}
        Some(status_count) => summary.parse_errors.push(format!(
            "offset {offset}: decision target rows {target_rows} exceed targeted_timed_count {status_count} for tag_db_id {tag_db_id}"
        )),
        None => summary.parse_errors.push(format!(
            "offset {offset}: decision target rows exist without a decision status row for tag_db_id {tag_db_id}"
        )),
    }
}
add_compact_domain_data_unit(
    summary,
    COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
    "decision_country_target_rows",
    country_target_rows,
);
```

The snippet above assumes the decision-status decode loop keeps enough local
state for `decoded_decision_status_rows`. If the current loop does not retain
rows, add:

```rust
#[derive(Clone, Copy)]
struct CompactFrdDecodedDecisionStatus {
    tag_db_id: u32,
    targeted_timed_count: u32,
}
```

and push one row per decoded decision-status row.

## Edit 3: `extractor/native/recorder_host/src/world_fold.rs`

### Add constants

```rust
const FRD_DECISION_TARGET_ROW_FIXED_LEN: usize = 16;
const FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
const FRD_DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
const FRD_DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
const FRD_DECISION_TARGET_COUNTRY_FLAGS: u32 =
    FRD_DECISION_TARGET_FLAG_COUNTRY_TAG | FRD_DECISION_TARGET_FLAG_RECORD_0X20;
const FRD_MAX_TARGET_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
```

### Extend fold state

Add to `WorldFold`:

```rust
frd_decision_targets: BTreeMap<u32, Vec<FrdDecisionTargetState>>,
```

Initialize it in `Default`.

Add the row type near `FrdDecisionEventState`:

```rust
#[derive(Clone)]
struct FrdDecisionTargetState {
    tag_db_id: u32,
    target_tag_db_id: u32,
    flags: u32,
    source_kind: u32,
    decision: String,
}

impl FrdDecisionTargetState {
    fn to_json(&self, fold: &WorldFold, unresolved: &mut BTreeSet<u32>) -> Value {
        json!({
            "decision": self.decision,
            "target": fold.country_ref_json(self.target_tag_db_id, unresolved),
            "target_tag_db_id": self.target_tag_db_id,
            "flags": self.flags,
            "source_kind": self.source_kind,
            "source": "CDecisionStatus+0xE8 record+0x20",
        })
    }
}
```

Extend `FrdDepthFrame`:

```rust
decision_target_rows: Vec<FrdDecisionTargetState>,
```

### Replace the domain 45 header parser in `parse_frd_depth_payload`

```rust
let decision_target_count = read_u32(payload, 0) as usize;
let schema_flags_reserved_zero = read_u32(payload, 4);
let depth_status_flags = read_u32(payload, 8);
let focus_count = read_u32(payload, 12) as usize;
let research_count = read_u32(payload, 16) as usize;
let decision_status_count = read_u32(payload, 20) as usize;
let decision_event_count = read_u32(payload, 24) as usize;
let observed_focus_count = read_u32(payload, 28) as usize;
let observed_research_count = read_u32(payload, 32) as usize;
let observed_decision_status_count = read_u32(payload, 36) as usize;
let observed_decision_event_count = read_u32(payload, 40) as usize;
let badge_count = read_u32(payload, 44) as usize;
if schema_flags_reserved_zero != 0 {
    return Err("focus_research_decisions_depth schema flags are nonzero".into());
}
```

After the decision event row loop and before badges:

```rust
let mut decision_target_rows = Vec::with_capacity(decision_target_count);
for row_index in 0..decision_target_count {
    need(
        payload,
        cursor + FRD_DECISION_TARGET_ROW_FIXED_LEN,
        "frd decision target row",
    )?;
    let tag_db_id = read_u32(payload, cursor);
    let target_tag_db_id = read_u32(payload, cursor + 4);
    let flags = read_u32(payload, cursor + 8);
    let source_kind = read_u32(payload, cursor + 12);
    cursor += FRD_DECISION_TARGET_ROW_FIXED_LEN;
    let decision = read_string(payload, &mut cursor, "frd decision target token")?;
    if tag_db_id == 0 || tag_db_id > FRD_MAX_TARGET_TAG_DB_ID {
        return Err(format!(
            "frd decision target row {row_index} has invalid owner tag_db_id {tag_db_id}"
        ));
    }
    if flags != FRD_DECISION_TARGET_COUNTRY_FLAGS {
        return Err(format!(
            "frd decision target row {row_index} has non-country flags {flags:#x}"
        ));
    }
    if target_tag_db_id == 0 || target_tag_db_id > FRD_MAX_TARGET_TAG_DB_ID {
        return Err(format!(
            "frd decision target row {row_index} has invalid country target id {target_tag_db_id}"
        ));
    }
    if source_kind != FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED {
        return Err(format!(
            "frd decision target row {row_index} has invalid source_kind {source_kind}"
        ));
    }
    if decision.is_empty() {
        return Err(format!("frd decision target row {row_index} has no decision token"));
    }
    decision_target_rows.push(FrdDecisionTargetState {
        tag_db_id,
        target_tag_db_id,
        flags,
        source_kind,
        decision,
    });
}
```

Return the new vector in `FrdDepthFrame`.

### Fold target rows under the schema-safe R2 key

In `fold_focus_research_decisions_depth`, after status rows have been applied:

```rust
let mut observed_status_tags = BTreeSet::new();
for row in &frame.decision_status_rows {
    observed_status_tags.insert(row.tag_db_id);
}
for tag_db_id in observed_status_tags {
    self.frd_decision_targets.remove(&tag_db_id);
}
for row in frame.decision_target_rows {
    self.frd_decision_targets
        .entry(row.tag_db_id)
        .or_default()
        .push(row);
}
```

This preserves last-known values for countries not present in the current
domain 45 frame, but clears stale target rows for countries that were observed
and now emit zero validated country targets.

In `build_country_values`, after the existing decision status/event folding:

```rust
for (tag_db_id, targets) in &self.frd_decision_targets {
    let country = ensure_country_projection_object(countries, *tag_db_id);
    let decisions = ensure_json_object_mut(country, "decisions");
    decisions.insert(
        "_r2_depth_targets".into(),
        Value::Array(
            targets
                .iter()
                .map(|row| row.to_json(self, unresolved))
                .collect(),
        ),
    );
}
```

Use the actual helper name in the current file if it differs from
`ensure_country_projection_object`; the important rule is that owner countries
are still keyed through the fold's TAG re-keying path, and target countries are
resolved through `country_ref_json`.

## Draft tests

### Collector unit test

```rust
#[test]
fn compact_frd_depth_payload_includes_country_target_rows_only() {
    let read = CompactFrdDepthRead {
        focus_events: Vec::new(),
        research_eta: Vec::new(),
        decision_status: vec![CompactFrdDecisionStatusRow {
            tag_db_id: 10,
            active_timed_count: 0,
            targeted_count: 0,
            targeted_timed_count: 1,
            decisions_taken_bucket_count: 0,
            re_enable_count: 0,
            timed_live_count: 0,
        }],
        decision_events: Vec::new(),
        decision_targets: vec![CompactFrdDecisionTargetRow {
            tag_db_id: 10,
            target_id: 20,
            flags: COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS,
            source_kind: COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED,
            decision: Some("POR_targeted_timed_country".to_string()),
        }],
        badges: Vec::new(),
        read_time_us: 0,
        changed_rows: 1,
        status_flags: 0,
        depth_status_flags: COMPACT_FRD_STATUS_KNOWN_MASK,
        observed_focus_rows: 0,
        observed_research_slot_rows: 0,
        observed_decision_rows: 1,
        observed_timed_decision_rows: 0,
    };
    let payload = compact_frd_depth_data_payload(&read, 0);
    assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 1);
}
```

### Recorder decoder tests

```rust
#[test]
fn compact_summary_decodes_frd_country_target_rows() {
    let payload = test_frd_depth_payload_with_one_country_target_row();
    let mut summary = CompactSummary::default();
    decode_compact_frd_depth_data_payload(0, &payload, &mut summary);
    assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
    assert_eq!(
        compact_domain_data_unit(
            &summary,
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            "decision_country_target_rows"
        ),
        1
    );
}

#[test]
fn compact_summary_rejects_frd_numeric_target_flags_in_country_slice() {
    let mut payload = test_frd_depth_payload_with_one_country_target_row();
    let target_row_offset = COMPACT_FRD_DEPTH_HEADER_LEN + COMPACT_FRD_DECISION_STATUS_ROW_LEN;
    payload[target_row_offset + 8..target_row_offset + 12].copy_from_slice(&(1u32 << 2).to_le_bytes());
    let mut summary = CompactSummary::default();
    decode_compact_frd_depth_data_payload(0, &payload, &mut summary);
    assert!(summary
        .parse_errors
        .iter()
        .any(|err| err.contains("non-country or unknown flags")));
}
```

### World fold test

```rust
#[test]
fn world_fold_frd_country_targets_use_r2_depth_targets() {
    let mut fold = WorldFold::default();
    fold.country_tags_by_db_id.insert(10, "POR".to_string());
    fold.country_tags_by_db_id.insert(20, "SPR".to_string());
    let payload = test_frd_depth_payload_with_one_country_target_row();

    fold.fold_focus_research_decisions_depth(&payload).unwrap();
    let mut countries = Map::new();
    let mut unresolved = BTreeSet::new();
    fold.build_country_values(&mut countries, &mut unresolved);

    let por = countries.get("POR").unwrap().as_object().unwrap();
    let decisions = por.get("decisions").unwrap().as_object().unwrap();
    let targets = decisions
        .get("_r2_depth_targets")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(targets[0].get("target").unwrap(), "SPR");
    assert_eq!(
        targets[0].get("source").unwrap(),
        "CDecisionStatus+0xE8 record+0x20"
    );
}
```

## Validation commands

Run after applying the shared patch:

```powershell
cargo test --manifest-path extractor\native\collector\Cargo.toml frd
cargo test --manifest-path extractor\native\recorder_host\Cargo.toml frd
cargo test --manifest-path extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml
```

## Riskiest assumptions

- This is a header migration for domain 45. Collector, recorder summary, and
  world fold must land together; mixed binaries will misread the first 8 bytes.
- The shared collector may need a small enum or result type to distinguish
  structural read failure from out-of-scope non-country rows. If that distinction
  is unclear, fail closed with `PARTIAL_DATA` and no target rows.
- Schema 3.7 currently lacks first-class `targets`; this packet uses
  `_r2_depth_targets`. Promote later only after the schema is widened.
