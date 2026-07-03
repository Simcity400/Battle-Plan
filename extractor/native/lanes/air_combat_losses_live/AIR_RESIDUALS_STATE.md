# Air Residuals State

Status: discovery-open synthesis for wave-2 air residuals. This file is not a
promotion document. It records what prior probes proved, what they rejected, and
which polling-observable alternatives remain worth a live fixture run on build
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Source Rows

`extractor/documents/MASTER_EXTRACTION_CONTRACT.md` currently keeps these rows
open or fixture-blocked:

- Sortie/combat lifecycle: `ROOT_DISCOVERY_OPEN`.
- Kill/cause attribution: `ROOT_DISCOVERY_OPEN`.
- Reinforcement reason: `ROOT_DISCOVERY_OPEN`.
- Ace identity/assignment: `FIXTURE_BLOCKED`.

The accepted production slice is still only domain 42 strength/equipment deltas:
losses, reinforcements, deletes, mission region, and equipment amount deltas.
Those rows are useful replay evidence, but they are not sortie starts/ends,
combat pairings, killer/cause attribution, ace participation, or rich
reinforcement reason.

## Rejection History

### Stable Direct Air Roots

Tried:

- `StrategicAirManager`, airbase-owner containers, and air-wing pools.
- Unknown-vtable objects directly referenced from those roots.
- Candidate target objects whose first `0x400` bytes contained direct air-wing
  object/body references.

Rejected:

- Six direct-reference candidates were found, but all were same-owner,
  same-region, pool-local wing collections.
- Opposed event-root candidates found: `0`.
- Proof rejected this exact route as `rejected_direct_opposed_event_root`.

What this proves:

- The bounded manager/container/pool route is not a full opposed air event root
  on the live 870ebf profile.

What remains unprobed:

- Tick-local records created and destroyed inside air-resolution functions.
- Region-local structs that do not directly store wing object/body pointers.
- Per-wing counters/timers that encode lifecycle state without direct opponent
  references.

### Air-Wing Body Attachments

Tried:

- Direct heap pointer fields from active or sampled `CAirWing` bodies.
- Target objects with module-local vtables outside known wing/pool/region
  vtables.
- Upgrade criterion required opposed owner refs and region/source-wing context.

Rejected:

- `115` body attachments were classified, but `0` opposed event-root candidates
  were found.
- Candidates with only region refs were not accepted as combat roots.
- Proof rejected the route as `rejected_body_attached_opposed_event_root`.

What remains unprobed:

- Non-pointer scalar counters inside `CAirWing` bodies.
- Pointers behind a wider chain justified by disassembly or a controlled
  before/during/after active-combat fixture.

### Land-Style Combat Side / Registry

Tried:

- Current-build `CCombatSide` vtable route and combat registry small-vector
  layout.
- Participant identity checked for air-wing bodies.

Rejected:

- `217` `CCombatSide` hits were scanned.
- `77` populated side vectors were all division participants.
- Air-wing side vectors: `0`.
- Combat registry hits: `42`; no air-wing participant path.
- Proof rejected the route as
  `rejected_no_ccombat_side_air_wing_participants`.

What this proves:

- The current-build land-style combat-side/registry route is not reused as a
  persistent air-combat event root in that fixture.

What remains unprobed:

- Tick-local/internal air-combat structs outside the land registry.
- Region-owned battle/attrition containers that are not `CCombatSide`.

### Stable Rooted Residual Matrix

Tried:

- Stable production-readable roots reachable from `StrategicAirManager`, active
  mission-region objects, air-base records, air-wing pools/bodies, first-hop
  rooted manager objects, indirect vectors, and POD-like record arrays.

Rejected:

- Roots scanned: `597`; one-hop sources: `192`; active mission regions: `20`;
  wing bodies: `96`; valid air-base records: `368`.
- Accepted vector headers inspected: `4553`.
- Matrix candidates classified: `1301`.
- Direct rooted opposed event candidates: `0`.
- False routes separated: `47` air-base basing collections, `453` POD
  owner/wing-id collisions, `16` unanchored record-stride artifacts.

What this proves:

- Stable rooted residual paths under the validated air manager/region/wing graph
  did not expose opposed participant state in that live air-war run.

What remains unprobed:

- Function-local air-resolution records.
- Fields that delta only when the fixture is controlled around exact combat
  ticks.
- Region-side stats not reachable as direct wing refs.

### Air-Resolution Helper RVAs

Tried:

- Static/Ghidra-backed and runtime Frida-backed narrowing of update wrappers,
  phase bodies, worker chunks, internal calls, deep calls, and second-level
  interior calls.

Rejected or narrowed:

- `0xC57A50`, `0xC57960`, `0xC57AA0`, `0xC57B40`, `0xC57BE0`,
  `0xC57C30`: TBB scheduler wrappers, not direct event hooks.
- `0x1950F80` and `0x1950A30`: `CAirRegionCombatData` serializer/dispatch-like
  endpoints; no live hits during the air-worker window.
- `0x1963700`: `CFEXAir` with `CNavalCombat`; carrier/naval sortie efficiency,
  not strategic-air residuals.
- `0xF46A10` / `0xF4E0E0`: single-wing state/category helper; incoming second
  argument is not consumed.
- `0xF6B7B0` / `0xF49E80`: single-mission scalar helper; pair-shaped runtime
  registers were stale-register false positives.
- `0xC44FA0`, `0xC448F0`, `0xE0CD40`, `0xE0CF00`, `0xF71050`: region/base
  table accessors or aggregate scalar writers. They do not construct or receive
  opposed participants, loss cause, ace entries, or tactical event records.
- `0xF51060` subpath through `0xF51A60` and the
  `0x1004BF0/0x1004C40/0x1004C50/0x1004C80/0x1004E70` helper family:
  subunit/equipment helper/reset/copy routines, not attribution or rich
  reinforcement reason.

What remains unprobed:

- Return-value or mid-function state below not-yet-rejected air worker helpers,
  if a fixture shows polling-visible state is insufficient.
- A hook is still the likely requirement for exact tick-local records, but that
  is outside the polling extractor architecture and cannot be claimed here.

### Ace Routes

Validated:

- Country-owned ace roster root:
  `CGameState+0x310/+0x31C -> CCountry`, `CCountry+0x12E4` ace count,
  `CCountry+0x12D8` ace pointer vector, ace entry vtable `module+0x27C7DD0`,
  ace id `+0x0C`, name/surname/callsign strings at `+0xF0/+0x110/+0x130`.
- Air/ace lane-local proof also identified assigned ace id candidates at
  `wing_body+0x2DC` / `wing_object+0x2EC`.

Rejected:

- Direct wing-body `CArmyLeader` / `CNavyLeader` attachments as flying-ace
  participation. Regular leaders appeared on owners with zero ace count and
  leader targets were shared across unrelated owners/wings.
- Direct refs from bounded air roots / first-hop air roots to pinned country ace
  roster entries: `0`.

Narrowed:

- Ace-management command path:
  command id `0x2FB5` in dispatch table `0x6F9EB0` reaches block `0x6FA8AC`,
  callsite `0x6FA8BA`, function `0xC52840`.
- `0xC52840.rcx` is already resolved as the selected-country strategic-air
  container via `0x6C2D80(GameState+0x690)`.
- `rdx = r14` is the remaining unvalidated command/event payload.
- Static payload proof says `rdx` is copied to `r15`, discriminator is
  `dword [r15+0x30]`, and fields read include `+0x28`, `+0x34`, `+0x38`,
  and `+0x44`.

What remains unprobed:

- A controlled UI fixture assigning and removing an ace from a named wing.
- Polling-visible before/after changes on the selected strategic-air container,
  selected wing assignment field, and owner country ace roster.
- Actual `rdx = r14` register payload interpretation. Polling cannot observe
  this; it requires a separately approved callsite register capture at
  `0x6FA8BA` during command `0x2FB5`.

## Polling-Observable Alternatives

### Per-Wing Counters/Timers

Most promising polling target:

- Validated `CAirWing` body rows keyed by owner tag-db-id and wing id.
- Known fields: wing id, plane count, mission mask, mission region, effective
  planes, equipment bank.
- Candidate residual fields: unknown u32 fields in `CAirWing[0x0..0x380]` that
  delta during live air-war ticks and co-change with loss/reinforcement deltas.

Ready probe:

- `extractor/memory_reader/air/air_rtti_polling_probe_870ebf.py`

The probe produces:

- RTTI/vtable rediscovery for air classes.
- Active-wing snapshots and region/owner plane counts.
- Unknown wing-body u32 field deltas.
- Loss/reinforcement delta examples from count/equipment-bank changes.
- Explicit candidate-only status. No offset is accepted without save/UI oracle
  cross-check and repeated fixture proof.

### Air-Region / Mission Objects

Most promising polling target:

- Rediscovered `CAirMission`, `CAirRegionCombatData`, `CStrategicRegion`,
  `CStrategicAir`, and related air class vtables.
- Live body counts and sample addresses under an active air-war fixture.

Acceptance threshold:

- A candidate region/mission object must be present during the active air-war
  fixture, must be linked to the known strategic region or known wing set, and
  must expose fields that change with the UI/save oracle. Merely rediscovering
  RTTI or seeing live objects is not enough.

### Save/UI Oracle Cross-Check

For the next live run, the orchestrator should provide a small fixture JSON with
at least:

```json
{
  "fixture_name": "1943_active_air_war_western_germany",
  "expected_regions": [123],
  "expected_wings": [
    {"owner_tag_db_id": 1, "wing_id": 1001, "mission_region": 123}
  ],
  "expected_region_owner_counts": [
    {"mission_region": 123, "owner_tag_db_id": 1, "min_plane_count": 1}
  ]
}
```

The exact region id and wing ids must come from the live UI/save oracle. The
template values above are placeholders and must not be reused as evidence.

## Reinforcement Reason Attribution

Replay-side inputs already available:

- Domain 30/32: air-wing identity, owner, mission, region, and loadout context.
- Domain 42: per-wing count/equipment amount deltas, including reinforcement-
  like positive deltas and deletion/loss-like negative deltas.
- Domain 46: air variant archetype and current ace roster/wing-assignment slice.
- Domain 11: production, stockpile, and related equipment rows as currently
  encoded.

What can be inferred:

- A wing gained aircraft.
- Which variant/equipment amount increased, if the loadout bank changed.
- Whether the country had matching production/stockpile availability in the same
  time window, if those domains are present in the replay.
- Whether a wing was deleted/recreated or moved/rebased only if the existing air
  wing lifecycle rows show a stable identity transition.

What cannot be exactly attributed from polling state alone:

- Whether a reinforcement came from fresh production, stockpile replacement,
  transfer/lend-lease/market delivery, airbase reassignment, priority change, or
  delayed replacement queue.
- Whether a positive delta is the replay-time result of multiple causes merged
  across an extraction gap.
- The exact in-game reason/cause label, because no root currently ties a
  reinforcement event to a production/transfer queue item at the wing level.

Attribution spec:

- Emit exact facts only: wing key, time/window, old/new count, equipment amount
  delta, variant ids, mission region/mask, and country-level production/stockpile
  context available at the same replay hour.
- Badged inference may say `possible_from_stockpile_or_production` only when a
  matching variant stockpile/production source exists in the same window.
- Do not emit `reinforced_from_production`, `reinforced_from_transfer`, or
  `attrition_replacement` as exact causes without a new root or event hook that
  joins the wing delta to the source transaction.

Conclusion:

- Reinforcement reason remains `ROOT_DISCOVERY_OPEN`. The existing domains are
  enough for replay-side explanation heuristics, not exact cause attribution.

## Current Open Root Classes

Still unprobed or insufficiently probed under current constraints:

- Per-wing sortie counters/timers inside `CAirWing` bodies beyond known fields.
- Region-local combat/attrition containers discovered by current-build RTTI that
  do not look like land/naval `CCombatSide`.
- `CAirMission` state fields that change only under a controlled mission/combat
  fixture.
- Strategic-air owner container fields around command `0x2FB5` before/after ace
  assignment and removal.
- Tick-local/function-local air-resolution records that polling cannot observe.

Do not promote any of these without live evidence. Do not reject any of them
without a proof showing the exact candidate class, fixture state, and failure
criterion.
