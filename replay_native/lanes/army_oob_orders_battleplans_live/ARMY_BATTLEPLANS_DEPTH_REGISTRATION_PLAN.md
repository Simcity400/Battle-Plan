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
- fail closed if domain-54 payload decode finds bad status codes, nonzero
  reserved fields, non-exact type-5 rows, child-table mismatches, skipped hours,
  or source/partial/worker/bridge failure bits.

## Field policy

Validated now:

- `CArmy` vtable RVA `0x293BC60`;
- order vtable RVA `0x296FAE8`;
- `CArmy+0x98` order vector;
- `order+0x30` order type;
- `order+0x38` army back-reference;
- type-5 state vector `order+0xE0/+0xE8`.

Fail-closed until a real redraw/toggle fixture or exact rejection:

- non-type-5 line geometry;
- battleplan execution/activation state;
- plan-value/power caches.

The module must emit those unsupported fields as `STATUS_FIXTURE_REQUIRED`.
It must not infer geometry or execution state from passive scans, saves, or
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
  for non-type-5 geometry/execution fields.

If the shared integration slot is unavailable, the exact blocker is:
domain-54 DLL-only production proof cannot be produced because the only
remaining change is shared collector/recorder registration for the prepared
lane-owned module and decoder.
