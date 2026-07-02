# Domain 44 CSupply root hardening patch

Lane: `supply_logistics_attrition_live` / domain `44`.

This is the post-registration fix for the shared run
`extractor/native/runs/supply_csupply_shared_20260701T194442/native_session_manifest.json`.
That run proved DLL-only CSupply compact rows and Rust decoding, but the first
domain-44 frame took about `6.8s` because `CSupplySystem` discovery still used a
heap-wide vtable scan inside the recorded read.

## Validated Root

`csupply_root_path_probe.py` validates a bounded current-build root:

- `CGameState`: `u64[module+0x3316E90]`.
- `CSupplySystem`: `u64[CGameState+0x3D8]`.
- object vtables: `module+0x295D0A8`, `module+0x295D0D0`.
- province input array: `CSupplySystem+0x10`, count `13_414`.
- repeated validation timing: `27.719us` average over `500` reads.

Proof:

- `extractor/native/runs/supply_logistics_attrition_live_csupply_root_870ebf_20260701_v1/CSUPPLY_ROOT_PATH_PROBE.md`
- `extractor/native/runs/supply_logistics_attrition_live_csupply_root_870ebf_20260701_v1/csupply_root_path_probe.json`

Rejected roots:

- module static pointer to the CSupply object: none found;
- `CGameState` one-hop pointer-region reference: none found;
- direct `CCountry` pointer to the CSupply object: none found.

Useful corroboration:

- `CCountrySupplySystem+0x70` points back to the same CSupply object for `440`
  current-build objects, but discovering `CCountrySupplySystem` still requires a
  separate root/preflight and must not be the hot-loop CSupply path.

## Shared Collector Patch

Add a root constant near the existing domain-44 supply constants:

```rust
const CGAMESTATE_CSUPPLY_SYSTEM: usize = 0x3D8;
```

Replace in-frame heap scanning in
`compact_supply_logistics_attrition_csupply_input_rows` with direct root
resolution:

```rust
fn compact_supply_logistics_attrition_csupply_input_rows(
    module_base: usize,
    game_state: usize,
    tracker: &mut CompactSupplyLogisticsAttritionTracker,
) -> Vec<CompactSupplyLogisticsAttritionRow> {
    if let Some(object_ptr) = tracker.csupply_system_object {
        if compact_supply_logistics_attrition_csupply_object_valid(module_base, object_ptr) {
            if let Some(rows) =
                compact_supply_logistics_attrition_csupply_input_rows_from_object(object_ptr)
            {
                return rows;
            }
        }
        tracker.csupply_system_object = None;
    }

    let Some(object_ptr) =
        compact_supply_logistics_attrition_csupply_object_from_cgame_state(module_base, game_state)
    else {
        return Vec::new();
    };
    tracker.csupply_system_object = Some(object_ptr);
    compact_supply_logistics_attrition_csupply_input_rows_from_object(object_ptr)
        .unwrap_or_default()
}

fn compact_supply_logistics_attrition_csupply_object_from_cgame_state(
    module_base: usize,
    game_state: usize,
) -> Option<usize> {
    if !is_heap_pointer_aligned(game_state, 8) {
        return None;
    }
    let object_ptr =
        read_u64(game_state.checked_add(CGAMESTATE_CSUPPLY_SYSTEM)?)? as usize;
    compact_supply_logistics_attrition_csupply_object_valid(module_base, object_ptr)
        .then_some(object_ptr)
}

fn compact_supply_logistics_attrition_csupply_object_valid(
    module_base: usize,
    object_ptr: usize,
) -> bool {
    let Some(vtable0) = module_base.checked_add(CSUPPLY_SYSTEM_VTABLE0_RVA) else {
        return false;
    };
    let Some(vtable1) = module_base.checked_add(CSUPPLY_SYSTEM_VTABLE1_RVA) else {
        return false;
    };
    is_heap_pointer_aligned(object_ptr, 8)
        && read_u64(object_ptr).map(|value| value as usize) == Some(vtable0)
        && read_u64(object_ptr.saturating_add(8)).map(|value| value as usize) == Some(vtable1)
        && compact_supply_logistics_attrition_csupply_input_rows_from_object(object_ptr).is_some()
}
```

Update the call site in `read_compact_supply_logistics_attrition`:

```rust
let mut csupply_input_rows =
    compact_supply_logistics_attrition_csupply_input_rows(
        state.base,
        state.date_ptr as usize,
        tracker,
    );
```

Remove `compact_supply_logistics_attrition_find_csupply_object` from the scheduled
read path. If a heap-wide vtable scan is kept as a fallback, it must be called
only by an explicit preflight/warm-cache stage before scheduled recording. During
scheduled hourly reads, a missing or invalid direct root must fail closed by
emitting zero CSupply rows plus the existing CSupply badge.

## Required Shared Proof

Re-run the same shared live capture after the patch and require:

- `supply_csupply_input_rows > 0`;
- no parse/source/worker/bridge/partial failures;
- `read_time_us` and `frame_time_us` no longer include a multi-second first
  frame scan;
- speed-five/current-equivalent hourly proof met;
- zero skipped game hours;
- `.battleplan` verifies.
