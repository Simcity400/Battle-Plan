# Division/OOB Collector Adapter Contract

Scope: future in-process collector output for live divisions, division equipment,
division templates, commanders, and order-of-battle command graphs. This file is
the Python adapter/provenance contract only. It must not be read as an offset
table, hook plan, or permission to claim a collector domain before a validator
and fold path exist.

The scheduled `fuel_live` path lives in `extractor/live_capture/collector_frames.py`,
`extractor/live_capture/tests/test_collector_frames.py`, and
`extractor/native/collector/src/lib.rs`. Keep future division/OOB work scoped away
from fuel-specific behavior unless the change is deliberately updating a shared
collector-frame contract.

## Existing Guardrail

`live_capture.collector_frames.SUPPORTED_SNAPSHOT_DOMAINS` is the collector
allow-list. `snapshot_from_collector_frame()` rejects any claimed domain outside
that set before writing collector provenance. A current test already proves that
a frame claiming `divisions` is rejected.

Do not add `divisions`, `division_equipment`, `oob_command`, or related domains
to `SUPPORTED_SNAPSHOT_DOMAINS` until the adapter has all of the following:

- payload validator for that domain;
- fold function that writes only measured schema leaves;
- provenance rule that appends the domain only after a successful fold;
- worker-boundary guard preventing heavy payloads in 10 ms poll frames;
- synthetic tests for accept, reject, and provenance omission;
- at least one live proof against a current build profile.

## Adapter Provenance Guard Shape

Future division/OOB domains need an accepted-domain guard, not just a claimed-
domain guard. The safe adapter order is:

1. Clean `frame["domains"]` into `claimed_domains`.
2. Reject `claimed_domains - SUPPORTED_SNAPSHOT_DOMAINS` before any fold.
3. Run frame-level validation for payload size, capture boundary, sequence, and
   common metadata.
4. Fold each conditional heavy domain through a domain-specific helper that
   either returns `True` after writing measured schema leaves, returns `False`
   when the domain was not claimed, or raises/reports a validation failure.
5. Build `accepted_domains` from folds that actually succeeded.
6. Write collector provenance with `accepted_domains`, not the original claimed
   list, once the fold outcome is known.

This avoids a future bug class where `divisions` or `oob_command` appears in
provenance even though the adapter dropped the payload for a bad row, stale pin,
join miss, or oversized body. Existing always-on light domains can keep their
current path only while their frame validator proves the claimed payload is
complete before provenance is written.

The first implementation should keep helper names private and boring:

- `_validate_collector_divisions_payload(frame) -> list[str]`
- `_fold_collector_divisions(snap, frame) -> bool`
- `_validate_collector_division_equipment_payload(frame, snap) -> list[str]`
- `_fold_collector_division_equipment(snap, frame) -> bool`
- `_validate_collector_oob_command_payload(frame) -> list[str]`
- `_fold_collector_oob_command(snap, frame) -> bool`

Validation helpers should not repair data. Fold helpers should not guess missing
fields. A helper that rejects a claimed domain must prevent that domain from
being listed in collector provenance.

## Collector Domain Names

Use the existing replay domain vocabulary where possible, but keep collector
support closed until a domain is implemented.

| Collector domain | Cadence | Replay target | Notes |
|---|---:|---|---|
| `divisions` | hourly scheduled snapshot | `snapshot["divisions"]` | First promotion target. Identity and map/status fields only in the first slice. |
| `division_equipment` | hourly scheduled snapshot | `divisions[].equipment_live` | Join by `(owner, div_id)` and optional `div_type` when present. Must not clobber save-shaped `equipment`. |
| `designs` | hourly or on-change snapshot | `designs.division_templates` | Needed for template id/type/name/role if not carried directly on division rows. Do not mix with equipment variants. |
| `designs_variants` | hourly or on-change snapshot | `designs.equipment_variants_live` | Needed to interpret held-equipment variant ids. Existing live-memory domain name is already distinct. |
| `commanders_live` | hourly or on-change snapshot | country commander leaves | Needed to resolve raw OOB leader ids into names/skill in UI. Not needed to claim command tree membership. |
| `oob_command` | hourly scheduled snapshot | `order_of_battle.{armies,army_groups,fleets,theatres}` | Command tree only. Ships remain `ships`-owned; battle-plan orders remain separate. |
| `construction_orders` | hourly scheduled snapshot | `order_of_battle.battle_plan_orders` | For order types and offensive target-state sets. Keep out of the first division slice. |
| future movement deltas | sub-hour event/delta stream | separate delta/event channel | Do not claim snapshot domains from a worker poll. Deltas need their own schema and loss rules. |

Cadence floor: scheduled snapshot domains are hourly-or-better. Movement/status
can later run sub-hour as deltas, but the first safe collector adapter should
write hourly snapshots only. Worker poll frames at 10 ms may carry timestamps,
diagnostics, sequence data, and tiny root status, not full division/OOB payloads.

Source marker rule: a replay field counts as collector-sourced (`C`) only when
`provenance.collector.domains` includes the exact collector domain after that
domain's validator and fold both succeed. A preflight, cached root, or diagnostic
payload is not captured replay state. For the first slice this means `divisions`
can source identity/status leaves in `snapshot["divisions"]`; `division_equipment`,
`oob_command`, and movement/order deltas must not be implied by that marker.

## Replay Schema Needs

### `divisions`

The first accepted collector `divisions` payload should require these fields per
row:

- `div_id`: int, required.
- `div_type`: int, required for stable Clausewitz identity even though the
  current schema validator only requires `div_id`.
- `owner`: valid country TAG or null.
- `location_province`: int or null.
- `state_id`: int or null when known from map/state join.
- `template_id`: int or null.
- `template_type`: int or null.
- `template_name`: string or null.
- `template_role`: string or null.
- `organisation`: number or null.
- `strength`: number or null.
- `experience`: number or null.
- `moving`: bool or null.
- `movement_source_province`: int or null. This is not yet a schema dataclass
  field but native map code already reads it when present; keep it optional.
- `move_target_province`: int or null.
- `move_progress`: number or null. Treat as the engine progress metric, not a
  normalized 0..1 percentage.
- `in_combat`: bool or null.
- `supply_status`: number or null.

Do not write these save/full-body fields from collector until they have current
live roots and validation: `encircled`, `army_manpower`, `killed`, `dig_in`,
`dig_in_cap`, `acclimatization`, `division_name`, `equipment_archetypes`,
`has_officer`, `max_supply`, `supply_gain`, and `motorization_level`. Some are
valid future targets, but absence is better than guessed data.

### `division_equipment`

Required payload shape:

- top-level `by_div` object keyed as `"TAG|div_id"` for parity with the existing
  live-memory assembler, or an adapter-normalized list folded into that shape;
- each record has `owner`, `div_id`, optional `div_type`, and `equipment`;
- `equipment` is `{variant_id_string: nonnegative_number}`;
- raw owner tag-db ids and object pointers are stripped before schema fold.

The adapter should reject a claimed `division_equipment` domain unless the same
snapshot also contains accepted `divisions` rows or an explicit same-hour join
target is defined. The smallest safe rule is same-frame join only.

### `designs` / `designs_variants`

Minimum division-template needs for authentic replay:

- template id/type pair;
- owner TAG or country id that can be resolved to TAG;
- template name and role/icon category;
- battalion/support structure when available;
- manpower/equipment requirements only after exact live roots are validated.

Equipment-variant needs:

- variant id/type or stable variant id;
- owner TAG when resolvable;
- archetype/equipment type;
- localized name key or raw name token;
- module map and upgrade levels only when validated.

### `oob_command`

The collector `oob_command` payload must carry all required command leaves as
lists if the domain is claimed:

- `armies`: list of `{id, owner, leader_id, army_group_id, member_div_ids,
  member_count, live_member_count, missing_count, name?}`.
- `army_groups`: list of `{id, owner, leader_id, member_army_ids, name?}`.
- `fleets`: list with task forces nested or explicitly referenced by id.
- `theatres`: list.
- optional `orphan_task_forces`: list.

Leader ids, army ids, army-group ids, fleet ids, task-force ids, ship ids, and
member division ids are raw engine ints. Only `owner` is a resolved TAG.

### `construction_orders`

For battle-plan orders, keep the existing measured shape:

- `army_id`: int.
- `order_type`: int or null.
- `target_states`: list of int for confirmed type-5 offensive orders, otherwise
  null.

Never fabricate an empty `target_states` list for resisted order geometry.

## Root And Preflight Requirements

Before any of these domains can be emitted by the DLL path, the collector needs
a profile/preflight result that is stable for the running process and cheap to
validate per scheduled snapshot.

Required common roots:

- exact `hoi4.exe` build/profile gate and module base;
- date/hour boundary already used by scheduled snapshots;
- tag table / country database resolution for owner TAGs;
- mod/static profile hashes for template and equipment interpretation;
- per-run pipe token and monotonic sequence handling.

Candidate live sources, all build-specific and fail-closed:

- `divisions`: CDivision body pool. Existing external research locates bodies by
  CDivision vtable plus type tag 4713 and validates body id. The DLL path should
  not do a whole-heap scan in the hot loop. Use a preflight/root-derived address
  pool, then per-frame validate vtable, type, id, owner, and sane field bounds.
- `division_equipment`: CDivision held-equipment bank off the division body.
  Validate the division identity first, then validate each variant entry against
  the equipment-variant vtable/type and sane counts.
- `designs` and `designs_variants`: design/template and variant registries.
  Prefer root-derived registry pointers or an on-change cache; if only heap scan
  discovery exists, keep it as development validation, not speed-five runtime.
- `oob_command`: CArmy/CArmyGroup/CFleet/CTaskForce/CTheatre/CDivision/CShip
  object sets plus command-edge fingerprint. Existing live-memory code validates
  vtable plus stable handle ids and a mutable structure fingerprint. The DLL
  path needs the same class of proof, but via bounded preflight roots rather than
  hot-loop arena scans.
- `commanders_live`: commander pools for leader details. Command tree can ship
  raw leader ids without this; UI name/skill resolution waits for this domain.

If a pin validates, re-read volatile scalar/status fields live. If a pin misses,
drop the domain for that snapshot, record a diagnostic, and request/perform
bounded rediscovery outside the hook callback. Do not serve stale cached decode
after a mutable-edge fingerprint miss.

## Validation Failure Modes

A collector frame must fail closed in these cases:

- Claimed domain is not in `SUPPORTED_SNAPSHOT_DOMAINS`.
- Claimed domain has missing payload, wrong top-level type, or exceeds payload
  size/object-count budget.
- Snapshot claims a heavy domain from `capture_boundary = "worker_poll"`.
- `divisions` row lacks int `div_id` or int `div_type`.
- Duplicate division identity `(div_id, div_type)` or ambiguous duplicate
  without type.
- Owner field is neither a valid TAG nor null.
- Province/state/template ids are negative or outside current profile bounds.
- Numeric scalar is NaN, infinite, or outside conservative sane bounds.
- Movement source/target is present without a valid province id or without a
  validated movement/order object when the field depends on one.
- `division_equipment` has no accepted division join target.
- `division_equipment.equipment` is not a dict, contains negative counts, or
  carries unreadable/ambiguous empty banks as measured truth.
- `oob_command` omits any required command leaf or uses a non-list leaf.
- `oob_command` object identity validates but mutable membership/parent/leader
  fingerprint changed.
- Raw diagnostic addresses, owner tag-db ids, or pointer values would leak into
  schema payloads.

Provenance rule: append the collector domain only after the fold succeeds. A
failed or omitted domain may be visible in diagnostics, but must not be listed
as captured replay data.

## Payload Budget

Initial hourly `divisions` payload is acceptable at speed five if it stays near
one compact row per live land division. A practical starting budget:

- hard cap `MAX_DIVISION_ROWS = 20000`;
- reject rows with large nested objects in the first slice;
- no per-division equipment in the first `divisions` domain;
- no template battalion tree in the first `divisions` domain;
- no command tree in the first `divisions` domain;
- explicit byte cap per scheduled snapshot domain, recorded in diagnostics when
  exceeded.

Expected ordering for payload growth:

1. `divisions` identity/status rows.
2. `division_equipment` sidecar join.
3. `designs` template metadata and `designs_variants` equipment metadata.
4. `oob_command` command tree.
5. `construction_orders` battle-plan orders.
6. Optional sub-hour movement/status deltas.

## Smallest Safe Implementation Slice

The first conflict-free code slice after the current `fuel_live` work should be
adapter tests and validators only:

1. Add collector constants for future heavy snapshot domains, but keep them out
   of `SUPPORTED_SNAPSHOT_DOMAINS`.
2. Extend collector-frame validation tests so `worker_poll` frames carrying
   `divisions`, `division_equipment`, or `oob_command` payloads are rejected.
3. Add synthetic snapshot tests proving claimed unsupported domains still reject
   before provenance.
4. Add private validation/fold helpers for `divisions` and unit-test both
   accepted and rejected claimed payloads without registering the domain.
5. Add an accepted-domain provenance test proving a rejected claimed domain is
   not listed as captured data.
6. Only after those helpers are strict, add a fold for `divisions` and then add
   the domain to `SUPPORTED_SNAPSHOT_DOMAINS`.

The first runtime DLL slice should not discover new offsets. It should emit a
diagnostic-only root/preflight report for division/OOB candidates:

- build/profile id;
- whether tag table and country database roots are available;
- count of located/validated division body candidates, with no schema payload;
- count of rejected candidates by validation reason;
- elapsed microseconds for preflight and warm validation;
- no raw pointers in persisted replay schema.

The first schema-writing DLL slice should be `divisions` only, hourly scheduled,
using already-validated preflight roots and omitting equipment, templates beyond
stable ids/names, OOB membership, and battle-plan orders.

## Live Proof Checklist

Before enabling each collector domain in normal recording:

- synthetic frame tests pass for accept/reject/provenance behavior;
- replay schema validation passes on the resulting snapshot;
- live proof records at least 12 consecutive hourly scheduled snapshots at speed
  five with zero skipped hours or explicit manifest gaps;
- per-frame monotonic sequence and capture boundary are present;
- payload count matches same-hour development oracle within the known live-vs-save
  gaps for that domain;
- warm validation cost is inside the scheduled snapshot budget;
- layout/profile mismatch omits the domain instead of emitting guessed fields.
