# Final crack list — re-attempt DERIVED + moved RE joins + no-user attempts (2026-07-04)

Every remaining item that involves REVERSE-ENGINEERING (not wiring). All cracking
is on THIS agent; the engineering agent does pure wiring only. Discovery-only:
write to gitignored `runs/cracklist_<item>/`; touch NO tracked source; NO commits.
For each: CRACK it (offset/chain + melt-oracle or structural proof), CONFIRM-DERIVED
(prove no stored field, with the exact disasm), or return a precise NEEDS-USER
negative. The coordinator verifies each crack against the melt before trusting it.

Environment: worktree below, PID 28044, SHA gate `870ebf1f...`, paused D04
1944.2.11.18, melt oracle
`runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`,
`PYTHONPATH=extractor/memory_reader`, probe_common + winmem (scan_module/rip_target/
iter_regions), **capstone 5.0.7**. Use `open_process_with_sha_or_report`. Codex
adversarial gate before claiming ANY crack (Mode A, xhigh, unique -o; retry on the
2-min timeout; honest on os error 10013). Do NOT spawn sub-agents. Do NOT touch
`replay_interface/` or any tracked source (two agents edit it concurrently).

## GROUP A — re-attempt the DERIVED items as real cracks
These were dispositioned DERIVED (replay recomputes). Re-attack: hunt for a STORED
field the prior pass missed. Some are strongly derived (confirm with disasm); some
may have a cached field. Prior evidence in `runs/wallcrack_*` / `runs/finalsweep_*`.

- **A1 Province supply** — prior: CONFIRMED-DERIVED (tooltip formatter rva 0x1623536
  sums CSupplySystem per-node supply `[node+0x80]+0x138`, ×1e5). Re-attempt: is the
  summed result CACHED anywhere (per-province, on CProvince or a supply-node)? If a
  cached scalar exists, crack it; else confirm DERIVED with the disasm.
- **A2 Resource flow** (produced/imported/exported/used) — prior: lives in
  `NInternationalMarket` (global trade mgr) + `STemporaryResourceData@CState` (RVA
  0x2920670, per-state); produced=Σ state outputs, import/export=market rows. Decode
  those containers and VALIDATE per-country produced/used/imported/exported vs the
  melt resource + trade blocks. This is likely CRACKABLE (stored, just not a per-
  country array) — pin the fields.
- **A3 Attrition reason** — melt `attrition={}` empty (no serialized enum). Re-attempt:
  does CDivision cache a dominant-attrition-reason enum for the tooltip (disasm the
  attrition-icon/tooltip code)? Scalars already cracked (supply_ratio +0x600, fuel
  +0x610, disrupted +0xA0). If no stored enum → confirm DERIVED.
- **A4 Offmap factory total** — prior: DERIVED-candidate; Codex REFUTED a clean
  DERIVED (a stored counter, possibly country `buildings.level`, isn't ruled out).
  Re-attempt: find the stored offmap civ/mil factory counter on CCountry/economy;
  validate vs the melt offmap block. Resolve DERIVED-vs-stored definitively.
- **A5 Surrender progress** — prior: draw-time. Re-attempt: cached per-country
  surrender_progress scalar near the capitulation fields (D=u64[CCountry+0xF88];
  capitulated @D+0x2E0 already cracked — scan D+0x1B0..0x300 for a 0..1 progress).
- **A6 Reinforcement reason** — prior: no stored cause (Codex-confirmed). Re-attempt:
  disasm the reinforcement path (production-reinforce vs transfer vs attrition-replace
  vs delete) for a stored cause enum; else confirm DERIVED.
- **A7 Land-combat width/terrain/phase/planning/direction** — prior: DERIVED. Re-attempt:
  are these stored on CCombatSide (vt 0x29913C0) or CLandCombat? Scan the combat body
  for width/frontage/terrain-id/phase/planning-bonus/direction fields; validate vs the
  melt active-combat blocks (50 live). Any that are stored → crack; rest → confirm DERIVED.

## GROUP B — RE joins moved off the engineering (wiring) queue
- **B1 Land-combat tactic** — CCombatSide has a tactic POINTER/table-backed value (no
  inline encoding). Decode the tactic-definition table it points into; map to the
  tactic id/name; validate vs the melt combat `tactic=` for the 50 live battles.
- **B2 Breakthrough per-record join** — CProject identity solid (vt 0x296C270);
  breakthrough completion/progress candidates @+0x35/+0x38/+0x3C/+0x70. Close the
  per-record join on the program_status owner-container (like the projects lifecycle
  chain CCountry+0xFA8→CProgramStatus→CProjectPool) and validate breakthrough state
  per (country, project) vs the melt.
- **B3 Active-war casualty magnitude** — the war-pool casualty offsets are valid
  (958/958 pairs), but ACTIVE wars use a running accumulator not byte-equal to the
  serialized snapshot. Find the serialized-snapshot-equivalent field (or prove the
  accumulator IS the right live value and the melt snapshot is a serialize-time copy).
- **B4 d53 base relation/opinion root** — the d53 base extract is
  SOURCE_UNAVAILABLE+PARTIAL on 870ebf (its live relation/opinion root offsets are
  unvalidated). CRACK the base root so the relation/opinion/subject data can emit.
  (The engineering agent is separately DECOUPLING operations/diplo/intel into
  independent records so THOSE emit regardless; your job is the base relation data
  itself.) The operatives discovery already works, so part of d53 is reachable — start
  there. Validate relations/opinions vs the melt.

## GROUP C — no-user attempts on items previously filed NEEDS-USER
Each has a plausible NO-USER (disasm) route. Try it; if it genuinely needs the user,
say so precisely.
- **C1 Live air-combat stats** — SAirWingCombatData (vt 0x2A099E0, +0x10/+0x14 country
  pair cracked 40/40) stat fields (kills/losses/sorties) read ZERO on the paused save
  (no air combat). NO-USER route: disasm the air-combat writer to PIN the stat field
  OFFSETS structurally (where kills/losses/sorties are written), even though values
  can't be value-validated while paused. Deliver the field offsets + an honest "values
  need live air combat to validate."
- **C2 Per-entity killer label** — CSunkConvoyInfo (0x295D008) / CSunkShipInfo
  (0x2941608) carry 3 country slots; which is attacker-credit vs victim vs partner is
  unlabeled. NO-USER route: disasm the SINK-RECORDING WRITER (the fn that writes
  CSunkConvoyInfo) to see which arg lands in +0x0C vs +0x10 vs +0x14. That labels the
  slots without a live event. (The country-granularity SDamageDone matrix is already
  cracked; this is the per-sink entity label.)
- **C3 Game-speed value** — mechanism found (CSetGameSpeedCommand vt 0x7FF757B005A0,
  CIncrease/Decrease, define GAME_SPEED_SECONDS); `+0x66C` RETIRED (constant 100). NO-
  USER route: disasm `CSetGameSpeedCommand::DoExecute` to find the field it WRITES
  (the stored speed index 0..5), OR index-slice GAME_SPEED_SECONDS by a bounded 0..5
  operand to locate the field the scheduler reads. If the disasm genuinely can't pin
  it, defer to fixture B0 (10s user tap) with the ≤3 CGameState-first candidates.

## DELIVERABLE
Per item: `runs/cracklist_<item>/EVIDENCE.md` (chain/offsets, disasm/heap trace, melt
validation numbers, Codex verdict, CRACKED / CONFIRMED-DERIVED / NEEDS-USER). Final
master table over A1–A7, B1–B4, C1–C3. That is the definitive last word on what in
memory is crackable without the user.
