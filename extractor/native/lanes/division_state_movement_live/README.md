# division_state_movement_live lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/division_state_movement_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane scope: compact per-division live state and deltas for replay. This lane is
intentionally local-first: it defines and gates the current domain-20 compact
wire contract without editing shared collector or recorder entrypoints.

## Current production roots

Current shared collector code already uses the DLL-native anchored division root:

- `CGameState + 0x310` country table.
- For each validated `CCountry`, division wrapper vector at
  `CCountry + 0x290/+0x298/+0x29C`.
- Each wrapper body is `wrapper + 0x10`.
- A `CDivision` body validates with vtable `module + 0x2943CB0`, type tag
  `4713` or observed wrapper alt type, and stable `div_id` at `+0x0C`.

The compact `divisions` row currently carries:

- identity: `div_id`, type `4713`;
- location and movement: `location`, `move_target_province`,
  `movement_source_province`, `movement_progress`, `progress_bucket`;
- combat state: `combat_ref_count`, `in_combat`;
- status scalars: organisation, strength, experience, max supply, current
  supply, supply gain, fuel;
- linkage: owner tag-db id, army id, template id;
- lifecycle: baseline rows, delta rows, and zero-filled delete rows.

The lane-local decoder derives replay lifecycle events from the ordered compact
domain-20 stream:

- first domain-20 data record: baseline live set;
- later non-delete row for an already live `div_id`: update;
- later non-delete row for an unseen `div_id`: created/spawned for replay;
- delete row for a live `div_id`: deleted;
- delete row for an unknown `div_id`: gate failure.

This proves spawn/update/delete continuity from current compact rows without a
shared collector flag. It does not claim that baseline rows spawned during the
capture; they are the initial replay state.

## Open lane gaps

`controller_tag_db_id` is not part of the current compact domain-20 production
row. The draft external offset notes `CDivision + 0x1D0` as likely controller,
but it is not validated against a diverged owner/controller case. Keep it out
of production rows until an expeditionary/borrowed-unit proof validates it.

Per-division current manpower is not pinned. `CDivisionTemplate` manpower and
country manpower pools are aggregate/design values, not current per-division
manpower. This lane accepts manpower only from a validated `CDivision` field,
a validated route reachable from `CDivision`, or an explicitly proven derived
contract joined to a time-aligned oracle. For the current domain-20 compact
path, manpower is badged unavailable by `manpower_compact_path_gate.py`: there
is no manpower flag, no `DIVISION_OFF_*MANPOWER` source offset, and the live
proof stream keeps row offset `+84` zero/reserved for every division row.

Equipment deltas are owned by the existing `division_equipment` compact domain.
The join key is `(owner_tag_db_id, div_id)` with optional `div_type=4713` when
surfaced. Domain 20 may keep `template_id` and `army_id` links, but it must not
inline held-equipment entries.

Army/order geometry is owned by `army_orders` and `oob_command`. Domain 20 owns
the division's live `army_id` link only. `army_orders` owns order type,
generation/slot, and target states keyed by `army_id`; `oob_command` owns army
and army-group membership, leaders, member division ids, and missing counts.

## Local gate

`domain20_compact.rs` is a standalone Rust encoder/decoder/gate for the current
compact `divisions` payload. Its H4CR framing parser is included directly from
the dependency-free canonical source at
`../../compact_contract/src/record_header.rs`, so these repo-root direct-`rustc`
commands still require no external crate:

```powershell
rustc --edition 2021 --test extractor\native\lanes\division_state_movement_live\domain20_compact.rs -o $env:TEMP\domain20_compact_tests.exe
& $env:TEMP\domain20_compact_tests.exe --nocapture

rustc --edition 2021 extractor\native\lanes\division_state_movement_live\domain20_compact.rs -o $env:TEMP\domain20_compact.exe
& $env:TEMP\domain20_compact.exe `
  extractor\native\runs\<run>\collector_stream.bin `
  --min-game-hours <positive-advancements> `
  extractor\native\runs\<proof>\domain20_gate_summary.json
```

The production CLI requires an explicit positive minimum expressed as game-hour
**advancements**. An `N`-advancement proof therefore requires at least `N+1`
consecutive closed frames. Every closed frame must contain exactly one canonical
domain-20 `DOMAIN_DATA` and one canonical domain-20 `DOMAIN_PERF` with matching
status. Its `FRAME_END` must be the physical final record, have exact
emitted/scheduled counts matching both the framed non-probe DATA/PERF record
counts and their distinct domain sets (so duplicate records cannot hide behind
a set cardinality),
carry the exact globally interpreted OR of those PERF statuses after canonical
lane-local normalization, use only known healthy status bits, and keep the
reserved word zero. Reserved domain-250 instrumentation PERF is physically
validated but excluded from coverage counts and frame status. The CLI fails on
any framing, sequence, record-flag, payload, status, lifecycle, duration, or
trailing-record violation.

## Gap fixture gate

`gap_contract_gate.py` is the executable contract for fixture-dependent fields:

```powershell
python extractor\native\lanes\division_state_movement_live\gap_contract_gate.py --self-test
python extractor\native\lanes\division_state_movement_live\gap_contract_gate.py `
  extractor\native\runs\<proof>\division_controller_proof.json `
  --out extractor\native\runs\<proof>\division_controller_gate.json
```

Controller proof must include at least one owner/controller-divergent division
where the candidate value, for example `CDivision+0x1D0`, matches the oracle
controller. Equality-only rows are rejected. Manpower proof must be per-division
current manpower; template, country-pool, and equipment-bank evidence is
rejected.

`controller_candidate_probe.py` is a read-only bounded probe for the specific
`CDivision+0x1D0` candidate. It follows the collector's anchored country-table
division roots and reports whether the current fixture contains a divergent
owner/controller case:

```powershell
python extractor\native\lanes\division_state_movement_live\controller_candidate_probe.py `
  --pid <hoi4-pid> `
  --out extractor\native\runs\<proof>\controller_candidate_probe.json
```

A `blocked_no_owner_controller_divergent_fixture` result is a precise fixture
block, not a controller validation.

`controller_fixture_gate.py` is the executable divergent-fixture contract. Run
the probe with enough rows to include the expected divergent division, then gate
it against an oracle built from the fixture save/scenario:

```powershell
python extractor\native\lanes\division_state_movement_live\controller_fixture_gate.py `
  --write-template extractor\native\runs\<proof>\division_controller_fixture_oracle.json

python extractor\native\lanes\division_state_movement_live\controller_candidate_probe.py `
  --pid <hoi4-pid> `
  --sample-limit 20000 `
  --out extractor\native\runs\<proof>\controller_candidate_probe.json

python extractor\native\lanes\division_state_movement_live\controller_fixture_gate.py `
  --probe extractor\native\runs\<proof>\controller_candidate_probe.json `
  --oracle extractor\native\runs\<proof>\division_controller_fixture_oracle.json `
  --out extractor\native\runs\<proof>\controller_fixture_gate.json
```

Acceptance requires at least one oracle row with `owner_tag_db_id !=
controller_tag_db_id`, the live `CDivision+0x1C8` owner matching the oracle
owner, and live `CDivision+0x1D0` matching the oracle controller.

`manpower_compact_path_gate.py` badges the current compact path as unavailable
for per-division manpower:

```powershell
python extractor\native\lanes\division_state_movement_live\manpower_compact_path_gate.py `
  --summary extractor\native\runs\<proof>\domain20_gate_summary.json `
  --out extractor\native\runs\<proof>\division_manpower_compact_path_gate.json
```

This badge is scoped to domain 20. Template manpower belongs to the future
design/template lane, country pools belong to country economy/manpower capture,
and combat/stat losses belong to their combat/stat lanes.
