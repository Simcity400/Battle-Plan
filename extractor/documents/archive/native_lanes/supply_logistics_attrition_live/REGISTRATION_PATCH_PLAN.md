# Domain 44 Minimal Registration Patch Plan

Lane: `supply_logistics_attrition_live` / domain `44`.

This plan is intentionally limited to registering and hardening the already-proven
CSupply `+0x10` compact rows in the one shared DLL collector and Rust recorder.
It does not claim the whole supply/logistics/attrition residual slice is complete.

The no-design-left patch artifact is `SHARED_REGISTRATION_PATCH.md`. It contains the exact
collector helper, recorder decode arm, and synthetic test edits to apply inside the shared
domain 44 path.

Post-registration hardening artifact: `CSUPPLY_ROOT_HARDENING_PATCH.md`. The shared
registration run at
`extractor/native/runs/supply_csupply_shared_20260701T194442/native_session_manifest.json`
proved DLL-only CSupply rows and Rust decode, but did not meet hourly proof because the
first domain-44 read spent about `6.8s` in heap-wide vtable discovery. Replace that in-frame
scan with the validated direct root `CSupplySystem = u64[CGameState+0x3D8]`.

## Current Shared Anchors

- Collector registry already defines domain `44` as `COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE`
  and raw name `supply_logistics_attrition_live` in `extractor/native/collector/src/lib.rs`.
- Shared collector read anchor:
  `extractor/native/collector/src/lib.rs:15300` `read_compact_supply_logistics_attrition`.
- Shared collector payload anchor:
  `extractor/native/collector/src/lib.rs:23653` `compact_supply_logistics_attrition_data_payload`.
- Shared collector row/read structs:
  `extractor/native/collector/src/lib.rs:3466` `CompactSupplyLogisticsAttritionRead`,
  `extractor/native/collector/src/lib.rs:3805` `CompactSupplyLogisticsAttritionRow`.
- Shared recorder decode anchor:
  `extractor/native/recorder_host/src/main.rs:7334`
  `decode_compact_supply_logistics_attrition_data_payload`.
- Shared recorder compact registry already maps domain `44` to
  `supply_logistics_attrition_live` at
  `extractor/native/recorder_host/src/compact_domain_registry.rs:508`.

## Schema Patch

No new compact domain and no new `.battleplan` container format are required.

- Keep the domain id as `44`.
- Keep the 48-byte domain payload header.
- Keep 40-byte rows.
- Reinterpret header offset `+0x2C` as `csupply_input_rows` instead of reserved zero.
- Add row kind `4`: `CSupplySystem+0x10` per-province bounded input row.
- Add row flags:
  - `1 << 17`: `CSUPPLY_EFFICIENCY`.
  - `1 << 18`: `CSUPPLY_PROVINCE_ARRAY`.

Row kind `4` contract:

- `entity_id`: province id, `1..=13_413`.
- `join_id`: `0`.
- `owner_tag_db_id`: `0`.
- `flags`: exactly `CSUPPLY_EFFICIENCY | CSUPPLY_PROVINCE_ARRAY`.
- `value_a_raw`: low signed i32 from `CSupplySystem+0x10` at
  `array + province_id * 8`, bounded to `0..=100_000`.
- `value_b_raw`, `value_c_raw`, `value_d_raw`, `aux_raw`: `0`.
- One row per emitted province id; duplicate province ids fail decode.

## Collector Patch

Apply only inside the existing domain 44 collector path.

1. At `extractor/native/collector/src/lib.rs:3029`, add:
   - `COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4`.
   - `COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17`.
   - `COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18`.
2. At `extractor/native/collector/src/lib.rs:3466`, add
   `csupply_input_rows: u32` to `CompactSupplyLogisticsAttritionRead`.
3. At `extractor/native/collector/src/lib.rs:13179`, set `csupply_input_rows: 0` in
   `compact_supply_logistics_attrition_worker_panic_read`.
4. At `extractor/native/collector/src/lib.rs:15300`, call a new helper after division rows and
   before badge rows:
   `compact_supply_logistics_attrition_csupply_input_rows(state.base)`.
5. The new helper should use the direct current-build root validated by
   `csupply_root_path_probe.py`:
   - read `CGameState = u64[module+0x3316E90]`;
   - read `CSupplySystem = u64[CGameState+0x3D8]`;
   - validate adjacent vtables `module+0x295D0A8` and `module+0x295D0D0`;
   - read `object+0x10` as vector/array pointer plus count;
   - require count `13_414`;
   - require heap pointer and full array read of `count * 8`;
   - for province ids `1..13_413`, read low `i32` at `array + province_id * 8`;
   - emit only `0..=100_000` values as row kind `4`;
   - reject negative and `>100_000` entries per value without fabricating rows;
   - if the object, pointer, count, or read fails, emit zero CSupply rows and keep the
     existing mandatory CSupply badge row.
   - do not run heap-wide vtable discovery inside a scheduled domain-44 read. Any vtable
     fallback must be explicit preflight/warm-cache outside the speed-five recording window.
6. At `extractor/native/collector/src/lib.rs:23653`, write
   `read.csupply_input_rows` at header offset `+0x2C` instead of `0u32`.
7. `row_count` and `changed_rows` already derive from `read.rows`; after appending the new rows,
   no separate counter math is needed beyond storing `csupply_input_rows`.

## Recorder Patch

Apply only inside the existing domain 44 decoder.

1. At `extractor/native/recorder_host/src/main.rs:147`, add row kind `4`.
2. At `extractor/native/recorder_host/src/main.rs:164`, add the two CSupply flags to
   `COMPACT_SUPPLY_ROW_VALID_FLAGS`.
3. At `extractor/native/recorder_host/src/main.rs:7357`, read
   `csupply_input_rows` from header offset `+0x2C`; remove the reserved-zero parse error.
4. At `extractor/native/recorder_host/src/main.rs:7370`, change the row-count invariant to
   `province_rows + division_rows + badge_rows + csupply_input_rows == row_count`.
5. In the row loop after `seen_divisions`, add `seen_csupply_input_provinces`.
6. Add a match arm for row kind `4` with the exact row contract above.
7. After the loop, require decoded CSupply rows to equal `csupply_input_rows`.
8. Add summary units:
   - `supply_csupply_efficiency_rows`.
   - `supply_csupply_input_rows`.
9. Update the existing domain 44 recorder tests near
   `extractor/native/recorder_host/src/main.rs:12262` with one CSupply row and a malformed
   CSupply row rejection case. The lane-local decoder test
   `decodes_and_rejects_csupply_input_rows` is the reference contract.

## Required Proof After Registration

Run this after the shared collector and recorder patches are applied:

```powershell
cargo test --manifest-path extractor\native\recorder_host\Cargo.toml
cargo test --manifest-path extractor\native\snapshot\Cargo.toml
```

Then run a live domain 44 session with the real injected DLL collector:

```powershell
.\extractor\native\scripts\run_native_capture.ps1 -Domains supply_logistics_attrition_live -Speed 5
```

Acceptance for the shared patch requires:

- DLL-only capture: yes.
- Compact stream contains domain `44` data records with row kind `4`: yes.
- Recorder decodes domain `44` without parse errors: yes.
- `.battleplan` verifies with `pack_replay verify`: yes.
- Speed-five/current-equivalent hourly-or-lower frames: yes.
- Zero skipped game hours: yes.
- No source/worker/bridge/partial failures in the compact summary: yes.

Until that live shared proof exists, this lane remains `NOT COMPLETE` even though the lane-local
`.battleplan` pack proof passes.

Current shared status after registration but before root hardening:

- DLL-only compact stream: yes.
- Rust decoded: yes.
- `.battleplan` generated: yes.
- CSupply rows emitted: `supply_csupply_input_rows=13396`.
- No parse/source/worker/bridge/partial failures: yes.
- Hourly proof: no, `game_hours_advanced=0`.
- Blocking issue: first domain-44 read `read_time_us.avg=6824452`, caused by in-frame
  heap-wide CSupply vtable scan.

Root-hardening proof:

- `extractor/native/runs/supply_logistics_attrition_live_csupply_root_870ebf_20260701_v1/CSUPPLY_ROOT_PATH_PROBE.md`
- `CSupplySystem = u64[CGameState+0x3D8]`, repeated validation avg `27.719us`.
