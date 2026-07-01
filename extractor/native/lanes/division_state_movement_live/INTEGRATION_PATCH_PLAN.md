# division_state_movement_live integration patch plan

No shared-entrypoint patch is needed for the current production domain-20 state
fields. The current collector already emits compact `divisions` records from a
DLL-native anchored root, and the lane proof at
`extractor/native/runs/division_state_movement_live_870ebf_20260701_v1` verifies
those records with the lane-local gate.

## Remaining shared integration slot

The remaining division-state backlog items that need a shared collector/recorder
slot are controller and manpower. Spawn/update/delete continuity is now proven
lane-locally by deriving lifecycle events from ordered domain-20 compact rows.

### Controller tag-db id

Prerequisite proof:

- Validate or reject `CDivision + 0x1D0` as controller tag-db id using a live
  expeditionary, borrowed, or otherwise owner/controller-divergent division.
- If all observed rows equal owner, keep the field out of production; equality
  alone is not proof of controller semantics.
- Executable fixture gate:
  `controller_candidate_probe.py --sample-limit 20000` plus
  `controller_fixture_gate.py --probe ... --oracle ...`. The oracle must contain
  at least one `owner_tag_db_id != controller_tag_db_id` division and the gate
  must join it to a live probe row where `CDivision+0x1D0` equals the oracle
  controller.

Patch shape after validation:

- `extractor/native/collector/src/lib.rs`
  - Add `DIVISION_OFF_CONTROLLER_TAG_DB_ID: usize = 0x1D0`.
  - Add `controller_tag_db_id: Option<u32>` to `DivisionMovementSample`.
  - Read it with the same `valid_division_owner_tag_db_id` bound.
  - Add `COMPACT_DIVISION_ROW_FLAG_CONTROLLER: u32 = 1 << 14`.
  - Pack the controller tag into the current row offset `+84`, replacing the
    reserved tail field. Row length stays 88 bytes.
- `extractor/native/recorder_host/src/main.rs`
  - Add the same controller flag.
  - Decode row offset `+84` as `controller_tag_db_id` when the flag is set.
  - Stop enforcing the tail field as reserved for domain 20.
  - Add `division_controller_rows` to `data_units_total`.
  - Reject controller values outside `1..=COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID`
    or values present without the flag.
- `extractor/native/lanes/division_state_movement_live/domain20_compact.rs`
  - Mirror the controller flag and `+84` decode.
  - Add tests for valid controller, value-without-flag reject, and delete rows
    retaining zero-filled tail data.

### Manpower

Current status:

- Per-division manpower is badged unavailable from the current domain-20 compact
  path by `manpower_compact_path_gate.py`.
- The badge is scoped: it proves the current compact row has no manpower field,
  no source/recorder manpower flag, no `DIVISION_OFF_*MANPOWER`, and row `+84`
  remains zero/reserved in proof output. It does not claim no manpower exists
  elsewhere in the game memory.

Prerequisite proof to add manpower later:

- Validate a current per-division manpower value from `CDivision`, a bounded
  route reachable from `CDivision`, or an explicitly derived formula joined to a
  time-aligned save/oracle.
- Reject country manpower pools, template manpower requirements, and equipment
  bank quantities as current per-division manpower proof.
- Gate the fixture with
  `extractor/native/lanes/division_state_movement_live/gap_contract_gate.py`.

Alternative owners until that proof exists:

- Template/design lane owns template manpower requirements.
- Country economy/manpower lane owns deployable/country manpower pools.
- Combat losses/stat lanes own killed/lost manpower deltas.
- Save oracle remains development validation only.

Patch shape after validation:

- `extractor/native/collector/src/lib.rs`
  - Add `manpower: Option<u32>` or a labelled derived value to
    `DivisionMovementSample`.
  - Add `COMPACT_DIVISION_ROW_FLAG_MANPOWER` only after an unused row slot or
    row-length extension is granted.
- `extractor/native/recorder_host/src/main.rs`
  - Decode the manpower field, count `division_manpower_rows`, and reject values
    outside the validated range/contract.
- `extractor/native/lanes/division_state_movement_live/domain20_compact.rs`
  - Mirror the row schema change and add tests for missing-flag/value mismatch,
    sane bounds, and delete-row zero-fill.

### Lane-local lifecycle proof

No shared patch is required for replay lifecycle continuity. The lane gate
maintains the live division-id set while decoding ordered compact records:

- first domain-20 data payload becomes baseline rows;
- later known non-delete rows are updates;
- later unknown non-delete rows are created/spawned for replay;
- later delete rows remove known live ids;
- unknown delete rows fail the gate.

The proof counter names are `division_lifecycle_baseline_rows`,
`division_lifecycle_created_rows`, `division_lifecycle_updated_rows`, and
`division_lifecycle_deleted_rows`.

### Proof gate after integration

Run a focused proof:

```powershell
cargo build --release --manifest-path extractor\native\Cargo.toml -p collector -p recorder_host
extractor\native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll extractor\native\target\release\collector.dll `
  --out-dir extractor\native\runs\division_state_movement_live_controller_spawn_<build> `
  --duration-s 20 `
  --sample-ms 0 `
  --dwell-s 0.25 `
  --min-game-hours 12 `
  --stream-format compact `
  --extraction-perf-gate `
  --lightweight-root-frame `
  --scheduled-domain divisions
```

Then run:

```powershell
rustc --edition 2021 extractor\native\lanes\division_state_movement_live\domain20_compact.rs -o $env:TEMP\domain20_compact.exe
& $env:TEMP\domain20_compact.exe `
  extractor\native\runs\division_state_movement_live_controller_spawn_<build>\collector_stream.bin `
  extractor\native\runs\division_state_movement_live_controller_spawn_<build>\domain20_gate_summary.json
```

Acceptance: ordered and decodable compact stream, `.battleplan` written,
hourly-or-better cadence, zero skipped hours, no source/partial/bridge/worker
failures, nonzero controller/manpower rows only when the prerequisite fixture
proof is satisfied, and lifecycle counters matching expected compact-row
continuity.
