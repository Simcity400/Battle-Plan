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
