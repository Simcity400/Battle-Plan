# Final crack-sweep results (2026-07-04, exe 870ebf, D04 1944.2.11.18 paused)

Results of the comprehensive no-user sweep (`FINAL_CRACK_SWEEP_2026-07-04.md`,
targets T1–T6). Run by a Fable discovery agent, headline crack (capitulation)
INDEPENDENTLY verified by the orchestrator against the melt; the rest are the
agent's Codex-gated exact-match evidence. Per-target evidence:
`runs/finalsweep_<target>/EVIDENCE.md`.

**This sweep closes the last of the no-user memory gaps.** After it, the
irreducible remainder is: the 4 pre-existing user fixtures, the 10s speed-tap
(B0), live-air-combat stats (need live combat), the per-entity combat killer
label (1 event), and a couple of bounded no-user RE joins.

## T1 — Capitulation / war — CRACKED (capitulation orchestrator-verified)

Chain: `D = u64[CCountry+0xF88]`, then off `D`:
- **`capitulated` u8 @ D+0x2E0** — **orchestrator-verified: live read gives EXACTLY
  the melt's 15 capitulated countries** (BEL FRA GDC HOL KHM LEB LUX NOR PHI POL PRC
  SHX SIA SYR YUG), 0 live-only / 0 melt-only. (France's `capitulated_date` reads
  1940.6.18 — historically exact.)
- `capitulated_date` CGameDate i32 @ D+0x2F0; `last_surrender_date` @ D+0x2B8;
  `faction_join_date` @ D+0x2D0; `legitimacy` i32×1e5 @ D+0x1B0 (8/8);
  `exile_army_leaders` i32 @ D+0x1B8 — all 0 mismatch.
- **War pool** (D53): tags @+0x00/+0x04, date-vt @+0x08, casualties @+0x40/+0x48,
  echo @+0x68/+0x6C — 958/958 melt pairs live, only_melt=0; frozen-war casualties
  byte-exact. Active-war casualty magnitude is a running-accumulator (Codex: offset
  valid, not byte-equal to the serialized snapshot — expected).
- `surrender_progress` = **DERIVED** (draw-time); `peace_conference` = **absent**
  (melt block empty on this save).

## T2 — Land combat detail (50 live) — CRACKED
CCombatSide (vt 0x29913C0): committed @+0xD4, reserve @+0xEC, units @+0x2C; damage
as **i64 ×1e5** — anti_air @+0x138, air_dmg str/org @+0x148/+0x150, ground_dmg
str/org @+0x158/+0x160, prevented str/org @+0x168/+0x170; air_kills @+0x140,
org_loss_summary_index @+0x1A8, num_org_losses @+0x1AC — all **100/100 exact**.
`tactic` = **PARTIAL** (pointer/table-backed, no inline encoding — bounded no-user
RE). width/terrain/phase/planning/direction = **DERIVED** (Codex-endorsed).

## T3 — Naval combat detail (4 live) — CRACKED
CNavalCombatant (vt 0x29CD298), i64×1e5, **8/8 exact**: positioning @+0x138,
dominance_bonus @+0x148, anti_air @+0x150, total_initial_strength @+0x158,
total_damage_dealt @+0x160. Members/roles already D49/50. Per-sink killer =
pre-existing NEEDS-USER (1-event).

## T4 — Air combat + reinforcement
- Reinforcement reason = **DERIVED-absent** (no stored cause; Codex-confirmed).
- SAirWingCombatData @+0x10/+0x14 country-pair = **CRACKED (40/40)**; its
  kill/loss/sortie stat fields = **NEEDS-USER** (all-zero on the paused save,
  runtime-only, no melt oracle — need live air combat).

## T5 — Breakthrough / research (minor)
- Special-project breakthrough = **PARTIAL**: CProject identity solid (vt 0x296C270);
  completion/progress candidates @+0x35/+0x38/+0x3C/+0x70; per-record join deferred on
  the program_status owner-container (bounded no-user RE, not user).
- Research progress = **replay need MET by D45**; no serialized progress key in the
  melt (not required for replay).

## T6 — Scripted-state coverage audit
Domain 51 covers **every non-empty scope** (country/global/state, vars + flags). The
only "missed" categories are delayed-event queues + scripted-GUI (both already D51
fail-closed-badged, roots unresolved) + a character-var scope that is RNG-seed-only
(empty) on D04. **No new missed-content category** — D51 coverage is complete for
what exists in this save.

## Net: the definitive residual "needs the user" (memory)
1. Hub motorization level (2 differing hubs) — pre-existing.
2. Focus-paused bit (pause one focus) — pre-existing.
3. Convoy/port/disrupted-supply PRESSURE (one blockade) — pre-existing.
4. MP chat/messages + hotjoin (2nd player) — pre-existing.
5. Game-speed value (10s B0 tap, or deeper disasm) — from C2.
6. Live air-combat stats (SAirWingCombatData populated only during live air combat).
7. Per-entity naval/air KILLER label (1 controlled before/after event).

Everything else in memory is CRACKED, DERIVED (replay recomputes), or a bounded
no-user RE join (land-combat tactic table; breakthrough program_status container).
No un-attempted memory category remains.
