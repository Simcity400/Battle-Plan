# Integration Packet: `army_battleplans_depth_live` domain 54

> Historical application packet. Its draft snippets and
> `SHARED_INTEGRATION_PENDING` wording are not current authority. Current
> authority is `src/lib.rs`, `A729_BATTLEPLAN_RE_EVIDENCE.json`,
> `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`, and
> `extractor/documents/dossiers/W13_RESIDUAL_COMBAT_PROJECT_RE.md`: domain 54
> uses a 60-byte header and 88-byte rows with exact `+0x244` order identity,
> exact `+0x210/+0x21C` per-order assigned-division children, and exact
> `+0x248` active/execution state. The four remaining observation gates are
> activation-false and require no new Ghidra seed.
>
> 2026-07-19 residual RE correction: see `A729_BATTLEPLAN_RE_REPORT.md` and
> `A729_BATTLEPLAN_RE_EVIDENCE.json`. Type 5 is area defense, not offensive;
> exact `+0x70/+0x78` geometry is restricted to authenticated type-2 line and
> type-3 invasion rows. Execution/activation is exact at `order+0x248`.

Packet scope: registration-ready plan for the validated domain-54 slice only.
Do not apply this packet while another worker owns shared collector/recorder
entrypoints.

## Offset Provenance Finding

Serial integration status: the validated slice is eligible for schedulable
`ValidatedSliceOpenGaps` registration. This is not an older-build carryover:
the lane archive and proof gate identify the validated executable as
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad` and list
the dereferenced field policy validated for that build: `CArmy` vtable RVA
`0x293BC60`, order vtable RVA `0x296FAE8`, `CArmy+0x98`,
`order+0x30`, `order+0x38`, type-5 area-defense `order+0xE0/+0xE8`, and
type-2/type-3 `order+0x70/+0x78`.

Runtime collector identity is not a full executable-SHA read. Shared production
matching uses `compact_contract::build_profile` sentinel checks for the profile
label, daily-update bytes, and long-string bytes. If those sentinel checks
produce `PROFILE_MISMATCH`, domain 54 must emit a rowless `PARTIAL_DATA` frame
with the mismatch status and must not force or dereference its offset-dependent
dependency paths.

The decisive proof family is
`extractor/native/runs/army_battleplans_depth_live_save_geometry_870ebf_20260701_v1`:
the archive records `137/171` save/live paired type-5 rows matching save
`states` at `order+0xE0/+0xE8`, `95/111` save/live paired type-2/type-3 rows
matching save `path` at `order+0x70/+0x78`, and rejection of the old type-3
`order+0xC0/+0xC8` primary path candidate (`0/17` matches). The direct-root
candidate families remain rejected or partial and are not part of this hot-loop
integration.

## Validated Slice Target

Integrate only these fields:

- active army orders already reachable through the validated division-root to
  `CArmy+0xB0` to `CArmy+0x98` order-vector path;
- `order+0x30` order type and `order+0x38` back-reference validation;
- exact type-5 area-defense target-state vectors at `order+0xE0/+0xE8`;
- exact type-2 line/type-3 invasion path geometry vectors at
  `order+0x70/+0x78`, emitted as an
  8-byte province child table only when the live vector validates;
- exact same-frame assigned-unit and order-linkage status, derived from the
  paired `oob_command` army row for the same frame;
- exact active/execution state from `order+0x248`, plus explicit
  `STATUS_UNKNOWN` badges for stance/aggressiveness.

Keep out of this integration:

- direct empty/inactive armies, army groups, and theatres. Existing direct-root
  proof rows are cold-preflight/discovery evidence, not a DLL hot-loop
  production source;
- pure preflight address cache, targeted arena-wide handle refresh, and plain
  pointer-table direct-root candidates. Existing proof rejects these as
  scheduled hot-loop paths;
- an authenticated baseline -> same-instance type-2 redraw -> delete lifecycle,
  type-4 geometry, plan-value caches, stance/aggressiveness values, non-path
  geometry semantics, and UI semantic reconstruction of fronts.

Lane-local code in this packet:

- `src/lib.rs` contains the compact domain-54 encoder/decoder plus isolated
  live-memory helper traits for the validated path-vector slice.
- The integrated fold/parser lives in
  `extractor/native/recorder_host/src/world_fold.rs`; the prior draft file has
  been applied and removed.

## Shared Registration Prerequisites

These shared edits are intentionally not applied by this packet.

`extractor/native/Cargo.toml`:

```toml
members = [
    # ...
    "lanes/army_battleplans_depth_live",
]
```

`extractor/native/collector/Cargo.toml` and
`extractor/native/recorder_host/Cargo.toml`:

```toml
army_battleplans_depth_live_lane = { path = "../lanes/army_battleplans_depth_live" }
```

Use package name `army_battleplans_depth_live_lane`.

Before adding the member, remove the empty `[workspace]` stanza from this
lane-local `Cargo.toml`. It exists only so the lane can be tested before shared
workspace registration.

## Collector Wiring

Target file: `extractor/native/collector/src/lib.rs`.

### Import and constants

Add beside the existing domain-52 and domain-55 imports:

```rust
use army_battleplans_depth_live_lane as battleplans54;
```

Add beside the other raw-domain names and compact domain ids:

```rust
const RAW_ARMY_BATTLEPLANS_DEPTH_LIVE_DOMAIN: &str = "army_battleplans_depth_live";
const COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE: u16 = 54;
```

### Tracker and read structs

Add to the `CompactTrackers` struct near `occupation_resistance_compliance_live`
and `land_combat_detail_events_live`:

```rust
army_battleplans_depth_live: battleplans54::BattleplansDepthTracker,
```

Add near `CompactArmyOrderRead` / `CompactOobCommandRead`:

```rust
struct CompactArmyBattleplansDepthRead {
    payload: Vec<u8>,
    read_time_us: u64,
    status_flags: u32,
    emitted_rows: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
    target_state_rows: u32,
    path_geometry_rows: u32,
    non_type5_exact_path_rows: u32,
    fixture_blocked_rows: u32,
}
```

### Extend private army-order samples, not the domain-27 wire format

The current domain-27 payload must stay unchanged. Add private fields to the
collector's internal `ArmyOrderSample` and `CompactArmyOrderRow` only:

```rust
path_provinces: Option<Vec<u32>>,
geometry_status: &'static str,
```

In `read_army_order_sample`, after `read_army_order_target_states(...)`:

```rust
let (path_provinces, geometry_status) =
    read_army_battleplans_depth_path_provinces(order_ptr, order_type);
```

Populate the new fields in `ArmyOrderSample`, `compact_army_order_row`, and
`deleted_compact_army_order_row`. Deleted rows should use
`path_provinces: None` and `geometry_status: "none"`.

Do not serialize these fields in `compact_army_orders_data_payload`; they are
only the same-frame source for domain 54.

### Live-memory validation helper

Add near `read_army_order_target_states`. This mirrors the lane helper but uses
the collector's `read_u64`, `read_u32`, `read_bytes`, and
`is_map_control_heap_pointer` functions:

```rust
fn read_army_battleplans_depth_path_provinces(
    order_ptr: usize,
    order_type: i32,
) -> (Option<Vec<u32>>, &'static str) {
    if order_type == ORDER_TYPE_AREA_DEFENSE {
        return (None, "exact");
    }
    if !matches!(order_type, ORDER_TYPE_LINE | ORDER_TYPE_INVASION) {
        return (None, "unsupported");
    }
    let Some(begin_addr) = order_ptr.checked_add(battleplans54::ORDER_OFF_PATH_BEGIN as usize) else {
        return (None, "fixture_required");
    };
    let Some(size_addr) = order_ptr.checked_add(battleplans54::ORDER_OFF_PATH_SIZE as usize) else {
        return (None, "fixture_required");
    };
    let Some(begin) = read_u64(begin_addr).map(|value| value as usize) else {
        return (None, "fixture_required");
    };
    let Some(size) = read_u32(size_addr) else {
        return (None, "fixture_required");
    };
    if size == 0 {
        return (None, "fixture_required");
    }
    if size > 5_000 {
        return (None, "validation_failed");
    }
    if size > 256 {
        return (None, "overflow");
    }
    if !is_map_control_heap_pointer(begin) {
        return (None, "fixture_required");
    }
    let Some(byte_len) = (size as usize).checked_mul(4) else {
        return (None, "validation_failed");
    };
    let Some(raw) = read_bytes(begin, byte_len) else {
        return (None, "fixture_required");
    };
    if raw.len() < byte_len {
        return (None, "fixture_required");
    }
    let mut provinces = Vec::with_capacity(size as usize);
    for i in 0..size as usize {
        let offset = i * 4;
        let province_id = u32::from_le_bytes([
            raw[offset],
            raw[offset + 1],
            raw[offset + 2],
            raw[offset + 3],
        ]);
        if province_id == 0 || province_id > 65_535 {
            return (None, "validation_failed");
        }
        provinces.push(province_id);
    }
    (Some(provinces), "exact")
}
```

### Concrete memory-trait impl

If the serial worker chooses to call lane helpers directly rather than keeping
the helper above inline, use this in-process implementation:

```rust
struct Domain54InProcessMemory;

impl battleplans54::BattleplansDepthMemory for Domain54InProcessMemory {
    fn module_base(&self) -> u64 {
        unsafe { GetModuleHandleW(ptr::null()) as u64 }
    }

    fn read_u64(&self, address: u64) -> Option<u64> {
        read_u64(usize::try_from(address).ok()?)
    }

    fn read_u32(&self, address: u64) -> Option<u32> {
        read_u32(usize::try_from(address).ok()?)
    }

    fn read_i32(&self, address: u64) -> Option<i32> {
        read_i32(usize::try_from(address).ok()?)
    }

    fn read_bytes(&self, address: u64, len: usize) -> Option<Vec<u8>> {
        read_bytes(usize::try_from(address).ok()?, len)
    }

    fn is_heap_pointer(&self, address: u64) -> bool {
        usize::try_from(address)
            .ok()
            .is_some_and(is_map_control_heap_pointer)
    }
}
```

### Enable flags and forced dependencies

Replace the current `army_orders_enabled` / `oob_command_enabled` definitions
with dependency-aware definitions:

```rust
let army_battleplans_depth_live_enabled =
    compact_explicit_domain_enabled(control, RAW_ARMY_BATTLEPLANS_DEPTH_LIVE_DOMAIN);
let army_orders_explicit_enabled =
    compact_explicit_domain_enabled(control, RAW_ARMY_ORDERS_DOMAIN);
let oob_command_explicit_enabled =
    compact_explicit_domain_enabled(control, RAW_OOB_COMMAND_DOMAIN);
let army_orders_enabled = army_orders_explicit_enabled || army_battleplans_depth_live_enabled;
let oob_command_enabled = oob_command_explicit_enabled || army_battleplans_depth_live_enabled;
```

Update `compact_division_roots_enabled`:

```rust
|| compact_explicit_domain_enabled(control, RAW_ARMY_BATTLEPLANS_DEPTH_LIVE_DOMAIN)
```

Add the read slot beside the other `*_read` locals:

```rust
let mut army_battleplans_depth_live_read = None;
```

Add the tracker ref beside `army_order_tracker` and `oob_command_tracker`:

```rust
let army_battleplans_depth_live_tracker = &mut trackers.army_battleplans_depth_live;
```

### Worker spawn/join site

Do not spawn domain 54 in the first scoped worker group. Domain 54 depends on
the already-joined same-frame domain-27 and domain-28 reads. The worker edit
site is therefore the existing domain-27/domain-28 workers: requesting domain
54 forces those workers on. After their join block and after the domain-55
derived read, add this panic-wrapped derived read:

```rust
if army_battleplans_depth_live_enabled {
    army_battleplans_depth_live_read = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read_compact_army_battleplans_depth_live(
                army_order_read.as_ref(),
                oob_command_read.as_ref(),
                status_flags,
                army_battleplans_depth_live_tracker,
            )
        }))
        .unwrap_or_else(|_| compact_army_battleplans_depth_worker_panic_read(status_flags)),
    );
}
```

This follows the domain-55 post-dependency pattern, with a domain-52-style panic
wrapper added because the packet is registration-only and should fail closed.

### Read and panic helpers

Add near the domain-52 and domain-55 read helpers:

```rust
fn read_compact_army_battleplans_depth_live(
    army_order_read: Option<&CompactArmyOrderRead>,
    oob_command_read: Option<&CompactOobCommandRead>,
    status_flags: u32,
    _tracker: &mut battleplans54::BattleplansDepthTracker,
) -> CompactArmyBattleplansDepthRead {
    let read_started = Instant::now();
    let mut final_status_flags = status_flags;
    let Some(army_order_read) = army_order_read else {
        return compact_army_battleplans_depth_unavailable_read(read_started, final_status_flags);
    };
    let Some(oob_command_read) = oob_command_read else {
        return compact_army_battleplans_depth_unavailable_read(read_started, final_status_flags);
    };
    final_status_flags |= army_order_read.status_flags & COMPACT_STATUS_DELTA_DATA;
    if compact_contract::compact_status_is_failure(army_order_read.status_flags)
        || compact_contract::compact_status_is_failure(oob_command_read.status_flags)
    {
        final_status_flags |= (army_order_read.status_flags | oob_command_read.status_flags)
            & compact_contract::COMPACT_STATUS_FAILURE_MASK;
        final_status_flags |= COMPACT_STATUS_PARTIAL_DATA;
        return compact_army_battleplans_depth_empty_read(read_started, final_status_flags);
    }

    let exact_oob_armies = oob_command_read
        .rows
        .iter()
        .filter(|row| {
            row.kind == COMPACT_OOB_KIND_ARMY
                && !row.deleted
                && row.member_status == "exact"
        })
        .map(|row| row.object_id)
        .collect::<BTreeSet<_>>();

    let observations = army_order_read
        .rows
        .iter()
        .map(|row| {
            let linked = exact_oob_armies.contains(&row.army_id);
            battleplans54::RawOrderObservation {
                army_id: row.army_id,
                generation: row.generation,
                engine_order_instance_id: row.engine_order_instance_id,
                deleted: row.deleted,
                slot_index: row.slot_index,
                order_type: row.order_type,
                target_states: row.target_states.clone().unwrap_or_default(),
                path_provinces: row.path_provinces.clone().unwrap_or_default(),
                target_declared_count: row.target_state_count,
                target_status: battleplans54::status_code_from_name(row.target_status),
                geometry_status: battleplans54::status_code_from_name(row.geometry_status),
                assigned_division_ids: row.assigned_division_ids.clone().unwrap_or_default(),
                assigned_units_status: if linked {
                    battleplans54::STATUS_EXACT
                } else {
                    battleplans54::STATUS_FIXTURE_REQUIRED
                },
                order_linkage_status: if linked {
                    battleplans54::STATUS_EXACT
                } else {
                    battleplans54::STATUS_FIXTURE_REQUIRED
                },
                active: row.active.unwrap_or(false),
                active_status: battleplans54::status_code_from_name(row.active_status),
                stance_status: battleplans54::STATUS_UNKNOWN,
                aggressiveness_status: battleplans54::STATUS_UNKNOWN,
            }
        })
        .collect::<Vec<_>>();

    let encoded = battleplans54::collect_and_encode_depth_record(
        &observations,
        duration_millis_u64(read_started.elapsed()),
        final_status_flags,
        army_order_read.observed_army_rows,
        army_order_read.observed_order_rows,
    );
    let read_time_us = duration_micros_u64(read_started.elapsed());
    CompactArmyBattleplansDepthRead {
        payload: encoded.payload,
        read_time_us,
        status_flags: final_status_flags,
        emitted_rows: encoded.emitted_rows,
        observed_army_rows: army_order_read.observed_army_rows,
        observed_order_rows: army_order_read.observed_order_rows,
        target_state_rows: encoded.extraction.type5_exact_rows,
        path_geometry_rows: encoded.extraction.path_geometry_rows,
        non_type5_exact_path_rows: encoded.extraction.non_type5_exact_path_rows,
        fixture_blocked_rows: encoded.extraction.non_type5_fixture_blocked_rows,
    }
}

fn compact_army_battleplans_depth_unavailable_read(
    read_started: Instant,
    status_flags: u32,
) -> CompactArmyBattleplansDepthRead {
    compact_army_battleplans_depth_empty_read(
        read_started,
        status_flags | COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
    )
}

fn compact_army_battleplans_depth_worker_panic_read(
    status_flags: u32,
) -> CompactArmyBattleplansDepthRead {
    let read_started = Instant::now();
    compact_army_battleplans_depth_empty_read(
        read_started,
        status_flags | COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA,
    )
}

fn compact_army_battleplans_depth_empty_read(
    read_started: Instant,
    status_flags: u32,
) -> CompactArmyBattleplansDepthRead {
    let encoded = battleplans54::collect_and_encode_depth_record(
        &[],
        duration_millis_u64(read_started.elapsed()),
        status_flags,
        0,
        0,
    );
    CompactArmyBattleplansDepthRead {
        payload: encoded.payload,
        read_time_us: duration_micros_u64(read_started.elapsed()),
        status_flags,
        emitted_rows: 0,
        observed_army_rows: 0,
        observed_order_rows: 0,
        target_state_rows: 0,
        path_geometry_rows: 0,
        non_type5_exact_path_rows: 0,
        fixture_blocked_rows: 0,
    }
}
```

### Append function

Add beside the domain-52 and domain-55 append helpers:

```rust
fn append_compact_army_battleplans_depth_data_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    read: &CompactArmyBattleplansDepthRead,
) {
    append_compact_record(
        out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &read.payload,
    );
}
```

### Perf and data emission

Emit after domain 27 and 28 data records, before frame end:

```rust
if let Some(read) = army_battleplans_depth_live_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
            read_time_us: read.read_time_us,
            row_count: read.observed_army_rows,
            changed_rows: read.emitted_rows,
            unit_count: read.observed_order_rows,
            changed_unit_count: read
                .target_state_rows
                .saturating_add(read.path_geometry_rows),
            status_flags: read.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_compact_army_battleplans_depth_data_record(
        &mut out,
        sequence,
        game_hour_raw,
        &read,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

## Recorder Decode Wiring

Target file: `extractor/native/recorder_host/src/main.rs`.

Add import:

```rust
use army_battleplans_depth_live_lane as battleplans54;
```

Add constant:

```rust
const COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE: u16 = 54;
```

Add decode arm beside domains 52 and 55:

```rust
COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE => {
    decode_compact_army_battleplans_depth_live_data_payload(
        offset,
        payload,
        &mut summary,
    )
}
```

Add decoder function:

```rust
fn decode_compact_army_battleplans_depth_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let lane_summary = battleplans54::decode_depth_payload(offset, payload);
    summary.status_flags_or |= lane_summary.totals.status_flags_or;
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE)
    {
        domain.status_flags_or |= lane_summary.totals.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.totals.rows);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.totals.changed_rows);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_depth_rows",
        lane_summary.totals.rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_target_state_rows",
        lane_summary.totals.target_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_path_geometry_rows",
        lane_summary.totals.path_geometry_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_non_type5_exact_path_rows",
        lane_summary.totals.non_type5_exact_path_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_fixture_blocked_rows",
        lane_summary.totals.unsupported_geometry_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_assigned_units_exact_rows",
        lane_summary.totals.assigned_units_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_order_linkage_exact_rows",
        lane_summary.totals.order_linkage_exact_rows as usize,
    );
}
```

Update forced dependency manifest reporting:

```rust
let requested_54 = scheduled_domains
    .iter()
    .any(|domain| compact_domain_id(domain) == Some(COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE));
let requested_27 = scheduled_domains.iter().any(|domain| compact_domain_id(domain) == Some(27));
let requested_28 = scheduled_domains.iter().any(|domain| compact_domain_id(domain) == Some(28));
```

If `requested_54`, append dependencies for domains 27 and 28 when absent:

```rust
{
    "requested_domain": "army_battleplans_depth_live",
    "forced_domain": "army_orders",
    "forced_domain_id": 27,
    "reason": "domain 54 derives generation/order rows from the same-frame army_orders read",
}
{
    "requested_domain": "army_battleplans_depth_live",
    "forced_domain": "oob_command",
    "forced_domain_id": 28,
    "reason": "domain 54 requires exact same-frame assignment and order-linkage status from oob_command",
}
```

Also update any scheduled-name/coverage tests to include domain 54 once the
descriptor below becomes schedulable.

## World Fold Wiring

Target file: `extractor/native/recorder_host/src/world_fold.rs`.

Current file folds domain 55 but not domain 52. Domain 54 must be added to the
fold at the same time as recorder decode registration, otherwise the compact
manifest will list a production-profile domain as unfolded.

Add import:

```rust
use army_battleplans_depth_live_lane as battleplans54;
```

Update folded ids:

```rust
pub const FOLDED_DOMAIN_IDS: &[u16] =
    &[10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 54, 55];
```

Add fields to `WorldFold`:

```rust
army_battleplans_depth: BTreeMap<ArmyBattleplanDepthKey, ArmyBattleplanDepthOrder>,
army_battleplans_depth_baseline_seen: bool,
army_battleplans_depth_unknown_this_hour: bool,
```

Initialize/reset:

```rust
army_battleplans_depth: BTreeMap::new(),
army_battleplans_depth_baseline_seen: false,
army_battleplans_depth_unknown_this_hour: false,
```

```rust
self.army_battleplans_depth_unknown_this_hour = false;
```

Add apply arm:

```rust
54 => self.fold_army_battleplans_depth(payload),
```

Use the integrated `world_fold.rs` parser and accumulator behavior:

```rust
fn fold_army_battleplans_depth(&mut self, payload: &[u8]) -> Result<(), String> {
    let decoded = battleplans54::decode_depth_payload(0, payload);
    if !decoded.parse_errors.is_empty() {
        return Err(decoded.parse_errors.join("; "));
    }
    let status_flags = decoded.totals.status_flags_or;
    if compact_status_is_failure(status_flags) {
        self.army_battleplans_depth_unknown_this_hour = true;
        self.note(format!(
            "army_battleplans_depth_live failure status {:#x}; battleplan depth state unknown this hour",
            status_flags & COMPACT_STATUS_FAILURE_MASK
        ));
        return Ok(());
    }
    let is_delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    if is_delta && !self.army_battleplans_depth_baseline_seen {
        self.army_battleplans_depth_unknown_this_hour = true;
        self.record_domain_problem(54, "delta_before_baseline");
        self.note(
            "army_battleplans_depth_live delta arrived before a full baseline; rows ignored this hour"
                .to_string(),
        );
        return Ok(());
    }
    if !is_delta {
        self.army_battleplans_depth.clear();
        self.army_battleplans_depth_baseline_seen = true;
    }

    // Decode target/path child rows as values. Apply deleted rows as removals
    // and non-deleted rows as upserts keyed by (army_id, generation,
    // slot_index). Preserve omitted-row counters in the extra provenance fields.
    Ok(())
}
```

The current `record_domain_problem` helper is private and takes `domain_id/problem`.
If that helper is renamed by the concurrent provenance worker, wire this to the
new equivalent rather than silently logging only a note.

Add to snapshot construction. Current snapshots emit `"order_of_battle": {}`.
Replace that with:

```rust
let mut order_of_battle = Map::new();
let mut order_of_battle_extra = Map::new();
order_of_battle_extra.insert(
    "army_battleplans_depth_live".into(),
    self.army_battleplans_depth_extra_json(),
);
order_of_battle.insert("extra".into(), Value::Object(order_of_battle_extra));
```

Then in the final snapshot:

```rust
"order_of_battle": Value::Object(order_of_battle),
```

The extra JSON should include:

```json
{
  "known": true,
  "baseline_seen": true,
  "orders": [
    {
      "army_id": 10,
      "generation": 1,
      "slot_index": 0,
      "order_type": 5,
      "target_states": [44, 45],
      "path_provinces": [],
      "target_declared_count": 2,
      "target_status": "exact",
      "geometry_status": "exact",
      "active": true,
      "active_status": "exact",
      "execution_status": "exact",
      "assigned_units_status": "exact",
      "order_linkage_status": "exact",
      "stance_status": "unknown",
      "aggressiveness_status": "unknown"
    }
  ]
}
```

Failure handling:

- If `compact_status_is_failure(status_flags)` is true, do not apply rows.
  Retain the accumulator, set `"known": false` for that hour, and add a
  provenance note.
- If a delta arrives before a full baseline, do not apply rows. Add
  `domain_problems` entry `delta_before_baseline`, set `"known": false`, and
  keep the accumulator unchanged.
- If the first-hour baseline guard worker lands before this packet is applied,
  re-check their conventions and use their shared helper instead of duplicating
  the guard.

## Compact Contract Descriptor

Target file: `extractor/native/compact_contract/src/lib.rs`.

Current descriptor:

```rust
domain(
    54,
    "army_battleplans_depth_live",
    "army_battleplans_depth_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

After the serial worker applies collector, recorder, and fold registration,
change it to:

```rust
domain(
    54,
    "army_battleplans_depth_live",
    "army_battleplans_depth_live",
    CompactDomainStatus::ValidatedSliceOpenGaps,
    true,
    true,
    &[],
),
```

Do not use `ProductionComplete` or `ProductionCompleteBase`. The validated slice
still has open replay-relevant gaps.

Update the production-profile descriptor test expectation if it hardcodes the
included ids. Current profile membership is derived from schedulability and
status, so domain 54 should enter the profile only after this descriptor change.

## R2 Schema Delta Text

Target file after serial integration:
`replay_interface/documents/REPLAY_SCHEMA.md`.

Add under section 12:

```markdown
### 12.x `order_of_battle.extra.army_battleplans_depth_live` (live compact domain 54)

Until the native fold has a full save-shaped OOB accumulator for `armies[]`,
domain 54 folds into this clearly named extra bucket rather than mutating the
documented `armies[].orders[]` shape.

The bucket is a live delta accumulator keyed by
`(army_id, generation, slot_index)`. A non-delta domain-54 record is the
baseline and replaces the accumulator; records with `COMPACT_STATUS_DELTA_DATA`
upsert/delete rows after the baseline. If a delta arrives before a baseline or
the compact status carries a failure bit, the fold keeps prior state, marks the
bucket `known=false` for that hour, and records the reason in provenance.

Rows carry only the validated slice: exact type-5 area-defense `target_states`,
exact type-2 line/type-3 invasion `path_provinces` from live
`order+0x70/+0x78`, exact active/execution state from `order+0x248`, exact
assigned-unit/order-linkage statuses, and explicit `unknown` badges for
unvalidated stance and aggressiveness fields.
```

## Contract and Registry Wording After Integration

`extractor/documents/MASTER_EXTRACTION_CONTRACT.md` row:

```markdown
| `army_battleplans_depth_live` / 54 | `SHARED_INTEGRATION_PENDING` | Shared collector/recorder/fold wiring exists for the validated partial slice: exact type-5 area-defense target-state rows, exact save/live-paired type-2/type-3 `order+0x70/+0x78` path-geometry rows, exact `order+0x248` active/execution state, and same-frame assignment/linkage status. Shared speed-five `.battleplan` proof and live redraw/delete fixture closure remain. |
```

Battleplan geometry capability row:

```markdown
| Battleplan geometry | Line/invasion/area-defense geometry, redraws, and activation/execution state. | `SHARED_INTEGRATION_PENDING` | `army_battleplans_depth_live` / 54; shared wiring covers exact type-5 area-defense target states, exact type-2/type-3 `order+0x70/+0x78` path rows from the validated save/live paired slice, and exact `order+0x248` active/execution state. | Pass shared DLL/Rust/.battleplan proof; capture the minimal same-instance type-2 redraw/delete and type-4 discriminator; close or exactly reject stance/aggressiveness and non-path geometry semantics. |
```

`extractor/documents/DOMAIN_REGISTRY.md` row:

```markdown
| 54 | `army_battleplans_depth_live` | shared integration pending: validated type-5 area-defense target states, type-2/type-3 `order+0x70/+0x78` path geometry, and `order+0x248` active/execution state wired; redraw/delete fixture and direct-root manager coverage open | army OOB/orders/battleplans depth |
```

Do not call the whole lane complete until the open fields below are validated,
rejected with proof, or fixture-blocked with exact evidence.

## Open and Fixture-Blocked Items That Stay Out

- Direct roots for empty/inactive armies, army groups, and theatres:
  stay out until a complete current-build manager/container root or bounded
  DLL-native refresh contract exists. Cold-preflight rows are not production.
- Redraw/delete lifecycle and type-4/non-path geometry semantics:
  require the minimal controlled fixture in
  `X1_X8_REMAINING_HUMAN_FIXTURE_CARD.json` or exact rejection.
- Activation/execution state:
  exact at `order+0x248`; `order+0x150` remains a rejected candidate and must
  not be retried or emitted.
- Stance/aggressiveness:
  `CArmy+0x1A4/+0x1A8/+0x1AC` are near-match scalar candidates only. Width and
  exactness are unresolved. Emit `unknown` only.
- Plan-value/power runtime caches:
  cached/non-authoritative and not part of this validated compact slice.
- Full save-shaped `order_of_battle.armies[].orders[]` reconstruction:
  out until domains 27/28/54 share a complete OOB fold.

## Post-Application Proof Gate

After the serial worker applies shared registration, run a live speed-five or
current-equivalent capture with domain 54 scheduled. Required pass criteria:

- domain 54 appears as `army_battleplans_depth_live`;
- domains 27 and 28 are forced when 54 is scheduled;
- one domain-54 data record per frame hour;
- compact stream ordered and decodable;
- no skipped hours;
- no `source_unavailable`, `partial_data`, `bridge_unavailable`, or
  `worker_panic` failure bits;
- `.battleplan` verifies;
- decoder units show exact type-5 area-defense rows and exact type-2/type-3
  path-geometry rows when present;
- all non-deleted rows have exact assignment and order-linkage status;
- active/execution status is exact; stance/aggressiveness remain `unknown`.
