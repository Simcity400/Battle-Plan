# Integration Packet: naval_missions_status_live (compact domain 56)

Packet owner: lane-local only. Shared files are read-only for this packet.

Validated slice targeted:

- Validated live roots: `CGameState`, `CGameState+0x310/+0x31C` country table, `CCountry+0x278/+0x280/+0x284` fleet vector, `CFleet`, `CTaskForce`, `CTaskForce+0x18` current province, `CTaskForce+0x364` mission enum, `CTaskForce+0x4A8` detached-activity enum, `CFleet+0xD8` home-base province pointer, `CTaskForce+0x4B0` repair-target province pointer, `CTaskForce+0x338/+0x344` ship vector, and `CShip+0x3C` aggregate sunk-convoy counter.
- Integrated rows: full-replace task-force status rows keyed by `task_force_id`, carrying `fleet_id`, `owner_tag_db_id`, current `location_province_id`, optional derived `strategic_region_id`, exact mission enum/name, exact detached-activity `mission_state`, nullable home/repair-base province ids, and optional aggregate `sunk_convoys`.
- Explicitly out of scope: repair progress, task-force retreat-state enum, and
  convoy route/context semantics. Those remain absent
  rather than guessed. The save oracle separately exposes exact persisted
  mission/repair/path fields and the boolean `task_force.retreat=yes`.

Important lane-local changes made in this packet:

- `Cargo.toml` and `src/lib.rs` define package `naval_missions_status_live_lane` with a production-shaped memory trait, root resolver, row extractor, and full-replace tracker.
- `domain56_compact.rs` now exposes `decode_status_payload()` returning rows for `world_fold` and moved fixture-required status bits to `1 << 16..19` so they do not overlap shared `compact_contract` failure bits.
- `fold_draft.rs` is a standalone fold draft with tests.

## 1. Workspace And Dependency Registrations

These are shared-file edits for the serial integration worker.

`extractor/native/Cargo.toml`:

```toml
members = [
    # existing members...
    "lanes/naval_missions_status_live",
]
```

`extractor/native/collector/Cargo.toml`:

```toml
naval_missions_status_live_lane = { path = "../lanes/naval_missions_status_live" }
```

`extractor/native/recorder_host/Cargo.toml`:

```toml
naval_missions_status_live_lane = { path = "../lanes/naval_missions_status_live" }
```

After these registrations, the intended lane test command is:

```powershell
cd extractor/native
cargo test -p naval_missions_status_live_lane
```

Current hard-scope note: this packet could not add the workspace member, so `cargo test -p naval_missions_status_live_lane` currently fails with "did not match any packages". Standalone `rustc --test` lane checks pass.

## 2. Collector Wiring

All snippets target `extractor/native/collector/src/lib.rs` unless noted.

### 2.1 Import

Add beside the 52/55 lane imports in `mod attach_log`:

```rust
use naval_missions_status_live_lane as naval56;
```

### 2.2 Constants

Add beside the compact live domain names:

```rust
const RAW_NAVAL_MISSIONS_STATUS_LIVE_DOMAIN: &str = "naval_missions_status_live";
```

Add beside compact domain ids:

```rust
const COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE: u16 = 56;
```

Do not add this to `valid_raw_domain()` unless a raw JSON delta stream is introduced. This packet only emits compact H4CR domain data.

### 2.3 Tracker Field

Add to `CompactGateTrackers` beside 52/55:

```rust
naval_missions_status_live: naval56::CompactNavalMissionStatusTracker,
```

### 2.4 Read Struct

Add beside `CompactOccupationResistanceComplianceRead` and `CompactLandCombatDetailEventsRead`:

```rust
struct CompactNavalMissionStatusRead {
    frame: naval56::EncodedFrame,
    read_time_us: u64,
    status_flags: u32,
    observed_task_forces: u32,
}
```

### 2.5 Enable Flag And Tracker Borrow

In `build_compact_frame_payload`, add beside 52/55 enable flags:

```rust
let naval_missions_status_live_enabled =
    compact_explicit_domain_enabled(control, RAW_NAVAL_MISSIONS_STATUS_LIVE_DOMAIN);
```

Add beside the read slots:

```rust
let mut naval_missions_status_live_read = None;
```

Add beside the tracker borrows:

```rust
let naval_missions_status_live_tracker = &mut trackers.naval_missions_status_live;
```

### 2.6 Worker Spawn

Add in the `thread::scope` spawn section near the 52 worker:

```rust
let naval_missions_status_live_handle = if naval_missions_status_live_enabled {
    Some(scope.spawn(move || {
        read_compact_naval_missions_status_live(
            state,
            status_flags,
            naval_missions_status_live_tracker,
        )
    }))
} else {
    None
};
```

### 2.7 Worker Join And Panic Wrapper

Add in the join section near the 52 join:

```rust
if let Some(handle) = naval_missions_status_live_handle {
    naval_missions_status_live_read = Some(match handle.join() {
        Ok(read) => read,
        Err(_) => compact_naval_missions_status_worker_panic_read(status_flags),
    });
}
```

### 2.8 Concrete Live Memory Trait Impl

Add beside `Domain52InProcessMemory`:

```rust
struct Domain56InProcessMemory;

impl naval56::NavalMissionMemory for Domain56InProcessMemory {
    fn module_base(&self) -> u64 {
        unsafe { GetModuleHandleW(ptr::null()) as u64 }
    }

    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, naval56::ExtractError> {
        let addr = usize::try_from(address)
            .map_err(|_| naval56::ExtractError::MemoryRead { address, len })?;
        read_bytes(addr, len).ok_or(naval56::ExtractError::MemoryRead { address, len })
    }

    fn province_to_region(&self, _province_id: u32) -> Option<u32> {
        // Fail closed until the shared collector owns an in-DLL province->region map.
        // The validated slice still emits current province without derived region.
        None
    }
}
```

If the serial worker has a current collector-side province-to-strategic-region table by then, replace the `None` body with that lookup. Do not read JSON from the injected DLL hot path.

### 2.9 Read Function

Add beside the 52 read function:

```rust
fn read_compact_naval_missions_status_live(
    state: &CollectorState,
    status_flags: u32,
    tracker: &mut naval56::CompactNavalMissionStatusTracker,
) -> CompactNavalMissionStatusRead {
    let read_started = Instant::now();
    let mem = Domain56InProcessMemory;
    let mut final_status_flags = status_flags;
    let (frame, observed_task_forces) =
        match naval56::resolve_naval_roots(&mem).and_then(|roots| {
            let observed_task_forces = roots.task_forces.len().min(u32::MAX as usize) as u32;
            naval56::extract_rows_from_roots(&mem, &roots).map(|extracted| {
                let encoded = tracker.encode_frame(
                    duration_millis_u64(read_started.elapsed()),
                    status_flags,
                    extracted.observed_task_forces,
                    &extracted.rows,
                );
                (
                    encoded,
                    observed_task_forces.max(extracted.observed_task_forces),
                )
            })
        }) {
            Ok(result) => result,
            Err(_) => {
                final_status_flags |=
                    COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
                let encoded = tracker.encode_frame(
                    duration_millis_u64(read_started.elapsed()),
                    final_status_flags,
                    0,
                    &[],
                );
                (encoded, 0)
            }
        };
    let read_time_us = duration_micros_u64(read_started.elapsed());
    let status_flags = final_status_flags | frame.header.status_flags;
    let _ = state;
    CompactNavalMissionStatusRead {
        frame,
        read_time_us,
        status_flags,
        observed_task_forces,
    }
}
```

### 2.10 Panic Wrapper

Add beside the 52 panic wrapper:

```rust
fn compact_naval_missions_status_worker_panic_read(
    status_flags: u32,
) -> CompactNavalMissionStatusRead {
    let mut tracker = naval56::CompactNavalMissionStatusTracker::default();
    let status_flags = status_flags | COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
    let frame = tracker.encode_frame(0, status_flags, 0, &[]);
    CompactNavalMissionStatusRead {
        frame,
        read_time_us: 0,
        status_flags,
        observed_task_forces: 0,
    }
}
```

### 2.11 Append Function

Add beside `append_compact_occupation_resistance_compliance_data_record`:

```rust
fn append_compact_naval_missions_status_live_data_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    frame: &naval56::EncodedFrame,
) {
    append_compact_record(
        out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &frame.payload,
    );
}
```

### 2.12 Perf And Data Emission

Add in the compact payload emission section near 52:

```rust
if let Some(read) = naval_missions_status_live_read {
    let emitted_rows = read.frame.rows.len().min(u32::MAX as usize) as u32;
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE,
            read_time_us: read.read_time_us,
            row_count: read.observed_task_forces,
            changed_rows: read.frame.header.changed_rows,
            unit_count: emitted_rows,
            changed_unit_count: read.frame.header.changed_rows,
            status_flags: read.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_compact_naval_missions_status_live_data_record(
        &mut out,
        sequence,
        game_hour_raw,
        &read.frame,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

## 3. Recorder Decode Arm

All snippets target `extractor/native/recorder_host/src/main.rs`.

### 3.1 Import And Constant

Add beside the 52/55 imports:

```rust
use naval_missions_status_live_lane as naval56;
```

Add beside compact domain ids:

```rust
const COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE: u16 = 56;
```

### 3.2 Decode Match Arm

Add beside 52/55 in the `COMPACT_RECORD_TYPE_DOMAIN_DATA` match:

```rust
COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE => {
    decode_compact_naval_missions_status_live_data_payload(offset, payload, &mut summary)
}
```

### 3.3 Decode Function

Add near the 52/55 decode helpers:

```rust
fn decode_compact_naval_missions_status_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    let lane_summary = naval56::decode_payload(offset, payload);
    summary.status_flags_or |= lane_summary.status_flags_or;
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            let entry = domain.data_units_total.entry(name.to_string()).or_default();
            *entry = entry.saturating_add(count);
        }
    }
}
```

## 4. World Fold Arm

All snippets target `extractor/native/recorder_host/src/world_fold.rs`.

Use `fold_draft.rs` as the lane-local tested draft. The snippets below show how to move it into the current `WorldFold` shape.

### 4.1 Import And Folded Domain Set

Add:

```rust
use naval_missions_status_live_lane as naval56;
```

Update:

```rust
pub const FOLDED_DOMAIN_IDS: &[u16] = &[10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 55, 56];
```

### 4.2 State Fields

Add to `WorldFold`:

```rust
naval_missions_status: BTreeMap<u32, NavalMissionTaskForceFoldRow>,
naval_missions_status_unknown_this_hour: bool,
```

Initialize in `WorldFold::new`:

```rust
naval_missions_status: BTreeMap::new(),
naval_missions_status_unknown_this_hour: false,
```

Reset in `reset_hour_state`:

```rust
self.naval_missions_status_unknown_this_hour = false;
```

Add struct near the other fold structs:

```rust
#[derive(Clone, Debug, Default)]
struct NavalMissionTaskForceFoldRow {
    task_force_id: u32,
    fleet_id: u32,
    owner_tag_db_id: u32,
    location_province_id: Option<u32>,
    strategic_region_id: Option<u32>,
    mission_enum: Option<u32>,
    mission_state: Option<u32>,
    home_base_province_id: Option<u32>,
    repair_base_province_id: Option<u32>,
    repair_progress_milli: Option<u32>,
    retreat_state: Option<u32>,
    sunk_convoys: Option<u32>,
    convoy_route_context_id: Option<u32>,
    fixture_required: bool,
    active_battle_fixture_required: bool,
    source_status: u32,
}
```

### 4.3 Dispatch Arm

Add in `fold_domain_data`:

```rust
56 => self.fold_naval_missions_status(payload),
```

### 4.4 Fold Function

Add after the 55 fold or near other full-replace folds:

```rust
fn fold_naval_missions_status(&mut self, payload: &[u8]) -> Result<(), String> {
    let decoded = naval56::decode_status_payload(0, payload)
        .map_err(|errors| errors.join("; "))?;
    let status_flags = decoded.header.status_flags;
    if compact_status_is_failure(status_flags) {
        self.naval_missions_status_unknown_this_hour = true;
        let failure_flags = status_flags & COMPACT_STATUS_FAILURE_MASK;
        self.note(format!(
            "naval_missions_status_live failure status {failure_flags:#x}; naval mission/status state unknown this hour"
        ));
        return Ok(());
    }

    let delta = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    if delta && !self.delta_baselines_seen.contains(&56) {
        self.naval_missions_status_unknown_this_hour = true;
        self.record_domain_problem(56, "delta_before_baseline");
        self.note(
            "naval_missions_status_live delta_before_baseline; naval mission/status state unknown this hour",
        );
        return Ok(());
    }

    // The first integration emits full-replace frames because domain 56 has no
    // deletion tombstone. Do not enable delta emission until delete semantics exist.
    if !delta {
        self.naval_missions_status.clear();
    }

    for row in decoded.rows {
        self.naval_missions_status.insert(
            row.task_force_id,
            NavalMissionTaskForceFoldRow {
                task_force_id: row.task_force_id,
                fleet_id: row.fleet_id,
                owner_tag_db_id: row.owner_tag_db_id,
                location_province_id: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_LOCATION,
                    row.location_province_id,
                ),
                strategic_region_id: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_REGION,
                    row.strategic_region_id,
                ),
                mission_enum: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_MISSION,
                    row.mission_enum,
                ),
                mission_state: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_MISSION_STATE,
                    row.mission_state,
                ),
                home_base_province_id: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_HOME_BASE,
                    row.home_base_province_id,
                ),
                repair_base_province_id: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_REPAIR_BASE,
                    row.repair_base_province_id,
                ),
                repair_progress_milli: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_REPAIR_PROGRESS,
                    row.repair_progress_milli,
                ),
                retreat_state: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_RETREAT_STATE,
                    row.retreat_state,
                ),
                sunk_convoys: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_SUNK_CONVOY_COUNT,
                    row.sunk_convoys,
                ),
                convoy_route_context_id: option_flag(
                    row.flags,
                    naval56::compact::FLAG_HAS_CONVOY_ROUTE_CONTEXT,
                    row.convoy_route_context_id,
                ),
                fixture_required: row.flags & naval56::compact::FLAG_FIXTURE_REQUIRED != 0,
                active_battle_fixture_required: row.flags
                    & naval56::compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED
                    != 0,
                source_status: row.source_status,
            },
        );
    }

    if !delta {
        self.delta_baselines_seen.insert(56);
    }
    Ok(())
}
```

Helper:

```rust
fn option_flag(flags: u32, flag: u32, value: u32) -> Option<u32> {
    (flags & flag != 0).then_some(value)
}
```

### 4.5 Snapshot Shape And Provenance

In `build_snapshot`, add a note when unknown:

```rust
if self.naval_missions_status_unknown_this_hour {
    notes.push("naval_missions_status_live state unknown this hour; accumulator retained for the next valid full frame/delta".to_string());
}
```

Build an `order_of_battle` value before the final `json!`:

```rust
let naval_missions_status_extra = if self.naval_missions_status_unknown_this_hour {
    Value::Null
} else {
    self.build_naval_missions_status_extra()
};
let order_of_battle = json!({
    "extra": {
        "naval_missions_status_live": naval_missions_status_extra,
    }
});
```

Then replace the current empty OOB placeholder:

```rust
"order_of_battle": order_of_battle,
```

If another concurrent worker has already populated `order_of_battle`, merge the `extra.naval_missions_status_live` object into that existing shape instead of replacing it.

Add the builder:

```rust
fn build_naval_missions_status_extra(&self) -> Value {
    let mut rows = Vec::with_capacity(self.naval_missions_status.len());
    for row in self.naval_missions_status.values() {
        rows.push(json!({
            "task_force_id": row.task_force_id,
            "fleet_id": row.fleet_id,
            "owner_tag_db_id": row.owner_tag_db_id,
            "location_province_id": option_u32_json(row.location_province_id),
            "strategic_region_id": option_u32_json(row.strategic_region_id),
            "mission_enum": option_u32_json(row.mission_enum),
            "mission_state": option_u32_json(row.mission_state),
            "home_base_province_id": option_u32_json(row.home_base_province_id),
            "repair_base_province_id": option_u32_json(row.repair_base_province_id),
            "repair_progress_milli": option_u32_json(row.repair_progress_milli),
            "retreat_state": option_u32_json(row.retreat_state),
            "sunk_convoys": option_u32_json(row.sunk_convoys),
            "convoy_route_context_id": option_u32_json(row.convoy_route_context_id),
            "fixture_required": row.fixture_required,
            "active_battle_fixture_required": row.active_battle_fixture_required,
            "source_status_flags": row.source_status,
        }));
    }
    rows.sort_by(|a, b| {
        let ak = (
            a.get("owner_tag_db_id").and_then(Value::as_u64).unwrap_or(0),
            a.get("fleet_id").and_then(Value::as_u64).unwrap_or(0),
            a.get("task_force_id").and_then(Value::as_u64).unwrap_or(0),
        );
        let bk = (
            b.get("owner_tag_db_id").and_then(Value::as_u64).unwrap_or(0),
            b.get("fleet_id").and_then(Value::as_u64).unwrap_or(0),
            b.get("task_force_id").and_then(Value::as_u64).unwrap_or(0),
        );
        ak.cmp(&bk)
    });
    json!({
        "source": "C",
        "confidence": "validated_slice",
        "task_force_count": rows.len(),
        "validated_location_rows": rows.iter().filter(|row| row.get("location_province_id") != Some(&Value::Null)).count(),
        "derived_region_rows": rows.iter().filter(|row| row.get("strategic_region_id") != Some(&Value::Null)).count(),
        "fixture_required_rows": rows.iter().filter(|row| {
            row.get("fixture_required").and_then(Value::as_bool).unwrap_or(false)
                || row.get("active_battle_fixture_required").and_then(Value::as_bool).unwrap_or(false)
        }).count(),
        "convoy_sunk_count_rows": rows.iter().filter(|row| row.get("sunk_convoys") != Some(&Value::Null)).count(),
        "task_forces": rows,
    })
}
```

Provenance hooks:

- A successful fold naturally adds `naval_missions_status_live` to `provenance.collector.domains` through `mark_domain(56)`.
- Failure status returns `Ok(())` without error so the snapshot is emitted with notes and the domain is still marked by the current `fold_domain_data` success path. If the serial worker wants failure domains excluded from `domains`, change `fold_domain_data` to support a "decoded but unknown" result. Do not silently clear task-force state on failures.
- Unknown domain 56 before this fold arm would appear in `unfolded_domains`; after adding the arm it should not.
- The delta guard records `domain_problems` with `problem="delta_before_baseline"` if a future delta frame arrives before a full frame.

## 5. compact_contract Descriptor Change

Current descriptor in `extractor/native/compact_contract/src/lib.rs`:

```rust
domain(
    56,
    "naval_missions_status_live",
    "naval_missions_status_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

Required descriptor when shared collector/recorder/fold registration lands:

```rust
domain(
    56,
    "naval_missions_status_live",
    "naval_missions_status_live",
    CompactDomainStatus::ValidatedSliceOpenGaps,
    true,
    true,
    &[],
),
```

Also update the production-profile test in `recorder_host/src/main.rs` that currently asserts `naval_missions_status_live` is excluded from `--production`.

## 6. R2 Replay Schema Delta

Additive schema text for `replay_interface/documents/REPLAY_SCHEMA.md` after shared integration:

```markdown
Domain 56 `naval_missions_status_live` folds under
`order_of_battle.extra.naval_missions_status_live`, not into the save-shaped
`order_of_battle.fleets[]` list. The live object is optional/null and has:

`{source:"C", confidence:"validated_slice", task_force_count,
validated_location_rows, derived_region_rows, fixture_required_rows,
convoy_sunk_count_rows, task_forces[]}`.

Each `task_forces[]` row carries `task_force_id`, `fleet_id`, `owner_tag_db_id`,
`location_province_id`, optional `strategic_region_id`, nullable
`mission_enum`, `mission_name`, `mission_state`, `home_base_province_id`,
`repair_base_province_id`, `repair_progress_milli`, `retreat_state`,
optional aggregate `sunk_convoys`, nullable `convoy_route_context_id`,
`fixture_required`, `active_battle_fixture_required`, and
`source_status_flags`.

Mission enum/name, detached-activity mission state, home base, and repair target
are exact producer-backed fields. Repair progress, task-force retreat-state
enum, and convoy-route fields stay null. `sunk_convoys` is an aggregate `CShip+0x3C`
counter only; it is not route context.
```

Rationale: `REPLAY_SCHEMA.md` says high-frequency raw OOB and ship telemetry live outside save-shaped OOB arrays, and R2 requires shapes beyond the documented schema to use a clearly named extra key.

## 7. Contract And Registry Row Wording After Integration

The shared registrations and scheduling are applied. This is intentionally
`VALIDATED_SLICE_OPEN_GAPS`, not full production acceptance: the integrated
speed-five/current-equivalent replay proof and the explicitly rejected semantic
carriers below remain separate.

`extractor/documents/DOMAIN_REGISTRY.md` row 56:

```markdown
| 56 | `naval_missions_status_live` | shared integration active: registered and scheduled validated task-force status slice; full-replace rows for task_force_id/fleet_id/owner/current province, optional derived region, exact mission enum/name, exact detached-activity mission_state, nullable home/repair-base provinces, and aggregate sunk-convoy count; repair-progress/task-force-retreat-enum/route carriers absent (save `retreat=yes` is separate) | naval missions/status |
```

`extractor/documents/MASTER_EXTRACTION_CONTRACT.md` compact domain row:

```markdown
| `naval_missions_status_live` / 56 | `VALIDATED_SLICE_OPEN_GAPS` | Lane-local compact/Rust packet plus active collector/recorder/fold registration and scheduling for the validated fleet/task-force/ship-root slice: task-force id, fleet id, owner tag-db id, current province, optional derived region, exact mission enum/name, exact persisted detached-activity mission_state, nullable home/repair-base provinces, and aggregate sunk-convoy count. Repair progress, task-force retreat-state enum, and convoy route/context remain absent; save `retreat=yes` is a separate parsed boolean; shared speed-five/.battleplan proof still required. |
```

Concept rows should stay conservative:

```markdown
| Naval missions | Patrol/strike/convoy/repair/training/hold, region assignment, mission state. | `PARTIAL` | Domain 56 promotes the exact mission enum/name and exact persisted detached-activity state 0..4; the exact-ten receiver/base join proves formatter subobject `+0x4B8` is body `+0x4A8`. | Preserve the exact enum and fail closed above 4. |
| Home base/repair/retreat | Port, home base, repair queue, retreat state. | `PARTIAL` | Domain 56 promotes exact home-base and repair-target province ids; exact-ten provenance is closed but live repair progress and retreat-state enum remain mechanically rejected/absent. Save OOB separately carries exact repair fields and boolean `retreat=yes`. | Preserve the exact save/live carrier separation. |
| Convoy combat/losses | Convoy involvement, losses, route/mission context. | `FIXTURE_BLOCKED` | Domain 56 carries aggregate `CShip+0x3C` sunk-convoy counters only. | Active convoy battle/loss route fixture, then compact convoy-event rows or exact rejection. |
```

## 8. What Remains Open Or Fixture-Blocked

Keep all of these out of the first shared integration:

- `repair_progress_milli`
- `retreat_state`
- `convoy_route_context_id`
- Route attribution for `sunk_convoys`
- Promotion of `CTaskForce+0x18` to home base
- Delta emission until the schema has delete tombstones or another exact deletion contract
- Derived strategic region if the collector lacks an in-DLL province-to-region map

Required evidence before any of those can be promoted:

- No further authority input is owed for the formatter/serializer roots: the
  exact-ten receiver/base join is complete and centrally bound. It promotes
  `mission_state`; body `+0x4B8/+0x4C0` remain object pointers and mechanically
  reject scalar repair-progress/retreat interpretations.
- Active convoy battle/loss route fixture tying losses to route/mission context rather than aggregate ship counters.

## 9. Verification Performed In This Packet

The exact workspace command requested is blocked until the serial worker adds the lane as a workspace member:

```text
cd extractor/native; cargo test -p naval_missions_status_live_lane
error: package ID specification `naval_missions_status_live_lane` did not match any packages
```

Historical lane-local standalone checks passed before this crate adopted the
shared `compact_contract` dependency:

```text
rustc --edition=2021 --test extractor/native/lanes/naval_missions_status_live/src/lib.rs -o extractor/native/lanes/naval_missions_status_live/lib_tests.exe
extractor/native/lanes/naval_missions_status_live/lib_tests.exe
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```text
rustc --edition=2021 --test extractor/native/lanes/naval_missions_status_live/fold_draft.rs -o extractor/native/lanes/naval_missions_status_live/fold_draft_tests.exe
extractor/native/lanes/naval_missions_status_live/fold_draft_tests.exe
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Generated `.exe` test artifacts were removed after verification.

Those bare-`rustc` commands are retained only as historical evidence and are
not the current build contract. The supported current validation command is:

```powershell
cargo test --offline --manifest-path extractor/native/lanes/naval_missions_status_live/Cargo.toml
```

## 10. Exact activity disposition (2026-07-20; supersedes Wave-2 conflict)

Pinned build `870ebf` exact-ten producer/consumer mining closes the task-force
receiver layout:

  activity subobject at body `-0x10`. Their subobject `+0x4B8` enum is therefore
  `detached_activity`.
- D56 emits that exact enum as `mission_state`: `0 none`, `1 repairing`,
  `2 moving_to_refit`, `3 refitting`, `4 reinforcing`; present zero is retained
  and values above four fail closed.
- Serializer body `+0x4B8/+0x4C0` are nullable object pointers, not scalar
  repair-progress or task-force retreat-state fields. Those two D56 columns
  remain mechanically rejected/absent. The retained-save census separately proves
  `task_force.retreat=yes` as a save-tick boolean (26 rows in 11 saves plus
  three control-melt rows); `parse_oob.py` extracts it without aliasing it to
  D56. D50 member states 8/0x10 are another separate combat-member carrier.
- The same retained census proves exact nested save mission fields, repair
  mode/target/link/activity fields, and province movement `path`; it finds no
  `repair_progress(_milli)`, `convoy_route(_context_id)`, or `route_id` in
  20,212 task-force rows. The movement path is not convoy route identity.
  Census SHA-256:
  `aee3ececd03321899a01782f78730523daf14ef5d916cdb0d25ace9dca9082a2`.
- The terminal sunk-convoy history carrier contains month, count, killer, and
  owner only. It has no route identity. D56 preserves cumulative
  `sunk_convoys` but no longer advertises a task-force-row fixture for a field
  absent from that carrier. The expanded active-combat census exposes client
  handles before collapse but lacks a retained route-manager join; the separate
  pre-collapse acquisition is specified in the naval v2 human-input register.

Evidence jobs/hashes are pinned in
`../naval_combat_live/evidence/A8_A9_B6_ARTIFACT_MANIFEST.json`. The historical
fixture bullets above remain provenance but are not active RE requirements.
