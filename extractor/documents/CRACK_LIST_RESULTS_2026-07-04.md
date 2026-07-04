# Final crack-list results (2026-07-04, exe 870ebf, D04 1944.2.11.18 paused)

Results of the RE crack list (`CRACK_LIST_FINAL_2026-07-04.md`, groups A/B/C). Run
by the crack agent; the headline/overturning cracks INDEPENDENTLY verified by the
orchestrator against the melt + the live air-combat fixture. Full per-item evidence:
gitignored `runs/cracklist_<item>/EVIDENCE.md` + `runs/cracklist_MASTER_TABLE.md`.

**6 new CRACKs (2 overturn prior verdicts), 1 STORED (was derived-candidate),
B3 resolved, C2 corrected, 5 confirmed-derived.** Orchestrator-verified in bold.

## CRACKED (no user) — orchestrator-verified

| item | offset chain | verification |
|---|---|---|
| **C3 game speed** (overturns DEFER) | `speed_index = i32[CGameState+0x4BC]` (scheduler slices GAME_SPEED_SECONDS by it; tick_progress f32 @+0x4B8) | **VERIFIED: live = 1 == melt top-level `speed=1`.** No user fixture needed. |
| **C1 live air-combat stats** | `CLoopHistory` stat-23 yearly ring (vt 0x29BBE58), 5-slot stride 0x2D0, current=slot 2; 89-field i64×1e5 record; our_fighters@f3, our_bombers@f10, enemy_fighters@f44, enemy_bombers@f51, enemy_cas@f58, enemy_bombers_disrupted@f80 | **VERIFIED: record @0x13514EE6120 reads EXACTLY the ITA panel {104,3,44,5,9,21872}** (user screenshot fixture). Also = melt stat-23 ring ×1e5, 89/89. |
| **A2 resource flow** (overturns DERIVED) | `CCountryResources = u64[CCountry+0x11F8]`; pools inline at +0x28 produced / +0x188 imported / +0x238 used / +0x450 to_export / +0x500 base_export / +0x5B0 exported; resource i = `i64[pool+0x38+i*0x10]/1e5`, order [oil,alu,rub,tun,steel,chr,coal] | **VERIFIED: GER produced [134,264,93,128,1035,81,853] + SOV [86,164,3,56,572,433,130] both EXACT vs melt.** 95/95 countries. |
| **B4 d53 base relations** | `mgr=u64[CCountry+0xF88]; table=u64[mgr+0x08]; rel=u64[table+tgt*8]`; cached_sum i32 @rel+0x300, border_friction i32 @rel+0x308 /1e5 | agent: cached_sum 10354/10354, border 44382/44382 vs melt. UNBLOCKS the d53 relation data. |
| **B1 land-combat tactic** | per side `tactic_id = i32[u64[CCombatSide+0x110]+0x98]` → CCombatTactic (vt 0x27D0058), token@+0x10, name@+0x30 | agent: attacker 50/50 + defender 50/50 vs melt. (Completes E2's tactic PARTIAL.) |
| **B2 breakthrough** | join CProject id@+0x0C (21511/21511 global); `phase_progress = i32[u64[CProject+0x28]+0x60]/1e5` | agent: 58/58 nonzero exact vs melt. |
| **A4 offmap factory** — STORED (DERIVED refuted) | serialized as country `buildings.<type>.level`; live container = country construction manager (CBuilding vt 0x2973B88); root a deeper-RE residual | agent: oracle built; STORED not derived. |

## Resolved / corrected
- **B3 active-war casualty**: the live running accumulator (row +0x40/+0x48) IS
  authoritative — no separate serialized-snapshot field exists (the melt value is a
  serialize-time copy). Frozen GER-LUX 1110/948 exact. E1 already wires this with the
  RUNNING_ACCUMULATOR badge.
- **C2 per-entity killer**: brief offsets were STALE. Corrected: CSunkShipInfo (vt
  0x2941608) owner/victim db_id @+0x48, attacker/killer db_id @+0x78, sink date @+0x58,
  ship-type @+0x80 (NOT +0x0C/+0x10/+0x14). 2 of 3 slots labeled; full 3rd-slot label +
  arg-labeling needs a caller/writer trace (no-user, residual).

## CONFIRMED-DERIVED (replay recomputes — Codex-gated)
A1 province supply, A3 attrition reason, A5 surrender progress, A6 reinforcement
reason, A7 land width/terrain/phase/planning/direction. Genuine absence of stored
state; reconstructed at replay from already-cracked inputs.

## Impact on the user-fixture floor (it SHRANK)
- **Game speed** — CRACKED (C3), no longer needs the B0 tap. REMOVED from the user list.
- **Live air-combat stats** — CRACKED (C1) via the user's live fixture (now DONE) +
  the melt stat-23 ring. REMOVED.
- **d53 relations** — CRACKED (B4): the relation/opinion data can now emit (further
  unblocks the diplomacy category alongside E7's decouple).

**Remaining genuine user-fixture floor (memory):** hub motorization level, focus-paused
bit, convoy/port PRESSURE, MP chat/hotjoin — plus the per-entity killer 3rd-slot label
(a no-user writer-trace residual, not strictly user-blocked). Everything else in memory
is cracked, stored (RE residual), or genuinely derived.
