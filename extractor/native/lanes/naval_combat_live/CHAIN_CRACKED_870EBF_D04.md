# Naval combat 49/50 — full chain CRACKED on live 870ebf (D04 1944 fixture, 2026-07-03)

Fixture: live `hoi4.exe` build `870ebf...`, save `D04_1944_02_11_18.hoi4` loaded and PAUSED,
with 4 active `naval_combat` blocks (locations 6434, 2804, 1032, 8819). The same-tick melted save is the oracle.

Evidence dir: `extractor/native/runs/nav49_d04/` (probes: `rtti_vtable_rediscover`,
`side_layout_diag`, `handle_hunt`, `member_ref_hunt`, `group_chain`, `group_to_side`,
`forward_chain`). All read-only, SHA-gated.

## What was wrong before

The prior candidate (`side+0x2A0/+0x2A8/+0x2B0` "member-group vector", constant
`side_group_begin_off: 0x2A0` in `src/extractor.rs`) is REJECTED on this build/fixture: it
resolved an allocation whose elements were not CFEXMember (the live `active_fixture_proof`
run reported `member_count=308` with every element `bad_member_vtable`,
`validated_combat_count=0`). The real member container is a **CFEXGroup**, reached through a
side child object, not an inline side vector at a fixed offset.

## Validated chain (13/13 ships + 4/4 locations matched the save oracle)

RTTI-rediscovered current vtables (RVAs off module base):
- `CNavalCombat`   body vtable `0x29C6F18`  (unchanged from CHAIN_REVERSAL_870EBF)
- `CNavalCombatant` side vtable `0x29CD298`
- `CFEXGroup`      member-group vtable — resolve by RTTI name `.?AVCFEXGroup@@` (live rediscover;
  add it to the fixture-probe RVA refresh like the others; do not hardcode a stale RVA)
- `CFEXMember`     member vtable `0x2A0A228` (unchanged)
- `CShip`          vtable `0x2941748` (unchanged)

Body (CNavalCombat):
- `body+0x28` -> attacker CNavalCombatant side
- `body+0x30` -> defender CNavalCombatant side
- `body+0x38` -> front object; **`front+0xA4` (u32) = battle location province** —
  **NOW VALIDATED**: live front+0xA4 == save `naval_combat.location` for ALL 4 battles
  (6434, 2804, 1032, 8819). Promote the previously `STATUS_LOCATION_CANDIDATE_UNVALIDATED`
  field to a validated compact location and clear the unvalidated status.

Side -> member groups (the corrected member chain):
- The battle's CFEXGroups form a **linked list**: each `CFEXGroup+0x20` -> next CFEXGroup.
- Reach the list from a side child: `side+0x20` (also mirrored at `side+0x68`, `side+0xf8`)
  -> child object -> a CFEXGroup pointer; then walk `+0x20` to enumerate all groups.
- Authoritative side assignment is the group's own back-ref: **`CFEXGroup+0x38` -> owning
  CNavalCombatant side**. Assign each group's members to the side its `+0x38` points at
  (the list reached from one side spans BOTH sides — do not assume all reached groups belong
  to that side).

CFEXGroup layout (member allocation):
- `CFEXGroup+0x08` -> member pointer array begin (contiguous CFEXMember*, 8-byte stride)
- `CFEXGroup+0x10` (u32) = capacity; `CFEXGroup+0x14` (u32) = **member count**
- Array elements are `CFEXMember*` (verify each vtable == CFEXMember rva; a leading `0`/null
  slot occurs — the defender group showed count=9 with one `0` id = 8 real ships, matching the
  save's 8 defender ships + 1 non-ship member).

Member -> ship join (UNCHANGED, re-confirmed):
- `CFEXMember+0x08` = `(ship_instance_id << 32) | 51`; high u32 = CShip instance id.
- Join to `CShip` (vtable `0x2941748`) whose `CShip+0x08` high-u32 == that id.

## Cross-check (battle at location 6434)

Save: attacker ships [4431,4438,4449,4420,4458] (5), defender ships
[3064,3099,3114,3051,3163,3190,3208,3970] (8).
Live via chain: attacker CFEXGroup (`+0x38`->attacker side) count=5 ids exactly those 5;
defender CFEXGroup (`+0x38`->defender side) count=9 ids = [0,3064,3099,3114,3051,3163,3190,3208,3970]
= the 8 defender ships + one null. `expected_missing: []`.

## Remaining work to close rows 49 + 50 (code, serial-gated)

1. DONE 2026-07-03: `src/extractor.rs` + the fixture probe now use the CFEXGroup chain.
   The AUTHORITATIVE walk is the CFEXGroup vtable heap scan bucketed by the `+0x38`
   owning-side back-ref (the `side+0x20` forward child route proved unreliable live —
   the child is absent/unreadable for some battles — and is kept only as an advisory
   subset cross-check: any forward-walked group missing from the back-ref buckets is a
   hard error). CFEXGroup vtable 0x2A42D18 pinned + ALL pinned vtable RVAs RTTI-name
   verified at attach (fail closed, `vtable_rtti_mismatch`). Null member slots are
   skipped/recorded; a CFEXMember with packed handle 0 is a CONVOY member (matches the
   save's `convoy={...}` rows; validated on the 2804 12-convoy defender).
2. DONE 2026-07-03: `front+0xA4` promoted to compact `location_id` + static
   `region_id` join (map/strategicregions); `STATUS_LOCATION_CANDIDATE_UNVALIDATED`
   now only fires as the fail-closed missing-field signal.
3. DONE 2026-07-03: live gate PASSED on the paused D04 fixture
   (`runs/nav49_50_port_validation/live_gate/`): `status=passed`,
   `validated_combat_count=4`, `member_bridge_row_count=117` (== save total),
   `cship_unresolved_member_count=0` (101/101 joins), Rust proof `passed`,
   4/4 locations+regions emitted, `.battleplan` byte-verified. Extended oracle
   cross-check: live
   combat ids == save `naval_combat.id.id` 4/4; ship-id sets matched on 8/8 sides;
   member `+0xE0` state and `+0x120` slot multisets matched the save's
   `state`/`unique_id` on all sides.
4. OPEN: serial-integrate the compact 49/50 rows into the shared collector/recorder
   (one packet, gated) — orchestrator-owned. Rows 49/50 moved to
   SHARED_INTEGRATION_PENDING. Lifecycle-end (teardown) and member join/leave/sinking
   fixtures plus the speed-five proof also remain.
