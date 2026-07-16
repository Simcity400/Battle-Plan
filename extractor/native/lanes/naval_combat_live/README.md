# naval_combat_live lane-local production stack

> Active lane note: this README is the lane-local orientation page. Historical
> plans, proof reports, blocker notes, and registration packets for this lane
> are archived under `../../../documents/archive/native_lanes/naval_combat_live/`.
> Current extraction completion status lives in
> `../../../documents/MASTER_EXTRACTION_CONTRACT.md`.

This lane owns compact domains from `extractor/documents/DOMAIN_REGISTRY.md`:

- `49 = naval_combat_lifecycle_live`
- `50 = naval_combat_members_live`

The shared collector and recorder integration slot is intentionally separate.
This lane may prove constants and compact behavior, but the orchestrator must
serially port validated constants into shared hot files after live validation.

## Files

- `src/domain49_50_naval_combat_live.rs`: compact encoder/decoder/gate for
  lifecycle and member rows.
- `src/extractor.rs`: lane-local extraction boundary for validated
  `CNavalCombat -> CNavalCombatant -> CFEXGroup -> CFEXMember` rows plus the
  current `CFEXMember -> CShip -> CTaskForce/CFleet` join.
- `src/bin/naval_combat_lane_proof.rs`: proof harness for synthetic and
  `--fixture-json` active-fixture compact/replay verification.
- `tools/active_fixture_proof.py`: bounded live harness that waits for a valid
  fixture, promotes the validated `front+0xA4` location to compact
  `location_id` (joining `region_id` via `map/strategicregions`), enriches
  CShip task-force/fleet data, and invokes the Rust proof.
- `CHAIN_REVERSAL_870EBF.md`: byte-level chain reversal notes for build 870ebf
  (historical; its side+0x2A0 member walk was REJECTED on live D04).
- `CHAIN_CRACKED_870EBF_D04.md`: the validated CFEXGroup member chain.
- `LIVE_VALIDATION_RUNSHEET.md`: commands and success/failure criteria for the
  live naval battle validation.

## 870ebf Provenance

Body/side/member/CShip constants originate from
`extractor/native/runs/naval_combat_fixture_20260703_biscay_v2`. The member
walk and location promotion are pinned to the D04 1944-02-11 paused fixture
with a same-tick save oracle:
`extractor/native/runs/nav49_d04/` (chain reversal probes) and
`extractor/native/runs/nav49_50_port_validation/` (save-oracle cross-check:
4/4 combat ids, 4/4 locations, 8/8 sides' ship ids, member state/slot
multisets). The fixture build SHA is
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Current Layout

- `CNavalCombat` vtable: `module+0x29C6F18`.
- `CNavalCombatant` vtable: `module+0x29CD298`.
- `CFEXMember` vtable: `module+0x2A0A228`.
- `CFEXGroup` vtable: `module+0x2A42D18` (RTTI `.?AVCFEXGroup@@`).
- `CShip` vtable: `module+0x2941748`.
- `CFleet` vtable: `module+0x294BF28`.
- `CTaskForce` vtable: `module+0x294C480`.
- All pinned vtable RVAs are re-verified by RTTI class name at attach; any
  mismatch fails closed (`vtable_rtti_mismatch`).
- Body handle: `body+0x18` packed `{type=0x1269, combat_id}` (combat_id matches
  the save's `naval_combat.id.id`).
- Body sides: `body+0x28/+0x30`.
- Body front pointer: `body+0x38` -> `CProvince` (vtable `module+0x295AF68`,
  RTTI `.?AVCProvince@@`, vtable-checked before the read);
  `CProvince+0xA4` u32 = province id = battle location
  (PROMOTED; validated 4/4 vs the D04 save).
- Side parent back-ref: `side+0x18 -> CNavalCombat`.
- Member walk (AUTHORITATIVE): heap-scan `CFEXGroup` vtable hits and bucket
  each group by its owning-side back-ref `group+0x38`. The old
  `side+0x2A0/+0x2A8/+0x2B0` inline group vector is REJECTED on this build.
- Group layout: `group+0x08` member ptr array, `group+0x10` u32 capacity,
  `group+0x14` u32 count (count <= capacity; empty groups valid),
  `group+0x20` next group (circular list spanning BOTH sides).
- Advisory forward cross-check: `side+0x20` child may hold CFEXGroup pointers;
  every forward-walked group must already be in the back-ref buckets. The
  child is absent for some live battles, so forward reachability is optional.
- Member handle: `CFEXMember+0x08`; high u32 joins to `CShip+0x08` high u32
  instance id for ship members; a packed handle of 0 is a convoy member
  (matches save `convoy={...}` rows).
- Member fields validated vs the D04 save: `+0xE0` state (save `state`),
  `+0x120` slot (save `unique_id`).

`CNavalCombatSide` has no own RTTI hit on 870ebf. The old registry vtable
constant is not part of this lane's adopted 870ebf route. The side
`+0x68/+0x74` country vector interpretation is UNVALIDATED on 870ebf and is
reported as a diagnostic candidate only.

## Fail-Closed Rules

- Whole-stream certification requires sequence origin 1, exact non-wrapping
  successors, consecutive game-hour frames, and a physical final canonical
  `FRAME_END`. Every certified frame carries exactly one PERF and one DATA
  record for each of d49 and d50, in canonical d49-before-d50 order. PERF
  changed-row/unit authority must equal the decoded DATA cardinality, each
  owned PERF status word must exactly equal its DATA status word, and
  `FRAME_END` emitted/scheduled-domain counts, reserved word, known status
  mask, and full global failure mask are independently checked. Each active
  d50 membership row must bind the persistent d49 `(combat_id, generation)`
  lifecycle state after that hour's d49 delta is applied; requiring the parent
  row to re-emit in the same hour would falsely reject legitimate member-only
  delta updates. Duplicate parents, member positions, or live ship identities
  are rejected. A d49 parent tombstone must be accompanied by complete
  same-frame d50 child teardown; lifecycle and member tombstones permanently
  retire that generation/key, so duplicate tombstones and same-generation
  resurrection are rejected rather than creating an impossible later cleanup
  state. Payloads for domains other than d49/d50 remain opaque after
  their canonical H4CR envelope and frame placement are validated.
- `front+0xA4` is PROMOTED: live fixtures must supply `location_id` and the
  static `region_id` join for every active combat row; an absent/zero location or an
  unmapped province is a hard error. A lifecycle row missing either promoted
  field sets `STATUS_LOCATION_CANDIDATE_UNVALIDATED`; d49 certification treats
  that bit and any row missing either promoted field as lane-fatal.
- Every non-convoy d50 row carries a nonzero CShip instance identity and a
  validated CShip join. A single good join cannot mask another unresolved ship.
  Convoys preserve the proven zero packed-handle contract: no ship identity,
  CShip validation, OOB linkage, or CShip-derived hull telemetry is permitted.
- Null member array slots are skipped and recorded, never emitted; a non-null
  slot that is not a `CFEXMember` is a hard error.
- A forward-walked CFEXGroup missing from the back-ref buckets is a hard error
  (the authoritative scan dropped live data).
- Fixture JSON accepted by `naval_combat_lane_proof --fixture-json` must carry
  `source.root.sha256_ok == true` and `source.root.sha256` equal to the pinned
  870ebf SHA. Missing or mismatched build provenance hard-fails.
- With `--require-cship-join`, any unresolved non-convoy `CFEXMember -> CShip`
  join makes `validated_combat_count == 0`, suppresses normal `bridge.txt` and
  `member_bridge.jsonl`, and exits nonzero. Rejected diagnostics may use only
  `bridge.REJECTED.txt` and `member_bridge.REJECTED.jsonl`.
- Duplicate `ship_instance_id` within one combat (any side/group) is a hard
  error in both the probe and the Rust frame builder (stale-group guard).
- A truncated body/group vtable scan is terminal (`scan_truncated`) — a capped
  hit list could silently drop live battles/groups.

## Known Residual Risks (2026-07-03 adversarial review)

- A stale/freed CFEXGroup whose `+0x38` still points at a live side AND whose
  ships are disjoint from every live group would pass the duplicate-ship guard.
  Live D04 evidence showed zero such groups (all 17 non-battle groups pointed
  at non-naval objects), but production integration should add a stable-frame
  double-read before trusting group membership on an unpaused game.
- Member reads are not atomic: A-B-A owner changes or member-array reallocation
  between header and array reads are possible on an UNPAUSED game. The paused
  fixture gate is immune; production integration needs stable-hour/double-read
  guards (same pattern as the other live lanes).
- The `region_id` join parses `map/strategicregions` from the module_path
  parent (vanilla install). A total-conversion mod overriding the map would
  make the join wrong-but-present; the production loader must key the region
  map on the active mod set before this lane ships modded captures.

### Orchestrator second-review findings (2026-07-03) — MUST fix in the
### production `src/extractor.rs` walk during shared collector integration

These are fail-open paths that the paused-fixture proof does not exercise (the
D04 gate matched the save exactly: 117/117 members, 8/8 sides, exact state/slot
multisets), but the production DLL walk on an UNPAUSED game must fail closed:

1. `bucket_groups_by_owning_side` silently `continue`s when the `group+0x38`
   read returns None (a torn read on an unpaused game), and `validate_body`
   only rejects `forward_only` groups, not a back-ref group dropped by a failed
   `+0x38` read. Production walk: a failed `+0x38` read on any CFEXGroup hit must
   be a terminal `scan_truncated`-style error, not a silent drop.
2. A counted CFEXMember whose packed handle reads as `0` is treated as a convoy
   and bypasses CShip validation. On a paused game a counted slot is a live
   member (convoy counts matched the save), but the production walk must
   distinguish an unreadable/torn member read from a genuine zero handle before
   emitting a convoy row.
3. Ancillary member fields fail open: an unreadable `state` (`+0xE0`) coerces to
   `0` in the probe and a missing `slot_index` coerces to `0` in
   `active_fixture_proof.py`. Production must badge an unreadable ancillary field
   rather than emit a plausible `0`.

(Findings 4/5 above — stale-group liveness and mod-aware region map — are the
same class and already listed.) The shared-collector integration restructures
this walk for per-frame cost anyway (a full CFEXGroup vtable heap scan is ~4.2s;
it needs the domain-53 discovery-cache treatment or active-battle gating), so
these fail-closed guards land together with that work.

## Proof Commands

```powershell
Set-Location 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\mod\battle-plan\extractor\native'
cargo test -p naval_combat_live_lane
cargo test --workspace
```

Next live validation must use `LIVE_VALIDATION_RUNSHEET.md`. The domain is not
validated until that live run succeeds and the orchestrator records the manual
location candidate comparison.
