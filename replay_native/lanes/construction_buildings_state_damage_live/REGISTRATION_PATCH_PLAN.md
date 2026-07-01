# construction_buildings_state_damage_live domain-48 registration patch plan

This is the registration-only integration plan for the lane-owned mini stack. It intentionally
does not edit shared collector or recorder entrypoints while another lane owns the active
integration slot.

## Registry authority

- Source of truth: `replay_native/domain_registry.json`.
- Assigned domain: `48 = construction_buildings_state_damage_live`.
- Wire token: `construction_buildings_state_damage_live`.
- Current target-state status: unavailable, fail-closed, explicitly badged with
  `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE = 0x10`.

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
   country construction roots, state roots, province roots, `module_base`, and `game_hour_raw`.
5. Append one compact domain-data record for domain 48 using the bytes returned by
   `collect_domain48_compact`.
6. Append one compact domain-perf record for domain 48 with row/unit counters from the lane gate.
7. Do not add map-control, state owner/controller, occupation, or target-state interpretation logic
   to the collector hook. The hook only adapts already-owned join keys into lane roots.
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
6. Keep unknown target-state semantics visible: non-empty construction rows with no target state are
   acceptable only when the domain-48 source flag `0x10` is present. The lane decoder already rejects
   silent omission, target-state emission while unavailable-badged, and partial target-state emission.

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
- target state remains absent with unavailable badge until `subobject+0xC0/+0xC8` are validated or
  rejected by a time-aligned live/save/UI fixture.
