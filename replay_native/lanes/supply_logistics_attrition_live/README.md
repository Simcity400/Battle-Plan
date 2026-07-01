# supply_logistics_attrition_live

Lane-local production stack for current-build supply/logistics/attrition capture.

This lane is intentionally isolated from shared collector and recorder entrypoints. It packages
the live roots that are already safe for the DLL collector to read and keeps computed walls
explicitly badged:

- supply hubs, naval supply hubs, railways, and state infrastructure from live building vectors;
- division supply proxy rows from live division bodies;
- division supply-deficit and fuel proxy rows from the same validated division bodies;
- bounded CSupplySystem `+0x10` per-province efficiency/fulfilment input rows;
- exact per-province supply tooltip value as a computed/transient wall;
- attrition flag/reason as a runtime-computed wall until a field or function path is pinned.
- hub motorization settings, convoy/logistics pressure, and the remaining unsafe CSupplySystem
  province-input siblings as explicit wall/rejection badges until production roots are pinned.

Production integration uses the shared compact health/status field in the 48-byte payload header.
Lane-specific wall/rejection state is carried by the five required `computed_badge` rows, not by
private header bits, so the recorder's worker/source/partial failure gate remains meaningful.

## Current Build Roots

For `hoi4.exe` SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`:

- `module+0x3316E90` -> `CCurrentGameState`.
- `CGameState+0x2B0` / `+0x2BC` -> `CProvince*` array and count.
- `CProvince` vtable `module+0x295AF68`, `province_id u32@+0xA4`, `state_ptr@+0xC0`,
  province building vector `@+0x1C8`.
- `State` vtable `module+0x29206C0`, `state_id u32@+0x58`, state building vector `@+0x158`.
- `CBuilding` vtable `module+0x2973B88`, `type_id u32@+0x08`, `level u16@+0x40`.
- Division roots: `CGameState+0x310` country table -> country `+0x290` division wrapper vector
  -> wrapper `+0x10` division body; division vtable `module+0x2943CB0`, `type u32@+0x08`,
  `id u32@+0x0C`, `province u32@+0x18`,
  `max/gain/current supply + fuel @+0x5F0/+0x5F8/+0x600/+0x610`.
- CSupplySystem object: vtables `module+0x295D0A8` and `module+0x295D0D0`.
  `+0x10` is emitted as filtered per-province low-`i32`/100000 efficiency/fulfilment input
  rows. Raw out-of-range/packed `+0x10` entries are rejected per value. `+0x138` and `+0x150`
  remain un-emitted: bounded guards reject them on the current live frame.
- Stale 16022851 `CCountrySupplySystem` vtables `module+0x297B2F8/+0x297B348` and the
  bounded `+0x100F0` translation guess `module+0x298B3E8/+0x298B438` have zero instances on
  870ebf. Do not use them for motorization or convoy/logistics pressure.
- Current-build RTTI resolves `.?AVCCountrySupplySystem@@` to vtables
  `module+0x298BBF8` and `module+0x298BC48`. The v5 root proof found `469`
  double-vtable objects. This pins the replacement class pool, but not the per-country join,
  node-setting map, disrupted-supply, or foreign-homebase pressure fields. Those remain
  fixture-gated.

The previous Python logistics readers are still correctly build-gated to
`16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120` and must not be used
against the current `870ebf` process without rediscovery.

## Files

- `domain44_supply_logistics_attrition.rs`: compact domain 44 payload contract plus decoder/gate tests.
- `live_probe.py`: read-only current-build live proof runner. Writes `proof.json`, `PROOF.md`, and
  `domain44_payload.bin` under `replay_native/runs/supply_logistics_attrition_live_*`.
- `residual_probe.py`: read-only current-build residual scanner. It tests the bounded
  `CSupplySystem` structure hypothesis by scanning the vtable-shaped object for province-sized
  pointer/count fields, classifies numeric layouts, and writes `residual_probe.json` plus
  `RESIDUAL_PROBE.md`.
- `root_blocker_probe.py`: read-only current-build root blocker probe. It rejects stale
  `CCountrySupplySystem` roots for hub motorization and convoy/logistics pressure and validates
  the current CSupplySystem control root.
- `attrition_residual_probe.py`: read-only current-build division-body scalar probe. It scans
  simple attrition flag/reason hypotheses and keeps all candidates oracle-gated.
- `cprovince_residual_probe.py`: read-only current-build CProvince residual scanner. It rejects
  the stale 16022851 exact-supply chain on 870ebf, inventories current `CProvince+0x2C8`
  two-deref fixed-point candidates, and gates any exact-supply claim on same-frame tooltip anchors
  plus a temporal/network differential.
- `cprovince_residual_probe_tests.py`: stdlib unit tests for the candidate classifier and tooltip
  anchor gate.
- `RESIDUAL_FIXTURE_CONTRACTS.md`: exact same-frame oracle and differential contracts for the
  remaining exact supply, attrition, hub motorization, convoy/logistics pressure, and CSupply
  sibling-array residuals.
- `exact_supply_tooltip_anchors.template.json`: starter anchor file for the CProvince exact
  tooltip oracle contract.
- `save_supply_oracle_summary.py`: summarizes save-oracle `supply_system_2` anchors for
  motorization and convoy/logistics-pressure fixtures.
- `ccountry_supply_layout_probe.py`: read-only current-build `CCountrySupplySystem` layout probe.
  It validates the `+0x78` country backpointer and rejects stale direct country roots.
- `ccountry_supply_node_record_probe.py`: refines the `CCountrySupplySystem+0x20`
  country/node/tag record source and rejects record `+0x04` as an exact motorization level.
- `foreign_homebase_pressure_probe.py`: broad pressure-field scanner for save-oracle
  foreign-homebase anchors.
- `foreign_homebase_pressure_record_probe.py`: exact pressure-record refinement probe for
  `CCountrySupplySystem+0x1E0` foreign-homebase records. It writes a lane-local
  `domain44_pressure_candidate_payload.bin` when the candidate layout validates, with the
  convoy-pressure wall badge still mandatory.
- `hub_motorization_level_probe.py`: motorization fixture blocker and record-field scan. It
  confirms the current oracle has only level-`2` motorization anchors and cannot validate an
  exact level field without a same-frame mixed-level fixture.
- `Cargo.toml` / `src/bin/replay_pack_proof.rs`: lane-local compact stream and `.battleplan`
  packing proof for the domain 44 payload.
- `REGISTRATION_PATCH_PLAN.md`: exact minimal collector/recorder registration patch plan.

## Proof Commands

```powershell
python replay_native\lanes\supply_logistics_attrition_live\live_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_870ebf_20260701_v3
rustc --test replay_native\lanes\supply_logistics_attrition_live\domain44_supply_logistics_attrition.rs -o $env:TEMP\domain44_supply_tests.exe
& $env:TEMP\domain44_supply_tests.exe --nocapture
rustc replay_native\lanes\supply_logistics_attrition_live\domain44_supply_logistics_attrition.rs -o $env:TEMP\domain44_supply_decode.exe
& $env:TEMP\domain44_supply_decode.exe replay_native\runs\supply_logistics_attrition_live_870ebf_20260701_v3\domain44_payload.bin
python replay_native\lanes\supply_logistics_attrition_live\residual_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_residual_870ebf_20260701_v3
python replay_native\lanes\supply_logistics_attrition_live\root_blocker_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_roots_870ebf_20260701_v2
python replay_native\lanes\supply_logistics_attrition_live\attrition_residual_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_attrition_870ebf_20260701_v1
python replay_native\lanes\supply_logistics_attrition_live\cprovince_residual_probe_tests.py
python replay_native\lanes\supply_logistics_attrition_live\cprovince_residual_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_cprovince_870ebf_20260701_v2
python save_oracle\parse_supply.py "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --out-dir replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v1
python replay_native\lanes\supply_logistics_attrition_live\save_supply_oracle_summary.py replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v1\supply.json --out-dir replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v1
python replay_native\lanes\supply_logistics_attrition_live\ccountry_supply_node_record_probe.py --pid 45880 --supply-json replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3\supply.json --out-dir replay_native\runs\supply_logistics_attrition_live_ccountry_node_records_870ebf_20260701_v1
python replay_native\lanes\supply_logistics_attrition_live\foreign_homebase_pressure_record_probe.py --pid 45880 --supply-json replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3\supply.json --out-dir replay_native\runs\supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v1
python replay_native\lanes\supply_logistics_attrition_live\hub_motorization_level_probe.py --pid 45880 --supply-json replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3\supply.json --out-dir replay_native\runs\supply_logistics_attrition_live_hub_motorization_level_870ebf_20260701_v1
cargo run --manifest-path replay_native\lanes\supply_logistics_attrition_live\Cargo.toml --bin replay_pack_proof -- --payload replay_native\runs\supply_logistics_attrition_live_870ebf_20260701_v3\domain44_payload.bin --out-dir replay_native\runs\supply_logistics_attrition_live_replay_pack_870ebf_20260701_v1 --game-id supply44_v1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\supply_logistics_attrition_live_replay_pack_870ebf_20260701_v1\native_session.battleplan
python replay_native\lanes\supply_logistics_attrition_live\foreign_homebase_pressure_record_probe.py --pid 45880 --supply-json replay_native\runs\supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3\supply.json --out-dir replay_native\runs\supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v2
cargo run --manifest-path replay_native\lanes\supply_logistics_attrition_live\Cargo.toml --bin replay_pack_proof -- --payload replay_native\runs\supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v2\domain44_pressure_candidate_payload.bin --out-dir replay_native\runs\supply_logistics_attrition_live_pressure_replay_pack_870ebf_20260701_v1 --game-id supply44_pressure_candidate_v1
cargo run --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\supply_logistics_attrition_live_pressure_replay_pack_870ebf_20260701_v1\native_session.battleplan
```

Current gate coverage is eight tests: valid decode, header counter/missing badge rejection,
invalid division values, inconsistent supply-deficit/fuel-presence rejection,
province/division contract-flag rejection, malformed badge row rejection, and CSupply
input-row plus pressure-candidate validation/rejection.

Current lane-local live proof:

- `replay_native/runs/supply_logistics_attrition_live_870ebf_20260701_v3/PROOF.md`
- `replay_native/runs/supply_logistics_attrition_live_870ebf_20260701_v3/proof.json`
- `replay_native/runs/supply_logistics_attrition_live_870ebf_20260701_v3/domain44_payload.bin`
- `replay_native/runs/supply_logistics_attrition_live_attrition_870ebf_20260701_v1/ATTRITION_RESIDUAL_PROBE.md`
- `replay_native/runs/supply_logistics_attrition_live_roots_870ebf_20260701_v5/ROOT_BLOCKER_PROBE.md`
- `replay_native/runs/supply_logistics_attrition_live_save_oracle_870ebf_20260701_v1/SUPPLY_ORACLE_SUMMARY.md`
- `replay_native/runs/supply_logistics_attrition_live_replay_pack_870ebf_20260701_v1/REPLAY_PACK_PROOF.md`
- `replay_native/runs/supply_logistics_attrition_live_replay_pack_870ebf_20260701_v1/native_session.battleplan`
- `replay_native/runs/supply_logistics_attrition_live_ccountry_node_records_870ebf_20260701_v1/CCOUNTRY_SUPPLY_NODE_RECORD_PROBE.md`
- `replay_native/runs/supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v1/FOREIGN_HOMEBASE_PRESSURE_RECORD_PROBE.md`
- `replay_native/runs/supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v2/FOREIGN_HOMEBASE_PRESSURE_RECORD_PROBE.md`
- `replay_native/runs/supply_logistics_attrition_live_foreign_pressure_records_870ebf_20260701_v2/domain44_pressure_candidate_payload.bin`
- `replay_native/runs/supply_logistics_attrition_live_pressure_replay_pack_870ebf_20260701_v1/REPLAY_PACK_PROOF.md`
- `replay_native/runs/supply_logistics_attrition_live_pressure_replay_pack_870ebf_20260701_v1/native_session.battleplan`
- `replay_native/runs/supply_logistics_attrition_live_hub_motorization_level_870ebf_20260701_v1/HUB_MOTORIZATION_LEVEL_PROBE.md`

The lane-local replay pack proof wraps `domain44_payload.bin` in a compact `H4CR` stream,
packs it into `.battleplan`, reopens the container, verifies it, and byte-compares the embedded
stream. It validates CSupply `+0x10` rows through lane-local compact replay packing, but it is
not a shared DLL hourly proof. The whole supply concept remains open:
exact per-province supply, attrition reason, hub motorization, convoy/logistics pressure,
and CSupplySystem `+0x138` / `+0x150` arrays remain explicitly badged or rejected.

The shared registration proof
`replay_native/runs/supply_csupply_shared_20260701T194442/native_session_manifest.json`
proved DLL-only compact CSupply rows and Rust decode, but it is not accepted as
production-complete because `game_hours_advanced=0`. The first domain-44 read took
about `6.8s` due to in-frame heap-wide CSupply vtable discovery.
`csupply_root_path_probe.py` removes that specific blocker: on 870ebf,
`CSupplySystem = u64[CGameState+0x3D8]`, validated by vtables
`module+0x295D0A8/+0x295D0D0` and `+0x10` count `13_414`. Repeated root validation
averaged `27.719us` over `500` reads. The shared helper must use that direct root
and fail closed; heap-wide scan is preflight-only if kept at all.

## Residual Probe Contract

`residual_probe.py` does not retry the old `+0x10`, `+0x138`, or `+0x150` direct arrays as a
standalone path. It tests a broader structure hypothesis: all aligned pointer/count fields within
the first `0x400` bytes of the current-build `CSupplySystem` vtable object are scanned for
province-sized arrays and classified as `i32`, `u32`, `f32`, and `f64` layouts at 4- and 8-byte
strides. `+0x10` is already emitted as filtered bounded efficiency/fulfilment input rows. Any
new bounded sibling candidate remains `unproven` until a same-frame tooltip/save oracle or
controlled action fixture validates the field's semantic meaning.

The current blockers are exact fixture/root blockers, not acceptable omissions:

- exact per-province supply / remaining supply amount requires a same-frame province tooltip or
  save oracle for candidate validation. The stale 16022851 chain
  `CProvince+0x190 -> +0x348 -> i32@+0x108` is rejected on 870ebf: `+0x190` resolves to a
  module pointer (`0x7FF6AD972420`) across the current sample, not a heap leaf. Current-build
  CProvince probing narrows the search to `CProvince+0x2C8` two-deref supply-band candidates
  (`+0x218`, `+0x230`, `+0x2F8`, `+0x310`, `+0x348`, `+0x3D8`, `+0x428`), but all are
  `i32/100000` candidates requiring same-frame tooltip anchors and a temporal/network
  differential before emission;
- attrition flag/reason requires a controlled division attrition fixture with a visible or saved
  reason. `attrition_residual_probe.py` scans current division-body scalar candidates and found
  no production-safe attrition field without that oracle;
- hub motorization requires a controlled hub motorization toggle fixture and a current-build
  per-country node-setting field. The stale 16022851 `CCountrySupplySystem` path is rejected on
  870ebf by `root_blocker_probe.py`; v5 pins the current `CCountrySupplySystem` RTTI class pool
  (`module+0x298BBF8/+0x298BC48`). `ccountry_supply_node_record_probe.py` validates
  `CCountrySupplySystem+0x78 -> CCountry` and `CCountrySupplySystem+0x20` as
  `0x30`-stride country/node/tag records, but rejects record `+0x04` as the exact
  motorization level candidate (`21` matches, `96` mismatches against the freshest
  save oracle). `hub_motorization_level_probe.py` then proves the current oracle is
  fixture-blocked for exact level naming: `490` matched save pairs are all level `2`,
  and the best live candidate `u8@record+0x2D` still mismatches `20/117` matched
  actor-level records. The remaining blocker is a same-frame hub toggle fixture with at least
  two known levels, ideally one level `1` and one level `3`;
- convoy/logistics pressure requires a controlled overseas/convoy supply-pressure fixture and a
  current-build pressure field. The stale per-country supply-system path is rejected on 870ebf,
  while the current RTTI class pool is pinned. `foreign_homebase_pressure_record_probe.py`
  validates a concrete foreign-homebase pressure candidate:
  `CCountrySupplySystem+0x1E0` points to `0x70`-stride records with province `+0x00`,
  supply `i32/100000@+0x08`, penalty `i32/100000@+0x18`, and duplicate province `+0x50`.
  It matched `25/25` joined foreign-homebase provinces, `20` exact raw supply+penalty pairs,
  and all `25` duplicate province fields against the stale save oracle. This is still not
  emitted: the oracle is eight in-game days stale, tag `D04` is missing from the live join, the
  current save has zero disrupted-supply anchors, and convoy/port pressure semantics still need
  same-frame validation;
- any new `CSupplySystem` sibling path must satisfy province-indexed bounds and semantic oracle
  validation before it can replace the mandatory badge row.

See `RESIDUAL_FIXTURE_CONTRACTS.md` for the exact oracle and differential requirements.

## Compact Schema Delta

Domain 44 uses 40-byte rows. Legacy payloads use a 48-byte header with `csupply_input_rows`
at `+0x2C`; v2 lane-local payloads use a 52-byte header with `pressure_candidate_rows` at
`+0x30`. The decoder accepts both forms.
New row kind `4` carries CSupply per-province input rows:

- `entity_id`: province id.
- `flags`: `ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY`.
- `value_a_raw`: bounded low-`i32`/100000 efficiency value from `CSupplySystem+0x10`.
- `join_id`, `owner_tag_db_id`, `value_b_raw`, `value_c_raw`, `value_d_raw`, and `aux_raw`:
  must be zero.

New row kind `5` carries lane-local foreign-homebase pressure candidate rows. These rows are
not production pressure capture and do not remove the mandatory convoy-pressure wall badge:

- `entity_id`: province id from `CCountrySupplySystem+0x1E0` record `+0x00`.
- `owner_tag_db_id`: joined country tag id from `CCountrySupplySystem+0x78 -> CCountry`.
- `flags`: `ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT`.
- `value_a_raw`: candidate supply raw `i32/100000` from record `+0x08`.
- `value_b_raw`: candidate penalty raw `i32/100000` from record `+0x18`.
- `value_c_raw`: duplicate province id from record `+0x50`; gate requires it to equal
  `entity_id`.
- `value_d_raw`: record offset in the pressure region.
- `join_id` and `aux_raw`: zero.

The v2 pressure proof emitted `46` candidate rows plus the five mandatory wall badges, decoded
with zero parse errors, packed into `.battleplan`, and verified by `pack_replay`.

## Integration Patch Plan

See `REGISTRATION_PATCH_PLAN.md` and `CSUPPLY_ROOT_HARDENING_PATCH.md`. The shared registry
already has domain `44`; the CSupply row kind is registered and decodable, but the shared
collector must replace in-frame vtable discovery with the direct `CGameState+0x3D8` root and
then rerun speed-five/current-equivalent hourly proof.
