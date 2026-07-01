# supply_logistics_attrition_live

Lane-local production stack for current-build supply/logistics/attrition capture.

This lane is intentionally isolated from shared collector and recorder entrypoints. It packages
the live roots that are already safe for the DLL collector to read and keeps computed walls
explicitly badged:

- supply hubs, naval supply hubs, railways, and state infrastructure from live building vectors;
- division supply proxy rows from live division bodies;
- division supply-deficit and fuel proxy rows from the same validated division bodies;
- exact per-province supply tooltip value as a computed/transient wall;
- attrition flag/reason as a runtime-computed wall until a field or function path is pinned.
- hub motorization settings, convoy/logistics pressure, and unsafe CSupplySystem province-input
  candidates as explicit wall/rejection badges until production roots are pinned.

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
- CSupplySystem candidate object: vtables `module+0x295D0A8` and `module+0x295D0D0`.
  Province-input candidate arrays at `+0x10`, `+0x138`, and `+0x150` are not emitted:
  bounded `i32/100000` guards reject them on the current live frame.

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

## Proof Commands

```powershell
python replay_native\lanes\supply_logistics_attrition_live\live_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_870ebf_20260701_v2
rustc --test replay_native\lanes\supply_logistics_attrition_live\domain44_supply_logistics_attrition.rs -o $env:TEMP\domain44_supply_tests.exe
& $env:TEMP\domain44_supply_tests.exe --nocapture
rustc replay_native\lanes\supply_logistics_attrition_live\domain44_supply_logistics_attrition.rs -o $env:TEMP\domain44_supply_decode.exe
& $env:TEMP\domain44_supply_decode.exe replay_native\runs\supply_logistics_attrition_live_870ebf_20260701_v2\domain44_payload.bin
python replay_native\lanes\supply_logistics_attrition_live\residual_probe.py --pid 45880 --out-dir replay_native\runs\supply_logistics_attrition_live_residual_870ebf_20260701_v2
```

Current gate coverage is six tests: valid decode, header counter/missing badge rejection,
invalid division values, inconsistent supply-deficit/fuel-presence rejection,
province/division contract-flag rejection, and malformed badge row rejection.

Current lane-local live proof:

- `proof_current_20260701_pid45880/PROOF.md`
- `proof_current_20260701_pid45880/proof.json`
- `proof_current_20260701_pid45880/domain44_payload.bin`

That proof validates only the accepted slice. It does not complete the whole supply concept:
exact per-province supply, attrition reason, hub motorization, convoy/logistics pressure,
and CSupplySystem province-input arrays remain explicitly badged or rejected.

## Residual Probe Contract

`residual_probe.py` does not retry the old `+0x10`, `+0x138`, or `+0x150` direct arrays as a
standalone path. It tests a broader structure hypothesis: all aligned pointer/count fields within
the first `0x400` bytes of the current-build `CSupplySystem` vtable object are scanned for
province-sized arrays and classified as `i32`, `u32`, `f32`, and `f64` layouts at 4- and 8-byte
strides. Any bounded numeric candidate remains `unproven` until a same-frame tooltip/save oracle
or controlled action fixture validates the field's semantic meaning.

The current blockers are exact fixture/root blockers, not acceptable omissions:

- exact per-province supply / remaining supply amount requires a same-frame province tooltip or
  save oracle for candidate validation;
- attrition flag/reason requires a controlled division attrition fixture with a visible or saved
  reason;
- hub motorization requires a controlled hub motorization toggle fixture;
- convoy/logistics pressure requires a controlled overseas/convoy supply-pressure fixture;
- any new `CSupplySystem` province-input path must satisfy province-indexed bounds and semantic
  oracle validation before it can replace the mandatory badge row.

## Integration Patch Plan

When the orchestrator grants an integration slot:

1. Domain ID 44 is wired into the shared compact collector/recorder registry.
2. `live_probe.py` root walkers are ported into `replay_native/collector/src/lib.rs` beside map
   detail and division compact readers, reusing the same CGameState roots and building/division
   helpers.
3. Domain 44 emits full snapshot rows on the scheduled compact cadence. Division rows include
   current/max/gain supply, computed deficit, fuel, owner, and province join IDs.
4. The five badge rows stay mandatory until exact per-province supply, attrition, motorization,
   convoy/logistics pressure, and CSupplySystem input candidates are pinned or replaced.
5. Gate `.battleplan` acceptance on domain 44 decode success, no row parser errors, hourly-or-better
   frame-end coverage, and zero skipped game hours.
