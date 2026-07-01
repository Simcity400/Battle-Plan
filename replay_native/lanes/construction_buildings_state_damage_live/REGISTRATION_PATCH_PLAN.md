# construction_buildings_state_damage_live domain-48 registration patch plan

This is the registration-only integration plan for the lane-owned mini stack. It intentionally
does not edit shared collector or recorder entrypoints while another lane owns the active
integration slot.

## Registry authority

- Source of truth: `replay_native/domain_registry.json`.
- Assigned domain: `48 = construction_buildings_state_damage_live`.
- Wire token: `construction_buildings_state_damage_live`.
- Current target-state status: lane-local root validated. Construction target
  location is `subobject+0x1D8 -> target_location +0x6C`; the collector emits
  `target_state_id` for every construction row only when that exact location
  resolves through state roots or province-to-state topology. Otherwise it
  clears all target states and sets
  `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.
- Validated target-location proof:
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v29_bounded_root_search_static_topology/target_state_bounded_root_search.json`
  plus
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v31_target_location_root_inspect_secondhop/target_location_root_inspect.json`.
- Rejected target-state candidates: `subobject+0xC0` and `subobject+0xC8` are
  rejected for state-building target rows by
  `replay_native/runs/construction_buildings_state_damage_live_20260701_v23_stable_line_state_rejection/stable_line_rejection.json`.

## Lane-owned modules

- Collector/extractor module:
  `replay_native/lanes/construction_buildings_state_damage_live/src/collector_module.rs`.
- Compact schema/encoder:
  `replay_native/lanes/construction_buildings_state_damage_live/src/lib.rs`.
- Decoder/gate module:
  `replay_native/lanes/construction_buildings_state_damage_live/src/decoder_gate.rs`.

## Shared patch, collector side

Registration-only edits after an integration slot is granted:

1. Add the lane crate as a collector dependency:
   `construction_buildings_state_damage_live = { path = "../lanes/construction_buildings_state_damage_live" }`.
2. Import only the lane entrypoint and root structs:
   `construction_buildings_state_damage_live::collector_module::collect_domain48_compact`.
3. Verify the generated/shared registry helper already exposes the registry descriptor:
   `construction_buildings_state_damage_live -> 48`. If the target branch lacks the descriptor,
   regenerate or add the registry descriptor only; do not hand-pick another id.
4. Reuse existing shared roots and adapters only:
   country construction roots, state roots, complete province-to-state province roots,
   `module_base`, and `game_hour_raw`.
5. Append one compact domain-data record for domain 48 using the bytes returned by
   `collect_domain48_compact`.
6. Append one compact domain-perf record for domain 48 with row/unit counters from the lane gate.
7. Do not add map-control, state owner/controller, occupation, or target-state interpretation logic
   to the collector hook. The hook only adapts already-owned join keys into lane roots; the lane
   module owns the `subobject+0x1D8 -> target_location +0x6C` extraction and all-or-none target
   emission.
8. If any root adapter is unavailable, emit no domain-48 data and surface source-unavailable/partial
   status rather than synthesizing fields.

## Shared patch, recorder side

Registration-only edits after an integration slot is granted:

1. Add the lane crate as a recorder dependency:
   `construction_buildings_state_damage_live = { path = "../lanes/construction_buildings_state_damage_live" }`.
2. Verify the generated/shared registry helper already maps the scheduled-domain token and wire
   name to id 48. If the target branch lacks the descriptor, add only the registry descriptor from
   `replay_native/domain_registry.json`.
3. Add a domain-data dispatch arm for id 48:
   call `construction_buildings_state_damage_live::decoder_gate::decode_domain48_compact(payload)`.
4. Translate the lane decoder summary into existing recorder summary units:
   `construction_rows`, `construction_rows_with_target_state`, `target_state_unavailable_badged`,
   `building_rows`, `damaged_building_rows`, and `resource_rows`.
5. Keep `compact_domain_requires_data_record(48) == true`.
6. Keep target-state semantics all-or-none: non-empty construction rows must either all carry
   target state or set the domain-48 source flag `0x10`. The lane decoder already rejects silent
   omission, target-state emission while unavailable-badged, and partial target-state emission.

## Exact registration hunks

These hunks are intentionally not applied by this lane owner. They are the exact shared edits the
orchestrator should serialize when domain 48 gets the integration slot.

### Registry descriptors

Flip domain 48 from reserved/non-schedulable to schedulable in both
`replay_native/collector/src/domain_registry.rs` and
`replay_native/recorder_host/src/compact_domain_registry.rs`:

```diff
 domain(
     48,
     "construction_buildings_state_damage_live",
     "construction_buildings_state_damage_live",
-    CompactDomainStatus::Reserved,
-    false,
+    CompactDomainStatus::ValidatedSliceOpenGaps,
+    true,
     true,
     &[],
 ),
```

### Collector Cargo/imports

Add the dependency to `replay_native/collector/Cargo.toml`:

```toml
construction_buildings_state_damage_live = { path = "../lanes/construction_buildings_state_damage_live" }
```

Add imports near the other compact lane imports in `replay_native/collector/src/lib.rs`:

```rust
use construction_buildings_state_damage_live::collector_module::{
    collect_domain48_compact, CollectorRoots as CbsdCollectorRoots,
};
use construction_buildings_state_damage_live::{
    CountryConstructionRoot as CbsdCountryConstructionRoot, LaneMemory as CbsdLaneMemory,
    ProvinceRoot as CbsdProvinceRoot, StateRoot as CbsdStateRoot,
    DOMAIN_ID as COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
};
```

Add the scheduled token beside the other `RAW_*_DOMAIN` constants:

```rust
const RAW_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE_DOMAIN: &str =
    "construction_buildings_state_damage_live";
```

Add the memory adapter once in `collector/src/lib.rs`:

```rust
struct CbsdProcessMemory;

impl CbsdLaneMemory for CbsdProcessMemory {
    fn read_u8(&self, addr: usize) -> Option<u8> {
        read_u8(addr)
    }
}
```

Add the read result and extractor helper inside the collector module:

```rust
struct CompactCbsdRead {
    compact: Vec<u8>,
    read_time_us: u64,
    row_count: u32,
    unit_count: u32,
    status_flags: u32,
}

fn read_compact_construction_buildings_state_damage_live(
    state: &CollectorState,
    game_hour_raw: i32,
    status_flags: u32,
) -> CompactCbsdRead {
    let started = Instant::now();
    let mut read_status_flags = status_flags;
    let roots = match build_compact_map_control_topology(state) {
        Ok((topology, _, _)) if state.country_identity.has_scalar_source() => {
            let country_roots = state
                .country_identity
                .rows
                .iter()
                .map(|row| CbsdCountryConstructionRoot {
                    tag_db_id: row.tag_db_id,
                    country_ptr: row.object_ptr,
                })
                .collect::<Vec<_>>();
            let state_roots = topology
                .states
                .iter()
                .map(|root| CbsdStateRoot {
                    state_id: root.state_id,
                    state_ptr: root.state_ptr,
                })
                .collect::<Vec<_>>();
            let province_roots = topology
                .provinces
                .iter()
                .filter(|root| root.state_id != 0)
                .map(|root| CbsdProvinceRoot {
                    province_id: root.province_id,
                    province_ptr: root.province_ptr,
                    state_id: root.state_id,
                })
                .collect::<Vec<_>>();
            CbsdCollectorRoots {
                module_base: state.base,
                game_hour_raw,
                country_roots,
                state_roots,
                province_roots,
            }
        }
        _ => {
            read_status_flags |= COMPACT_STATUS_SOURCE_UNAVAILABLE;
            CbsdCollectorRoots {
                module_base: state.base,
                game_hour_raw,
                country_roots: Vec::new(),
                state_roots: Vec::new(),
                province_roots: Vec::new(),
            }
        }
    };
    let output = collect_domain48_compact(&CbsdProcessMemory, roots);
    let row_count = output
        .gate
        .construction_rows
        .saturating_add(output.gate.building_rows)
        .saturating_add(output.gate.resource_rows)
        .min(u32::MAX as usize) as u32;
    CompactCbsdRead {
        compact: output.compact,
        read_time_us: duration_micros_u64(started.elapsed()),
        row_count,
        unit_count: row_count,
        status_flags: read_status_flags | output.gate.status_flags,
    }
}
```

Add the enable/read/join/append path in `build_compact_extraction_gate_records`:

```rust
let cbsd_enabled = compact_explicit_domain_enabled(
    control,
    RAW_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE_DOMAIN,
);
...
let cbsd_handle = if cbsd_enabled {
    Some(scope.spawn(move || {
        read_compact_construction_buildings_state_damage_live(state, game_hour_raw, status_flags)
    }))
} else {
    None
};
...
let cbsd_read = if let Some(handle) = cbsd_handle {
    Some(match handle.join() {
        Ok(read) => read,
        Err(_) => CompactCbsdRead {
            compact: Vec::new(),
            read_time_us: 0,
            row_count: 0,
            unit_count: 0,
            status_flags: status_flags | COMPACT_STATUS_WORKER_PANIC,
        },
    })
} else {
    None
};
...
if let Some(read) = cbsd_read {
    append_compact_domain_perf_record(
        &mut out,
        sequence,
        game_hour_raw,
        CompactDomainPerfRecord {
            domain_id: COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
            read_time_us: read.read_time_us,
            row_count: read.row_count,
            changed_rows: read.row_count,
            unit_count: read.unit_count,
            changed_unit_count: read.unit_count,
            status_flags: read.status_flags,
        },
        elapsed_wall_ms,
    );
    sequence = sequence.saturating_add(1);
    append_compact_record(
        &mut out,
        sequence,
        game_hour_raw,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &read.compact,
    );
    sequence = sequence.saturating_add(1);
    emitted_domains = emitted_domains.saturating_add(1);
}
```

### Recorder Cargo/dispatch

Add the dependency to `replay_native/recorder_host/Cargo.toml`:

```toml
construction_buildings_state_damage_live = { path = "../lanes/construction_buildings_state_damage_live" }
```

Add this helper to `replay_native/recorder_host/src/main.rs`:

```rust
fn decode_compact_construction_buildings_state_damage_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut CompactSummary,
) {
    match construction_buildings_state_damage_live::decoder_gate::decode_domain48_compact(payload) {
        Ok(decoded) if decoded.gate.ok => {
            add_compact_domain_data_unit(summary, 48, "construction_rows", decoded.summary.construction_rows);
            add_compact_domain_data_unit(summary, 48, "construction_rows_with_target_state", decoded.summary.construction_rows_with_target_state);
            add_compact_domain_data_unit(summary, 48, "building_rows", decoded.summary.building_rows);
            add_compact_domain_data_unit(summary, 48, "damaged_building_rows", decoded.summary.damaged_building_rows);
            add_compact_domain_data_unit(summary, 48, "resource_rows", decoded.summary.resource_rows);
            if decoded.summary.target_state_unavailable_badged {
                add_compact_domain_data_unit(summary, 48, "target_state_unavailable_badged", 1);
            }
        }
        Ok(decoded) => summary.parse_errors.push(format!(
            "offset {offset}: domain 48 gate failed {:?}",
            decoded.gate.errors
        )),
        Err(err) => summary
            .parse_errors
            .push(format!("offset {offset}: domain 48 decode failed: {err}")),
    }
}
```

Add the dispatch arm in the `COMPACT_RECORD_TYPE_DOMAIN_DATA` match:

```rust
48 => decode_compact_construction_buildings_state_damage_live_data_payload(
    offset,
    payload,
    &mut summary,
),
```

## Lane-local schedule proof before slot

The registration hunks above are backed by the lane-owned host-shaped schedule proof:

```powershell
cargo run --manifest-path replay_native\lanes\construction_buildings_state_damage_live\Cargo.toml --bin construction_buildings_state_damage_replay_pack_proof -- replay_native\runs\construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root 12
```

Proof artifact:
`replay_native/runs/construction_buildings_state_damage_live_20260701_v34_replay_pack_hourly_schedule_target_location_root/replay_pack_proof.json`.

Result: 12 consecutive game hours, 12 domain-48 perf records, 12 domain-48 data records, 12
frame-end records, 24/24 construction rows with target state, no target-state unavailable badge,
36 building rows, 12 damaged-building rows, 12 resource rows, zero skipped hours, byte-identical
`.battleplan` stream verification, and no source-unavailable, partial-data, worker-panic, or
bridge-failure condition. This proves the lane-owned collector/encoder/decoder path can satisfy the
hourly-or-lower schedule once the shared registry and dispatch hooks are applied; it is not a
DLL-only production capture because this lane did not edit or invoke shared entrypoints.

## Production proof command after slot

```powershell
cargo build --release --manifest-path replay_native\Cargo.toml -p collector -p recorder_host
replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\compact_construction_buildings_state_damage_live_<date> `
  --duration-s 30 `
  --sample-ms 0 `
  --min-game-hours 10 `
  --scheduled-domain construction_buildings_state_damage_live `
  --stream-format compact `
  --extraction-perf-gate
```

Acceptance for production completion:

- domain 48 compact stream records decode through the Rust recorder;
- `.battleplan` opens and verifies with byte-identical `collector_compact_stream`;
- speed-five/current-equivalent cadence is hourly-or-better;
- zero skipped hours;
- no `source_unavailable`, `partial_data`, `worker_panic`, or bridge-failure flags;
- construction target state is emitted for every row from the validated target-location root; the
  unavailable badge must be false on accepted production proof;
- do not retry `subobject+0xC0` or `subobject+0xC8` as the state-building target-state root; the
  v23 stable-line proof rejected those candidates.
