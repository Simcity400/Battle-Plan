# army_battleplans_depth_live registration-only plan

Domain assignment: `54 = army_battleplans_depth_live`.

This plan is registration-only. It must not be applied while another lane owns
the shared integration slot.

## Collector hook

Shared hook target:
`replay_native/collector/src/domain_registry.rs`.

Required registry change when the slot is granted:

- keep id `54`;
- keep canonical and wire name `army_battleplans_depth_live`;
- change status from `Reserved` to `ActiveIntegration` or
  `ProductionComplete` only after live proof;
- set `schedulable = true`;
- keep `requires_data_record = true`.

Shared coordinator hook target:
`replay_native/collector/src/lib.rs`.

Hook shape:

- schedule name `army_battleplans_depth_live` resolves to domain id `54`;
- the coordinator feeds the lane module with the same live
  `RawOrderObservation` values already read from `CArmy+0x98` order vectors;
- the coordinator supplies exact assigned-unit and order-linkage statuses from
  the same-frame `oob_command` read when available. If that same-frame join is
  unavailable, the domain-54 decoder must fail closed instead of publishing a
  row with guessed assignment or linkage status;
- for direct empty/inactive armies, army groups, and theatres, add a lane-owned
  bounded preflight/cache-refresh helper that reuses the validated
  `direct_oob_probe.py` vtable/type/id checks and passes prepared
  `DirectRootRow` values into the module. Do not add a separate domain-44
  registration; domain 44 is not available for this lane;
- do not use a one-time preflight address cache as the scheduled hot-loop
  source. `army_battleplans_depth_live_direct_warm_cache_870ebf_20260701_v5`
  rejects that candidate under live current-equivalent capture: 20 cached roots
  failed liveness per warm iteration, direct armies dropped from 477 to 465, and
  empty/inactive direct-army coverage dropped from 12 to 0 in the first four
  warm reads;
- do not use cached roots plus an arena-wide targeted handle scan as the
  refresh-on-mismatch source. `army_battleplans_depth_live_direct_targeted_refresh_870ebf_20260701_v3`
  repaired a synthetic current-build mismatch for one army, one army group, and
  one theatre, but needed to read 9,303,945,216 bytes in 13.094 seconds, above
  the 512 MiB bounded hot-loop threshold. This rejects that candidate family,
  not a future true manager/container helper;
- `army_battleplans_depth_live_direct_table_870ebf_20260701_v2` validates only a
  partial table-shaped direct-root candidate: a stride-8 theatre pointer vector
  at `0x22D05419560`, length `73`, covering `73/100` current-build theatre
  roots. The same scan found only `45/487` best army coverage and `18/255` best
  army-group coverage, so it does not validate a full direct army/group/theatre
  root manager table;
- `army_battleplans_depth_live_direct_table_870ebf_20260701_v3` rejects the
  complete plain contiguous/fixed-stride pointer-table path for scheduled direct
  roots. In the later live frame, the best single-run coverage was only
  `46/477` armies, `18/242` army groups, and `49/101` theatres; all 13
  empty/inactive armies were referenced somewhere, but `0/13` were covered by an
  accepted table run;
- the direct-root helper must either locate a complete current-build
  manager/container root, prove a bounded DLL-native refresh-on-mismatch
  strategy, or omit/badge direct-root rows fail-closed for that frame;
- the hook calls
  `army_battleplans_depth_module::collect_and_emit_depth_record`;
- the hook emits exactly one domain-54 data record for every scheduled frame;
- no save-reader, Python worker, passive heap scan, or cached battleplan value is
  allowed in the hot loop.

The lane-owned implementation is
`replay_native/lanes/army_oob_orders_battleplans_live/army_battleplans_depth_module.rs`.
It emits domain-54 payload bytes directly and does not require converting a
domain-27 stream.

## Recorder hook

Shared registry target:
`replay_native/recorder_host/src/compact_domain_registry.rs`.

Required registry change when the slot is granted:

- keep id `54`;
- keep canonical and wire name `army_battleplans_depth_live`;
- change status from `Reserved` to `ActiveIntegration` or
  `ProductionComplete` only after live proof;
- set `schedulable = true`;
- keep `requires_data_record = true`;
- attach the domain-54 data decoder.

Shared decoder target:
`replay_native/recorder_host/src/main.rs`.

Hook shape:

- dispatch domain id `54` to the lane decoder logic from
  `army_battleplans_depth_compact.rs`;
- preserve domains `27 = army_orders_live` and `28 = oob_command_live`
  unchanged;
- require one domain-54 data record per frame hour when scheduled;
- keep `.battleplan` packing as the existing raw compact stream entry;
- fail closed if domain-54 payload decode finds bad status codes, non-exact
  type-5 rows, child-table mismatches, non-deleted rows without exact
  assigned-unit/order-linkage status, deleted rows with live residual statuses,
  non-type-5 exact geometry rows without path-geometry child ranges, type-5
  rows with path-geometry ranges, path child-table mismatches,
  bad direct-root kind/id/flag/status fields, direct-root child-table
  mismatches, skipped hours, or source/partial/worker/bridge failure bits.

## Field policy

Validated now:

- `CArmy` vtable RVA `0x293BC60`;
- order vtable RVA `0x296FAE8`;
- `CArmy+0x98` order vector;
- `order+0x30` order type;
- `order+0x38` army back-reference;
- type-5 state vector `order+0xE0/+0xE8`;
- non-type-5 path vector `order+0x70/+0x78` materialized as domain-54
  path-geometry child rows;
- same-frame assignment and order-linkage status through existing
  `oob_command` compact rows for active armies.
- domain-54 direct-root compact section:
  - row kind `1 = army`, `2 = army_group`, `5 = theatre`;
  - 64-byte direct-root rows;
  - 8-byte direct-root member child rows;
  - `STATUS_COLD_PREFLIGHT_ONLY` for direct roots until the helper is moved into
    the DLL collector/coordinator path;
  - current proof carries 660 army roots, 470 army-group roots, and 97 theatre
    roots per frame from the validated direct-OOB proof.
- rejected direct-root integration candidate:
  - pure cached-address warm loop after one preflight scan;
  - rejection proof:
    `replay_native/runs/army_battleplans_depth_live_direct_warm_cache_870ebf_20260701_v5`;
  - reason: cached army, army-group, and division roots went stale immediately
    under live current-equivalent capture even though the warm phase performed no
    heap sweep.
- rejected direct-root refresh candidate:
  - cached roots plus targeted handle-scan refresh-on-mismatch;
  - rejection proof:
    `replay_native/runs/army_battleplans_depth_live_direct_targeted_refresh_870ebf_20260701_v3`;
  - candidate status:
    `REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH`;
  - reason: current-build synthetic stale roots for one army, one army group,
    and one theatre were recoverable, but recovery read 9,303,945,216 bytes,
    exceeding the 512 MiB bounded hot-loop threshold.
- validated partial direct-root table candidate:
  - proof:
    `replay_native/runs/army_battleplans_depth_live_direct_table_870ebf_20260701_v2`;
  - candidate status:
    `VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE`;
  - accepted row: stride-8 theatre vector at `0x22D05419560`, length `73`,
    covering `73/100` theatre roots;
  - not validated for scheduled army/army-group roots: best table-shaped army
    coverage is `45/487`, and best army-group coverage is `18/255`.
- rejected direct-root table candidate:
  - proof:
    `replay_native/runs/army_battleplans_depth_live_direct_table_870ebf_20260701_v3`;
  - candidate status:
    `REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE`;
  - strict empty-army table-resolution gate fails because empty/inactive armies
    are not fully covered by accepted table runs;
  - evidence: `0/13` empty/inactive armies in accepted table runs; best
    single-run coverage `46/477` armies, `18/242` army groups, `49/101`
    theatres.
- narrowed army-level scalar candidates:
  - proof:
    `replay_native/runs/army_battleplans_depth_live_army_scalar_870ebf_20260701_v2`;
  - candidate status:
    `NARROWED_ARMY_SCALAR_NEAR_MATCH`;
  - near-match candidates from current autosave vs validated live `CArmy` roots:
    `execution_type` at `CArmy+0x1A4` (`447/463`), `cohesion_type` at
    `CArmy+0x1A8` (`462/463`), and `proximity_type` at `CArmy+0x1AC`
    (`455/463`);
  - strict gate fails because these are not exact same-frame matches and width
    remains ambiguous (`u8` and `u32` both match low-value histograms).
- validated/rejected save-paired geometry candidates:
  - proof:
    `replay_native/runs/army_battleplans_depth_live_save_geometry_870ebf_20260701_v1`;
  - candidate status:
    `VALIDATED_PATH70_AND_REJECTED_TYPE3_C0_GEOMETRY`;
  - positive control:
    137/171 save/live paired type-5 rows exactly matched save `states` at
    `order+0xE0/+0xE8`;
  - validated slice:
    95/111 save/live paired non-type-5 rows exactly matched save `path` at
    `order+0x70/+0x78`;
  - rejected candidate:
    0/17 paired type-3 rows matched save `path` at `order+0xC0/+0xC8`, so do
    not retry that offset pair as the type-3 primary path vector;
  - production implication:
    the lane-local domain-54 schema now has an additive path-geometry child
    table; a shared integration slot must feed live `order+0x70/+0x78` vectors
    through the DLL collector and Rust decoder gate. The master
    `Battleplan geometry` row remains open until shared live proof and redraw/toggle
    semantics close.
- validated path-geometry compact rows:
  - proof:
    `replay_native/runs/army_battleplans_depth_live_path_geometry_compact_870ebf_20260701_v1`;
  - compact schema:
    domain-54 header length `52`, 64-byte order rows, 8-byte target-state child
    rows, 8-byte path-geometry child rows, 64-byte direct-root rows, and 8-byte
    direct-root member rows;
  - proof result:
    95 exact non-type-5 rows and 324 path-province child rows emitted from
    save/live validated `order+0x70/+0x78` rows;
  - gate result:
    Rust decode PASS, `.battleplan` verify PASS, compact-entry hash check PASS,
    zero skipped hours in the materialized proof stream, failure status flags
    `0`;
  - production implication:
    this removes the lane-local compact schema/feed blocker for exact path
    geometry rows. Shared DLL registration and redraw/toggle lifecycle evidence
    are still required.

Fail-closed until a real redraw/toggle fixture or exact rejection:

- non-type-5 line geometry beyond the validated `order+0x70/+0x78` path-vector
  slice and its required compact schema/feed expansion;
- battleplan execution/activation state;
- stance/aggressiveness live fields;
- plan-value/power caches.

The module must emit non-type-5 geometry and execution as
`STATUS_FIXTURE_REQUIRED`, and stance/aggressiveness as `STATUS_UNKNOWN` until a
current-build live offset is validated. The narrowed `CArmy+0x1A4/+0x1A8/+0x1AC`
scalar candidates are not production fields until a same-frame fixture or other
exact validation resolves the mismatches and width. Do not infer geometry,
execution, stance, or aggressiveness from passive scans, saves, UI labels, or
unchanged cached values.

## Required post-slot proof

Run a live capture at speed five/current equivalent:

```powershell
replay_native\target\release\collector_record.exe `
  --pid <hoi4_pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\army_battleplans_depth_live_integrated_<build>_<date>_v1 `
  --duration-s 30 `
  --sample-ms 0 `
  --min-game-hours 12 `
  --scheduled-domain army_battleplans_depth_live `
  --stream-format compact `
  --extraction-perf-gate `
  --lightweight-root-frame
```

Required gates:

- compact stream ordered and decodable;
- domain `54` seen as `army_battleplans_depth_live`;
- one domain-54 data record per frame hour;
- zero skipped game hours;
- no source-unavailable, partial-data, bridge-unavailable, or worker-panic
  flags;
- independent `pack_replay verify` pass for `native_session.battleplan`;
- lane decoder summary reports exact type-5 rows and fixture-required badges
  for non-type-5 geometry/execution fields until the validated `+0x70/+0x78`
  path-vector schema/feed expansion is integrated and proven;
- lane decoder summary reports exact assigned-unit and order-linkage rows for
  every non-deleted domain-54 row;
- lane decoder summary reports explicit unknown rows for stance/aggressiveness
  until those fields are validated or rejected.
- if direct roots are scheduled, lane decoder summary reports direct army,
  army-group, theatre, and member-child rows, with cold-preflight status only
  until the bounded helper is proven DLL-native.

If the shared integration slot is unavailable, the exact blocker is:
domain-54 DLL-only production proof cannot be produced because the only
remaining change is shared collector/recorder registration for the prepared
lane-owned module and decoder.
