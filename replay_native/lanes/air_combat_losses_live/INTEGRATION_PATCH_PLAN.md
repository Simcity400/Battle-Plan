# air_combat_losses_live domain 42 integration patch plan

Status: accepted validated slice integrated in the current shared collector and
recorder. This is not full air-combat completion.

The accepted slice is air strength/equipment delta rows for losses,
reinforcements, deletes, mission region, and equipment amount deltas. Full
sortie lifecycle, combat pairing, kill attribution, cause/source of loss,
tactical engagement state, ace participation, and rich reinforcement reason
remain open unless a narrower blocker/rejection document says otherwise.

## Lane-local artifacts

- `replay_native/lanes/air_combat_losses_live/domain42_compact.rs`
  - Domain: `42`
  - Name: `air_combat_losses_live`
  - Header length: `48` bytes
  - Row length: `68` bytes
  - Scale: `100000` equipment amount per plane
- `replay_native/runs/air_combat_losses_live_domain42_lane_local_20260701_v1/proof.json`
- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session_manifest.json`
- `replay_native/lanes/air_combat_losses_live/FULL_EVENT_RECONSTRUCTION_BLOCKER.md`
- `replay_native/lanes/air_combat_losses_live/proofs/direct_event_root_rejection_20260701_v1/air_combat_losses_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/ccombat_side_event_root_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_ccombat_side_rejection_20260701_v1/ccombat_side_event_root_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/residual_route_matrix_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_residual_matrix_20260701_v5/residual_route_matrix_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/wing_leader_attachment_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_wing_leader_attachment_20260701_v2/wing_leader_attachment_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/ace_roster_route_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_ace_roster_route_20260701_v2/ace_roster_route_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/air_resolution_static_hook_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_target_disasm_870ebf.py`
- `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_static_hook_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_target_disasm_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/air_resolution_rva_contract_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_observed_vtable_rtti_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_target_disasm_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_callsite_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_wrappers/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_chunks_ace/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_internal_calls/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_internal_calls_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_interior_calls/air_resolution_rva_contract_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_rva_contract_20260701_v1_interior_calls_rtti/air_resolution_observed_vtable_rtti_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v4_pair_inner/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_callsite_probe_20260701_v2_pair_ace/air_resolution_callsite_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v7_interior_all_pdata/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v8_interior_all_unbounded/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v9_ace_callsite/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_callsite_probe_20260701_v5_ace_branch_context/air_resolution_callsite_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/air_resolution_dispatch_xref_probe_870ebf.py`
- `replay_native/lanes/air_combat_losses_live/air_resolution_dispatch_table_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_dispatch_xref_20260701_v1_ace_block/air_resolution_dispatch_xref_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_dispatch_xref_20260701_v2_ace_block_xref_context/air_resolution_dispatch_xref_probe_870ebf.json`
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v10_ace_caller_full/air_resolution_target_disasm_870ebf.json`
- `replay_native/runs/air_combat_losses_live_dispatch_table_20260701_v1_ace_block/air_resolution_dispatch_table_probe_870ebf.json`
- `replay_native/lanes/air_combat_losses_live/air_resolution_ace_payload_static_probe_870ebf.py`
- `replay_native/runs/air_combat_losses_live_ace_payload_static_20260701_v1/air_resolution_ace_payload_static_probe_870ebf.json`

## 68-byte row guard

The row is `68` bytes, not `64`.

Layout:

- `owner_tag_db_id:u32`
- `wing_id:u32`
- `generation:u64`
- `flags:u32`
- `mission_mask:u32`
- `mission_region:u32`
- `old_count:u32`
- `new_count:u32`
- `count_delta:i32`
- `reserved:u32`
- `old_equipment_amount_sum:i64`
- `new_equipment_amount_sum:i64`
- `equipment_amount_delta:i64`

The three `i64` equipment fields make the final row length `68`. The
lane-local tests include `encoded_payload_uses_68_byte_rows_not_64` and
`rejects_stale_64_byte_row_payload_shape` so stale `64`-byte integrations fail
review immediately.

## Current shared collector state

In `replay_native/collector/src/lib.rs`:

- `COMPACT_AIR_STRENGTH_DELTA_ROW_LEN` is `68`.
- The encoded field order matches `compact_air_strength_deltas_data_payload`.
- Domain `air_combat_losses_live` is registered as compact domain `42`.

## Current shared recorder_host state

In `replay_native/recorder_host/src/main.rs`:

- Domain 42 constants are present:
  - `COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN = 48`
  - `COMPACT_AIR_STRENGTH_DELTA_ROW_LEN = 68`
  - row flags `1`, `2`, `4`, `8`, `16`
  - scale `100000`
- Domain data dispatch is present:
  - `42 => decode_compact_air_combat_losses_live_data_payload(offset, payload, &mut summary)`
- Domain maps are present:
  - `compact_domain_id("air_combat_losses_live") => Some(42)`
  - `compact_domain_name(42) => "air_combat_losses_live"`
  - `42` is included in `compact_domain_requires_data_record`.
- The lane-local decoder validation is ported into
  `decode_compact_air_combat_losses_live_data_payload`.
- Shared recorder tests cover:
  - loss/reinforcement/deleted rows decode cleanly
  - zero-row baseline data record decodes cleanly
  - bad scale and delta mismatch are rejected
  - encoded row length is exactly `68`, not `64`
  - stale `64`-byte payload shape is rejected

## Validated slice live proof

Integrated compact recorder proof:

- `replay_native/runs/air_combat_losses_live_integrated_20260701_v1/native_session_manifest.json`

Observed result:

- DLL-only capture: yes
- Compact stream: yes
- Rust decoded: yes
- `.battleplan` verified: yes
- Speed-5/current-equivalent hourly-or-lower: yes
- Zero skipped game hours: yes
- Partial/source/worker/bridge failures: no
- Game hours advanced: `96`
- Domain data records: `97`
- Domain perf records: `97`

This proof validates the air strength/equipment delta slice only. Do not use it
to claim full sortie/combat event reconstruction.

## Residual candidate rejected after integration

The lane-local CCombatSide proof
`replay_native/runs/air_combat_losses_live_ccombat_side_rejection_20260701_v1/ccombat_side_event_root_probe_870ebf.json`
rejects the land-style combat-side/registry small-vector route on the current
870ebf profile:

- live air activity present during the scan: `465` wings, `199` region-bearing
  wings, `34` game hours advanced, `0` skipped hours, `23` loss-like deltas,
  and `10` reinforcement-like deltas;
- `CCombatSide` `module+0x29913C0` hits: `217`;
- exact populated side vectors: `77`, all division participants;
- air-wing side vectors: `0`;
- naval-unit side vectors: `0`;
- `CCombatRegistry` `module+0x29914D8` hits: `42`, with `41` type-`62`
  handles and one unrelated type-`10325` sample.

This narrows the full-event residual backlog by rejecting a direct
`CCombatSide`/registry event-root candidate. It does not change the compact
schema and does not complete sortie lifecycle, pairing, attribution, cause,
tactical state, ace, or rich reinforcement reason.

## Residual routes rejected or blocked after CCombatSide

Stable rooted residual matrix:

- proof:
  `replay_native/runs/air_combat_losses_live_residual_matrix_20260701_v5/residual_route_matrix_probe_870ebf.json`;
- command:
  `python replay_native\lanes\air_combat_losses_live\residual_route_matrix_probe_870ebf.py --pid 45880 --duration-s 10 --interval-s 0.10 --max-wing-bodies 96 --max-body-field-wings 96 --max-container-sources 48 --max-pool-sources 64 --max-region-sources 96 --max-one-hop-sources 192 --max-vector-records 1024 --max-record-scan-records 128 --object-span 0x280 --out-dir replay_native\runs\air_combat_losses_live_residual_matrix_20260701_v5`;
- live air activity present: `573` wings, `250` region-bearing wings,
  `17` game hours advanced, `0` skipped hours, `21` loss-like deltas,
  and `11` reinforcement-like deltas;
- roots scanned: `597` root sources, `192` one-hop sources, `20` active
  mission-region objects, `96` wing bodies, `368` valid air-base records;
- accepted vector headers inspected: `4553`;
- matrix candidates classified: `1301`;
- direct rooted opposed event candidates: `0`;
- rejected false routes: `47` air-base basing wing collections, `453` POD
  owner/wing-id collisions, `16` unanchored record-stride artifacts.

This rejects stable production-readable residual roots under
`StrategicAirManager`, active mission-region objects, air-base records,
air-wing pools/bodies, first-hop rooted manager objects, indirect vectors, and
POD-like record arrays. Tick-local/function-local air-resolution state remains
blocked until a controlled before/during/after fixture or air-resolution hook
exists.

Ace participation route status:

- wing-body leader proof:
  `replay_native/runs/air_combat_losses_live_wing_leader_attachment_20260701_v2/wing_leader_attachment_probe_870ebf.json`;
- country ace roster proof:
  `replay_native/runs/air_combat_losses_live_ace_roster_route_20260701_v2/ace_roster_route_probe_870ebf.json`;
- direct wing-body `CArmyLeader` / `CNavyLeader` attachments rejected as
  flying-ace participation because owners with `ace_count=0` had attachments
  and leader targets were shared across unrelated owners/wings;
- country ace roster root validated as `CCountry+0x12D8` entries with vtable
  `module+0x27C7DD0`, but direct refs from `833` air roots / `256` first-hop
  air roots to `154` pinned roster entries were `0`.

No compact schema or Rust decoder change is justified from these rejections.
The accepted compact domain remains the strength/equipment delta slice only.

## Static hook candidates after residual-route rejection

Static/Ghidra-backed hook discovery has narrowed the remaining forward path to
exact current-build RVAs, but has not validated a production event hook.

Proofs:

- static scan:
  `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_static_hook_probe_870ebf.json`;
- bounded target disassembly:
  `replay_native/runs/air_combat_losses_live_static_hooks_20260701_v3/air_resolution_target_disasm_870ebf.json`.

Key results:

- exe SHA-256 matched `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`;
- `25` air/update/combat RTTI names located;
- exact `CStrategicAirManager` update-body wrappers:
  `0xC57A50` -> `0xC350D0`, `0xC57960` -> `0xC34C80`,
  `0xC57AA0` -> `0xC35240`, `0xC57B40` -> `0xC35520`,
  `0xC57BE0` -> `0xC35800`, `0xC57C30` -> `0xC35970`;
- worker chunks worth instrumenting next:
  `0xC36380`, `0xC36610`, `0xC36D00`, `0xC36F90`,
  `0xC37380`, `0xC37760`, `0xC379F0`, `0xC37CE0`,
  `0xC37F70`;
- ace-management candidate: `0xC52840`;
- sortie-efficiency candidate: `0x1963700`;
- `CAirRegionCombatData` vtable `0x2942188` has useful static targets
  `0x1950F80` and `0x1950A30`, but these are serializer/dispatch-like and not
  validated live hooks.

Rejected tooling path:

- `Hoi4RootHunt` against the exact candidate VAs was not reliable because
  Ghidra collapsed targets into a giant containing function around
  `0x140082CB0` and spawned long-running decompiler workers. For this lane,
  use the `.pdata`-bounded Capstone evidence or repair Ghidra boundaries before
  retrying decompilation.

Next unblocked production path is bounded DLL-only instrumentation of the exact
remaining candidate RVAs below, or a controlled tick-synchronous fixture that
proves which candidate carries sortie lifecycle, combat pairing, attribution,
cause/source, tactical state, ace participation, and rich reinforcement reason.

## Runtime hook contract reduction

The lane-local Frida contract probe reduced the static candidate set. This is a
runtime proof/rejection artifact only; it does not add a production compact
event schema.

Commands:

```powershell
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 12 --max-hits-per-target 6 --target-group wrappers --target-group phases --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_wrappers
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 12 --max-hits-per-target 6 --target-group chunks --target-group ace_sortie --target-group combat_data --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_chunks_ace
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 45 --max-hits-per-target 12 --target-group ace_sortie --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_wrappers\air_resolution_rva_contract_probe_870ebf.json --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_chunks_ace\air_resolution_rva_contract_probe_870ebf.json --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_ace_sortie_long\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 8 --target-group internal_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_internal_calls_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 8 --target-group deep_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v2_deep_calls_air_fields_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_rva_contract_probe_870ebf.py --pid 45880 --duration-s 15 --max-hits-per-target 6 --target-group interior_calls --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls
python replay_native\lanes\air_combat_losses_live\air_resolution_observed_vtable_rtti_870ebf.py --contract-json replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls\air_resolution_rva_contract_probe_870ebf.json --out-dir replay_native\runs\air_combat_losses_live_rva_contract_20260701_v1_interior_calls_rtti
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v4_pair_inner --window-bytes 0x900 --max-instructions 160 --max-disasm-instructions 520 --max-evidence 180 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_callsite_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_callsite_probe_20260701_v2_pair_ace --preceding 16 --context-bytes 128
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v7_interior_all_pdata --window-bytes 0x900 --max-instructions 160 --max-disasm-instructions 520 --max-evidence 220 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v8_interior_all_unbounded --window-bytes 0x700 --max-instructions 180 --max-disasm-instructions 220 --max-evidence 160 --no-respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v9_ace_callsite --window-bytes 0x1200 --max-instructions 220 --max-disasm-instructions 700 --max-evidence 240 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_callsite_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_callsite_probe_20260701_v5_ace_branch_context --target-rva 0xC52840 --preceding 120 --context-from-function --branch-window 0x80 --max-incoming-branches 64
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_xref_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_xref_20260701_v1_ace_block --target-rva 0x6FA8AC --context-bytes 0x220 --after-bytes 0x40
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_xref_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_xref_20260701_v2_ace_block_xref_context --target-rva 0x6FA8AC --context-bytes 0x220 --after-bytes 0x40 --xref-context-bytes 0x120 --xref-after-bytes 0x80
python replay_native\lanes\air_combat_losses_live\air_resolution_target_disasm_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_target_disasm_20260701_v10_ace_caller_full --window-bytes 0x5200 --max-instructions 260 --max-disasm-instructions 6500 --max-evidence 2500 --respect-pdata-all
python replay_native\lanes\air_combat_losses_live\air_resolution_dispatch_table_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_dispatch_table_20260701_v1_ace_block --target-rva 0x6FA8AC --target-rva 0x6FA821 --target-rva 0x6FA8C4
python replay_native\lanes\air_combat_losses_live\air_resolution_ace_payload_static_probe_870ebf.py --out-dir replay_native\runs\air_combat_losses_live_ace_payload_static_20260701_v1
```

Results:

- wrapper/phase run: `21` game hours advanced, `72` hits, `6` matched
  wrapper-to-phase contracts, `0` Frida/script errors;
- rejected direct event-hook RVAs:
  `0xC57A50`, `0xC57960`, `0xC57AA0`, `0xC57B40`, `0xC57BE0`,
  `0xC57C30`;
- exact matched contract: phase entry receives
  `rcx = wrapper + 0x28`, `rdx = wrapper`, `r8 = wrapper + 0x08`;
- worker/chunk run: `21` game hours advanced, `54` hits, all nine worker
  chunks sampled, `0` Frida/script errors;
- chunk entry args are scheduler/work-slice shaped, not direct event payloads:
  `rcx = wrapper + 0x28`, `rdx = wrapper object`, `r8 = wrapper + 0x08`;
- rejected live-resolution RVAs:
  `0x1950F80` and `0x1950A30` remain `CAirRegionCombatData`
  serializer/dispatch endpoints;
- long ace/sortie run: `77` game hours advanced, `12` hits at `0x1963700`,
  `0` hits at `0xC52840`, `0` Frida/script errors;
- RTTI for `0x1963700` arguments:
  `rcx` vtable `0x2A0A588` = `.?AVCFEXAir@@`, `r8` vtable
  `0x29C6F18` = `.?AVCNavalCombat@@`.
- internal-call run: `23` game hours advanced, `120` hits, all `15` direct
  worker-call targets sampled, `0` Frida/script errors;
- internal-call RTTI resolved entry arguments to `CAirBase`, `CStrategicAir`,
  `CAirMission`, `CCountrySupplySystem`, and TBB scheduler body types. No
  entry argument carried observed `AirWingBody`, `AirWingPool`, `AirWingRegion`,
  or ace-entry vtables.
- deep-call run: `25` game hours advanced, `104` hits, `13` of `16`
  worker-interior callees sampled, `0` Frida/script errors. It validated
  single-wing `CAirWing` fields at `0xF4C430`, `0xF51060`, `0xF49710`, and
  `0xF4B6E0`, plus `CStrategicRegion` fields at `0xF652E0` and `0xF625B0`.
  These are scalar/single-wing calculation entries, not combat-event payloads;
- interior-call run: `25` game hours advanced, `168` hits, all `28` selected
  second-level callees sampled, `0` Frida/script errors. RTTI resolved ordinary
  gameplay objects including `CAirWing`, `CAirWingPool`, `CAirMission`,
  `CStrategicRegion`, `CAirBase`, `CStrategicAir`, equipment/subunit/country,
  weather, and army objects;
- rejected false pair hooks:
  `0xF46A10` ignores the incoming `rdx`, keeps only `rcx`, checks wing-local
  state at `CAirWing+0x38`, `+0xA0`, and `+0xA2C`, calls boolean helper
  `0xF4E0E0`, and returns small state constants. `0xF6B7B0` consumes only
  `rcx`, derives local `rdx/r8`, calls scalar helper `0xF49E80`, and returns a
  comparison result. The observed `CAirWing`/`CAirWing` and
  `CAirMission`/`CAirMission` shapes were stale-register false positives;
- ace callsite proof: byte-level xref scan found exactly one direct caller for
  `0xC52840`, callsite `0x6FA8BA` in runtime function `0x6F9170..0x6FE079`.
  Immediate call contract is `rcx = return(0x6C2D80(r15))`, `rdx = r14`.
- rejected region/base scalar helpers:
  `0xC44FA0` is a five-instruction `CStrategicAir` region-table accessor,
  `0xC448F0` is a `CAirBase` scalar resolver, and `0xE0CD40`,
  `0xE0CF00`, `0xF71050` write region aggregate ratios/sums to output
  pointers. They do not carry participants, loss cause/source, tactical state,
  ace entries, or reinforcement reason;
- rejected `0xF51060` helper subpath for ace/reinforcement attribution:
  `0xF51A60` is a bitmask-to-byte-index helper and
  `0x1004BF0`, `0x1004C40`, `0x1004C50`, `0x1004C80`, `0x1004E70` are
  `CSubUnitDefinition` / equipment variant helper wrappers or reset/copy
  routines;
- ace arg factory `0x6C2D80` resolved `0xC52840.rcx` to the selected country
  strategic-air container from `GameState+0x690`; the only unknown live
  payload at `0xC52840` is now `rdx = r14`, the caller command/event object.
- dispatch xref proof rejected ordinary direct branch/root xrefs to
  `0x6FA8AC` and `0x6FA8BA`; the only rel32-looking xref to `0x6FA8AC` was a
  false positive inside `mov qword ptr [rax+0x10], r12`;
- dispatch table proof decoded six `jmp rcx` command tables in caller
  `0x6F9170..0x6FE079`. Table `dispatch_jmp_6F9EB0` maps command id `0x2FB5`
  (`cmd_base 0x2F59`, index `0x5C`, selector `0x5`) to `0x6FA8AC`, the block
  that calls `0xC52840`. The nearby block `0x6FA8C4` maps to command id
  `0x2FB9`.
- ace payload static proof decoded `0xC52840`: `rdx` is copied to `r15`, the
  command discriminator is `dword [r15+0x30]`, and the only branch values are
  `0x31E2`, `0x199`, `0x1B`, and `0x34EC`. Payload fields read are
  `r15+0x28`, `+0x34`, `+0x38`, and `+0x44`; country ace-roster fields
  read/written are `r13+0x12D8`, `+0x12E0`, `+0x12E4`, and `+0x12E8`.
  Diagnostic strings include `Error scripting OOB`, `. The Ace '`,
  `Ace ignored.`, `. name set without airwing first`, and
  `. start_experience_factor set without airwing first`.

Candidate-set impact:

- do not retry wrapper RVAs as direct event hooks;
- do not retry `CAirRegionCombatData` serializer/dispatch RVAs as live
  resolution hooks;
- do not use `0x1963700` for domain-42 strategic-air residuals. It is
  carrier/naval sortie efficiency, not strategic-air sortie lifecycle, combat
  pairing, attribution, cause/source, tactical state, ace participation, or
  reinforcement reason;
- treat direct worker-call entries as live operational helper callbacks, not as
  already validated event payload hooks. Their entry contracts are air-base,
  strategic-air, mission, supply, or scheduler objects, not participant-pair or
  loss-attribution records;
- do not use `0xF46A10`, `0xF4E0E0`, `0xF6B7B0`, or `0xF49E80` for combat
  pairing or mission-pair extraction. Static function-body evidence rejects
  the runtime pair-looking argument shapes;
- do not use `0xC44FA0`, `0xC448F0`, `0xE0CD40`, `0xE0CF00`, or `0xF71050`
  for tactical state, loss attribution, cause/source, or reinforcement reason;
- do not use `0xF51A60` or the `0x1004B*` / `0x1004C*` / `0x1004E70` helper
  group for ace participation or rich reinforcement reason;
- do not retry phase bodies or worker chunks as direct event-payload hooks.
  They have been reduced to worker-interior single-wing/region/mission helpers
  and scheduler/work-slice entries;
- keep `0xC52840` only as a controlled command `0x2FB5` ace-management fixture
  candidate, not as a proven normal air-combat tick hook. The fixture must hit
  dispatch block `0x6FA8AC` / callsite `0x6FA8BA` and then validate or reject
  `rdx = r14`; `rcx` is already the country strategic-air container returned
  by `0x6C2D80`.
- reject `0xC52840` for sortie lifecycle, combat pairing, kill attribution,
  cause/source, tactical engagement state, and rich reinforcement reason. The
  bounded body is an OOB/ace command handler and country ace-roster mutator, not
  an opposed-wing combat event or loss/reinforcement attribution path.

## Proposed master checklist update

`replay_native/MASTER_EXTRACTION_CONTRACT.md` is orchestrator-owned. Do not
apply this lane-local proposal directly from the lane.

Proposed replacement row for `Ace identity/assignment`:

```markdown
| Ace identity/assignment | Ace roster, wing assignment, participation, death/wounding. | `FIXTURE_BLOCKED` | Air/ace lane-local roster proof; domain 42 residual proof narrows ace-management `0xC52840` to command id `0x2FB5` through dispatch block `0x6FA8AC` / callsite `0x6FA8BA`, with `rcx` resolved as the country strategic-air container and `rdx = r14` decoded as an OOB/ace command payload whose discriminator is `payload+0x30` and whose observed branch values are `0x31E2`, `0x199`, `0x1B`, and `0x34EC`. | Shared registration and speed-five proof for the roster slice, plus a controlled command `0x2FB5` ace assignment/loss fixture that validates or rejects `rdx = r14` fields `+0x28/+0x30/+0x34/+0x38/+0x44` for assignment, participation, death/wounding semantics. |
```

Evidence:

- `replay_native/runs/air_combat_losses_live_dispatch_table_20260701_v1_ace_block/air_resolution_dispatch_table_probe_870ebf.json`
  proves `dispatch_jmp_6F9EB0` maps command id `0x2FB5` to `0x6FA8AC`;
- `replay_native/runs/air_combat_losses_live_target_disasm_20260701_v10_ace_caller_full/air_resolution_target_disasm_870ebf.json`
  proves `0x6FA8AC` reaches callsite `0x6FA8BA -> 0xC52840` and preserves
  `rdx = r14`;
- `replay_native/runs/air_combat_losses_live_dispatch_xref_20260701_v2_ace_block_xref_context/air_resolution_dispatch_xref_probe_870ebf.json`
  rejects direct branch/root xrefs to `0x6FA8AC` as the fixture trigger.
- `replay_native/runs/air_combat_losses_live_ace_payload_static_20260701_v1/air_resolution_ace_payload_static_probe_870ebf.json`
  rejects `0xC52840` for sortie/combat/cause/reinforcement residuals and
  narrows the ace payload fields to `+0x28/+0x30/+0x34/+0x38/+0x44`.
