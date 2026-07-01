# army_oob_orders_battleplans_live lane

Lane scope: land army/army-group command hierarchy, commander assignment,
formation membership, selected army order roots, battle-plan target slices, and
OOB/order events needed for replay. Division scalar and movement fields remain
owned by the division lane; this lane consumes division identity roots only as a
join/source boundary.

Registry binding: depth work uses `army_battleplans_depth_live` (`domain_id =
54`). Existing base domains `army_orders_live` (`domain_id = 27`) and
`oob_command_live` (`domain_id = 28`) stay unchanged. Older lane-local
`direct_land_oob` domain-44 slot material is canceled by the compact domain
registry and is not a production integration path.

## Current production slice

The current production-shaped slice is the compact DLL/Rust path for active
armies reachable from live divisions:

- compact domain `army_orders` (`domain_id = 27`);
- compact domain `oob_command` (`domain_id = 28`);
- one DLL collector/coordinator, compact binary stream, Rust recorder decode,
  and `.battleplan` packing;
- no Python/save hot-loop dependency;
- division roots are used only to derive active `CArmy` parents and OOB joins.

Fresh focused live proof:

- run: `replay_native/runs/army_oob_orders_battleplans_live_870ebf_20260701_v1`;
- pid: `45880`;
- duration: 20 seconds, `sample_ms = 0`;
- scheduled domains: `army_orders`, `oob_command`;
- advanced: 24 game hours, 25 unique frame hours;
- skipped hours: 0;
- compact stream: ordered and decodable;
- `.battleplan`: `native_session.battleplan`, independently accepted by
  `pack_replay verify`;
- failures: no source-unavailable, partial-data, bridge-unavailable, or worker
  panic flags.

Direct-root/theatre proof:

- run: `replay_native/runs/army_oob_orders_battleplans_live_direct_oob_870ebf_20260701_v1`;
- pid: `45880`;
- proof module: `direct_oob_probe.py`;
- gate: `direct_oob_proof_gate.py`;
- result: direct vtable root scan found 660 `CArmy`, 470 army-group,
  97 theatre, and 3051 division roots with zero rejected vtable hits;
- active-army comparison: 652 armies reached through live `CDivision+0xB0`
  parents, leaving 8 direct empty/inactive army roots that the active-army
  production slice cannot see;
- order mix on direct armies: 630 order rows, including 166 exact type-5 target
  rows and 464 non-type-5 rows (`type=2`: 423, `type=3`: 41);
- production position: validated current-build discovery evidence and a
  cold-preflight/cache-refresh candidate, not yet a hot-loop DLL production root.

## Validated current-build roots and offsets

Current executable/profile in the existing proof family:
`870EBF1F615262FACCA48A3B947775C41F0A5338F3068C40958209F5C41C65AD`.

Production root dependency, as implemented in `replay_native/collector/src/lib.rs`:

- validated collector game-state pointer, then `CGameState+0x310` country table;
- country count bounded by `MAX_CGAMESTATE_COUNTRIES = 2048`;
- `CCountry` vtable `MODULE+0x27D0E90`;
- country division-wrapper vector:
  - data `CCountry+0x290`;
  - capacity `CCountry+0x298`;
  - count `CCountry+0x29C`;
- division wrapper vtable `MODULE+0x2943AD0`;
- wrapper-to-body delta `+0x10`;
- `CDivision` body vtable `MODULE+0x2943CB0`;
- division type accepted as `4713` or alternate type `51`;
- division id read from the validated body and used as the join key;
- `CDivision+0xB0` supplies the parent `CArmy*` for active-army discovery.

Army order roots:

- `CArmy` vtable `MODULE+0x293BC60`;
- army order vector at `CArmy+0x98`;
- order instance vtable `MODULE+0x296FAE8`;
- order type at `order+0x30`;
- army back-reference at `order+0x38`;
- type-5 target-state vector:
  - begin `order+0xE0`;
  - size `order+0xE8`;
- max 64 orders per army;
- max 5000 target states, with compact inline target rows capped by the encoder;
- max state id `65535`;
- target geometry is exact only when `target_status = exact`. Unsupported,
  overflow, and validation-failed rows are explicitly badged.

Land OOB command roots:

- active army roots are derived from the same `CDivision+0xB0` parents;
- army-group vtable `MODULE+0x293BCF8`;
- army group parent pointer from `CArmy+0x1B8`;
- army member engine count from `CArmy+0x5C`;
- leader pointer from `CArmy+0x88`;
- leader id from leader `+0x0C`;
- leader skill/trait fields are emitted only when validated by the collector;
- owner is derived from member-division owner consensus and emitted as tag-db id.

Direct land-OOB candidate roots:

- `CArmy` vtable `MODULE+0x293BC60`;
- army-group vtable `MODULE+0x293BCF8`;
- theatre vtable `MODULE+0x295EE18`;
- `CDivision` body vtable `MODULE+0x2943CB0`;
- army and army-group handles use type `53`;
- theatre handles use type `67`;
- direct root ids were accepted only after vtable, type, id, alignment, and
  sanity-bound validation;
- current proof rejected no candidate hits for army, army group, theatre, or
  division roots.

The older Python readers (`memory_reader/construction_orders_live.py` and
`memory_reader/oob_command_live.py`) remain useful discovery lineage for the
same shapes, but their hardcoded `16022851` offsets are not production-current.
The live proof above used the Rust collector's current `870EBF` profile.

## Compact records

`army_orders` payload:

- 40-byte domain header:
  - elapsed wall ms;
  - row count;
  - changed rows;
  - status flags;
  - division root rows;
  - observed army rows;
  - observed order rows;
  - target-state child rows;
  - deleted rows.
- 48-byte order rows keyed by `(army_id, generation, slot_index)`:
  - deleted flag;
  - order type;
  - optional target-state child range;
  - optional declared target-state count;
  - target status code.
- 8-byte target-state child rows:
  - state id;
  - reserved zero.

`oob_command` payload:

- 48-byte domain header:
  - elapsed wall ms;
  - row count;
  - changed rows;
  - status flags;
  - division root rows;
  - observed army rows;
  - observed army-group rows;
  - member child rows;
  - deleted rows;
  - observed fleet rows;
  - observed task-force rows.
- 80-byte OOB rows keyed by `(kind, object_id)`:
  - kind `1 = army`, `2 = army_group`, `3 = fleet`, `4 = task_force`;
  - deleted flag;
  - optional parent id;
  - optional owner tag-db id;
  - optional leader id and skill;
  - optional member child range;
  - engine member count and missing member count when measured;
  - optional location province for task-force rows;
  - status codes for parent/member/owner/leader/leader traits.
- 8-byte member child rows:
  - member object id;
  - reserved zero.

`compact_schema.rs` is the lane-local decoder/gate mirror for these two domains.
It intentionally has no dependency on shared recorder code so schema review can
happen without editing shared entrypoints.

`direct_oob_compact.rs` is the lane-local proposed additive compact payload for
direct land-OOB roots and theatres. It is not wired to the recorder. The current
proposal uses:

- proposed compact domain `direct_land_oob` (`domain_id = 44`);
- 32-byte payload header;
- 64-byte rows keyed by `(kind, object_id)`;
- kind `1 = army`, `2 = army_group`, `5 = theatre`;
- 8-byte member child rows;
- status `5 = cold_preflight_only` to prevent mistaking a slow scan proof for a
  hot-loop production source.

`direct_land_oob_slot_contract.json` and `direct_land_oob_slot_gate.py` are the
lane-local slot-readiness gate. They bind the existing direct-root proof to the
compact constants above and require a bounded preflight/cache-refresh collector
plan, no hot-loop heap sweep, additive recorder decode, and post-slot replay
gates. `DIRECT_LAND_OOB_SLOT_PLAN.md` contains the exact future patch plan.

`battleplan_fixture_gate.py` is the acceptance/rejection gate for remaining
battleplan gaps. Exact rows must carry validated offsets and concrete field
values. Rejected candidates must carry candidate, reason, and evidence list, and
only pass with `--allow-rejection`.
`battleplan_fixture_gate_tests.py` covers exact acceptance, rejection gating,
passive-candidate rejection, and cached plan-value rejection without live
offsets.
`battleplan_integration_guard.py` blocks bad future integration proposals:
existing `army_orders`/`oob_command` row contracts must stay unchanged, geometry
and execution must be additive, and passive scans or save-only/cache-only fields
cannot be used as integration evidence.
`battleplan_fixture_observer.py` is the executable before/after harness for a
future redraw/toggle fixture. It converts changed observations into exact rows
and unchanged observations into exact rejection rows, while rejecting passive
scan sources.

`army_battleplans_depth_collector.rs`,
`army_battleplans_depth_compact.rs`, `army_battleplans_depth_module.rs`, and
`army_battleplans_depth_from_stream.py` are the lane-local domain-54 mini stack
for the validated subset that does not need a redraw/toggle fixture:

- exact type-5 target-state rows are emitted as exact depth rows;
- non-type-5 rows are emitted with geometry fixture-required status;
- execution state is emitted as fixture-required until a live toggle fixture
  validates or rejects an offset;
- non-exact type-5 rows are omitted fail-closed rather than guessed.
- the production module boundary accepts live order observations and emits a
  domain-54 payload directly to a compact sink; it no longer depends on a
  domain-27 proof stream conversion for the future DLL hook.

Proof run:
`replay_native/runs/army_battleplans_depth_live_870ebf_20260701_v1`.

Replay/package proof run:
`replay_native/runs/army_battleplans_depth_live_module_replay_870ebf_20260701_v2`.
It wraps the exact domain-54 compact bytes in a `.battleplan`, runs
`pack_replay verify`, and checks the replay manifest entry hash against the
source compact stream.

`ARMY_BATTLEPLANS_DEPTH_REGISTRATION_PLAN.md` is the exact registration-only
hook plan for the collector registry/coordinator and Rust recorder. It is not
applied while another lane owns the shared integration slot.

Current candidate probe:
`replay_native/runs/army_battleplans_depth_live_candidate_probe_870ebf_20260701_v2`.
`battleplan_candidate_probe.py` validated 350 current-build order objects,
passed the type-5 `order+0xE0/+0xE8` positive control, rejected `order+0x150`
as `is_executing: u8`, and narrowed `order+0x70/+0x78` plus
`order+0xC0/+0xC8` to redraw-fixture-required geometry candidates.
`battleplan_candidate_probe_gate.py` locks that proof shape so failed positive
controls or passive vector-shaped rows cannot be cited as production roots.

## Remaining backlog

Not completed by this lane-local slice:

- DLL-native hot-loop or bounded preflight integration of direct full land OOB
  roots for empty armies not reachable through a live division parent;
- DLL-native hot-loop or bounded preflight integration of theatres and land
  command roots not emitted by the active-army compact slice;
- non-type-5 front/fallback/offensive-line geometry beyond exact type-5 target
  state sets. The current fixture gate requires exact live geometry evidence or
  exact rejection evidence before promotion;
- battleplan execution/activation state and plan-value/power runtime caches. The
  current fixture gate requires exact live execution-state evidence or exact
  rejection evidence before promotion. `order+0x150` as `is_executing: u8` is
  rejected for current build `870ebf` and should not be retried;
- UI-level semantic reconstruction of front lines from order geometry;
- broad integration into shared entrypoints beyond the existing compact
  domains.

## Integration patch plan

Do not edit shared collector/recorder entrypoints until the orchestrator grants
an integration slot. The merge-ready path is:

1. Keep `domain_id = 27` and `domain_id = 28` stable.
2. Register `domain_id = 54` as schedulable
   `army_battleplans_depth_live` and call the lane-owned module in
   `army_battleplans_depth_module.rs`.
3. Promote any new battleplan row fields as additive flags/reserved-field
   expansions, never by changing existing row widths without a version bump.
4. For direct land-OOB roots, add an isolated collector module that reuses the
   validated vtable/type/id checks from `direct_oob_probe.py`, refreshes roots
   in a bounded preflight/cache-refresh stage, and emits additive
   `direct_land_oob` rows using the layout in `direct_oob_compact.rs`.
5. Add theatre rows as `kind = 5` only after the shared recorder slot is granted;
   until then, keep the proof as lane-local evidence and do not mutate
   `oob_command` row widths.
6. Keep non-type-5 geometry and execution state behind
   `battleplan_fixture_gate.py`. Promotion requires a fixture JSON with DLL-only
   compact capture, `.battleplan` verification, zero skipped hours, no source or
   worker failures, and at least one exact or exactly rejected row for each gap.
   A no-fixture route is acceptable only if it supplies exactly rejected
   candidates and passes `battleplan_fixture_gate.py --allow-rejection`.
7. Preserve the existing join boundary with division lane: this lane may consume
   `(owner_tag_db_id, div_id, div_type/body identity, CArmy parent)` but must not
   emit division movement/scalar state.
8. Gate every widened proof with:
   - compact stream ordered/decodable;
   - one data record per scheduled domain per frame hour;
   - `.battleplan` verification;
   - zero skipped game hours;
   - no source-unavailable, partial-data, bridge-unavailable, or worker-panic
     flags.
