# air_combat_losses_live domain 42 integration patch plan

Status: lane-local decoder/gate packaging only. Do not wire this into shared
collector or recorder entrypoints until an integration slot is granted.

## Lane-local artifacts

- `replay_native/lanes/air_combat_losses_live/domain42_compact.rs`
  - Domain: `42`
  - Name: `air_combat_losses_live`
  - Header length: `48` bytes
  - Row length: `68` bytes
  - Scale: `100000` equipment amount per plane
- `replay_native/runs/air_combat_losses_live_domain42_lane_local_20260701_v1/proof.json`

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

## Required collector patch

In `replay_native/collector/src/lib.rs`:

- Change `COMPACT_AIR_STRENGTH_DELTA_ROW_LEN` from `64` to `68`.
- Keep the current encoded field order produced by
  `compact_air_strength_deltas_data_payload`.
- Re-run `cargo check -p collector --manifest-path replay_native\Cargo.toml`.

## Required recorder_host patch

In `replay_native/recorder_host/src/main.rs`:

- Add domain 42 constants:
  - `COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN = 48`
  - `COMPACT_AIR_STRENGTH_DELTA_ROW_LEN = 68`
  - row flags `1`, `2`, `4`, `8`, `16`
  - scale `100000`
- Add domain data dispatch:
  - `42 => decode_compact_air_combat_losses_live_data_payload(offset, payload, &mut summary)`
- Add domain maps:
  - `compact_domain_id("air_combat_losses_live") => Some(42)`
  - `compact_domain_name(42) => "air_combat_losses_live"`
  - include `42` in `compact_domain_requires_data_record`.
- Port the lane-local decoder validation into
  `decode_compact_air_combat_losses_live_data_payload`.
- Add shared recorder tests equivalent to the lane-local tests:
  - loss/reinforcement/deleted rows decode cleanly
  - zero-row baseline data record decodes cleanly
  - bad scale and delta mismatch are rejected
  - encoded row length is exactly `68`, not `64`
  - stale `64`-byte payload shape is rejected
- Re-run `cargo check -p recorder_host --manifest-path replay_native\Cargo.toml`.

## Required live proof after integration

Run an integrated compact recorder proof with scheduled domain
`air_combat_losses_live`, then verify the produced `.battleplan`.

Acceptance remains blocked until the integrated proof shows:

- DLL-only capture: yes
- Compact stream: yes
- Rust decoded: yes
- `.battleplan` verified: yes
- Speed-5/current-equivalent hourly-or-lower: yes
- Zero skipped game hours: yes
- Partial/source/worker/bridge failures: no
