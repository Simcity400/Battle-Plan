# air_wings_variants_lifecycle_live lane mini stack

The old slot-ready packet workflow is canceled. The orchestrator registry now
assigns this lane final compact domain `46 = air_variant_archetypes_live`.
Shared collector/recorder entrypoint edits should be registration hooks only.

Lane-owned production files:

- `Cargo.toml`: standalone lane crate for isolated tests.
- `src/domain46_air_variant_archetypes_live.rs`: compact schema, encoder, and
  Rust decoder/gate for domain 46.
- `src/extractor.rs`: lane-owned extractor core contract, validated live roots
  and offsets, same-session token-dictionary classification helpers.
- `src/lib.rs`: lane module exports.

Validated roots and offsets:

- `CGameState+0x690 -> StrategicAirManager`
- owner containers `+0x30/+0x3C`
- wing pool `+0x58/+0x64`, pool `+0x28`, object `+0x10`
- wing fields `id +0x0C`, count `+0x6C`, mission mask `+0x90`,
  mission region `+0xB0`, effective planes `+0x160`,
  equipment bank `+0x1E8`
- variant token/stat offsets `+0x18/+0x1C/+0x38`
- same-session token dictionary route:
  `CCountry+0x230 CFlagManager`, `CFlagManager+0x18` token-record vtable,
  token record `+0x08` id, token record `+0x10` text pointer

Compact schema:

- Domain: `46 = air_variant_archetypes_live`
- Header length: `48`
- Row length: `32`
- Header carries elapsed time, row/change counts, status flags, observed active
  variant count, classified/unresolved counts, static catalog hash, and fixed
  scale.
- Row carries variant id, row flags, equipment token raw, equipment type token
  raw, interface type raw, optional air-superiority weight, source mask, and
  string-tail tokens for archetype/base-airframe/role.
- Source bits include live stats, live definition, static catalog,
  rejected dev-save join, static heuristic, live token dictionary, and static
  source-enum closure.

Production proof:

- Final proof run:
  `replay_native/runs/air_wings_variants_lifecycle_live_modular_domain46_20260701_v3/`
- Command used the single DLL collector and Rust recorder with compact stream,
  `--stream-format compact`, `--extraction-perf-gate`,
  `--lightweight-root-frame`, and the non-loss air bundle plus domain 46.
- `.battleplan` was written:
  `native_session.battleplan` (`78933` bytes).
- Rust compact decode succeeded, domain coverage was OK, ordering was OK, and
  source/worker/bridge/partial failures were all false.
- Cadence gates passed:
  `hourly_requirement.met=true`, `game_hours_advanced=102`,
  `unique_game_hour_count=103`, `skipped_game_hours=0`.
- Domain 46 decoded `103` records, `112` rows, `112` classified rows, and
  `0` unresolved rows.
- The previous unresolved row is now intentionally fail-closed:
  first domain-46 data record row `97`, `variant_id=5085`,
  `equipment_token_raw=20378`, `equipment_type_token_raw=2`,
  `interface_type_raw=27`, `flags=2`, `source_mask=17`,
  archetype string `equipment_token_raw_20378`, empty base-airframe/role.
- Domain 46 read timing after amortization:
  avg `24104us`, max `42029us`, p95 `34741us`; frame max `89316us`,
  p95 `53643us`.

Registration-only patch plan:

1. Keep domain ID `46` from `replay_native/domain_registry.json`.
2. Shared collector hook uses the lane crate for active row closure and compact
   encoding; the remaining DLL-local memory walk is the coordinator adapter.
3. Shared recorder hook maps domain `46`/wire name `air_variant_archetypes` to
   the lane decoder/gate and only merges lane counters into the recorder
   summary.
