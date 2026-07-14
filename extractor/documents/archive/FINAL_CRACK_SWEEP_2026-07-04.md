# Final crack sweep — every remaining no-user memory item (2026-07-04, exe 870ebf, D04 1944.2.11.18 paused)

The comprehensive close-out: crack EVERY replay-relevant memory item that is still
open and does NOT need the user, from the live paused save. After this sweep the
only remaining extraction work should be (a) the 4 user fixtures, (b) the fine
per-entity combat-killer label (1 event), (c) C2 speed + C4 intel (owned by a
SEPARATE round-2 agent — do NOT touch), and (d) production WIRING (owned by the
wiring agent). Everything else is yours to crack.

For each item: CRACK it (offset/chain + melt-oracle or structural validation),
classify DERIVED (replay recomputes — state inputs), or return a precise
NEEDS-USER/absent negative. Discovery only: write to gitignored
`runs/finalsweep_<item>/`; touch NO tracked source; NO commits.

## OUT OF SCOPE — owned by other running agents, do NOT touch
- C2 game-speed root, C4 intel-network graph (round-2 crack agent).
- All production wiring / tracked source (wiring agent is editing
  collector/recorder/lanes concurrently — editing any tracked file corrupts it).
- User-fixture items (hub motorization, focus-paused bit, convoy/port PRESSURE,
  MP chat/hotjoin) and the finest per-entity combat killer label — genuinely need
  the user; do not re-attempt.
- DERIVED (already resolved, replay recomputes): province supply, resource flow,
  attrition reason, offmap factory.

## TARGETS (priority order)

### T1 — Capitulation / peace / surrender / war state  [biggest NOT_STARTED gap]
The political map's capitulation visuals + war state. This save HAS capitulated
countries and active wars. Crack from the paused save:
- **Per-country capitulated bool + surrender_progress scalar** on CCountry. Melt
  oracle: grep `capitulated=yes` and `surrender_progress=` under each country;
  count + values are the oracle. Find the CCountry offsets (structural + value join).
- **War state**: the war/faction system. Domain 53 already has war tag-pair/casualty
  rows "badged lifecycle-unvalidated" — VALIDATE them against the melt war blocks
  (participants, casualties, war exhaustion). Melt oracle: the `wars`/war blocks.
- **Peace conference state** (if a conference is active on this save — check the melt
  for a peace-conference block; if none, mark absent/fixture-blocked, not a failure).
- **Capitulation visual** (who controls what after capitulation) is already covered via
  map_control (D22); this item is the per-country surrender STATE + war detail.
DONE = capitulated/surrender_progress offsets validated vs melt + war-casualty rows
validated (or precise negatives).

### T2 — Land combat detail  [50 live battles on this save]
Domain 55 land_combat_detail is registered but the field set is incomplete. The 50
active CLandCombat objects are live NOW. Crack the per-combat detail fields:
combat width/frontage, attacker/defender selected tactic, casualties-so-far
(men+equipment), committed/reserve counts, terrain, phase, planning/entrenchment
bonus, direction. Melt oracle: the active combat blocks in the save (attacker/
defender, progress, etc.). Read from the live CLandCombat vtable (wall-crack found
CLandCombat 50 live). Cross-check the SDamageDone matrix (CStatsObserver→+0x38,
already cracked) for per-combat damage. DONE = the combat-detail fields validated
vs melt for the active battles (or precise per-field negatives).

### T3 — Naval combat detail  [4 live naval battles]
Beyond the wired D49/50 lifecycle+members: per-battle positioning/engagement state,
damage-so-far, participating task-force roles. The 4 CNavalCombat(body) objects are
live. Use the SDamageDone matrix + CSunkShipInfo (10416, sink-date @+0x50/+0x60 +
country tags) for loss detail. DONE = naval-combat detail fields validated vs melt
(or negatives) for the 4 battles.

### T4 — Air combat detail + reinforcement reason
- **Reinforcement reason** (D42, ROOT_DISCOVERY_OPEN): find the root that distinguishes
  production-reinforcement vs transfer vs attrition-replacement vs deleted-wing. If no
  stored cause exists → DERIVED/absent with proof.
- **Full air-combat exact fields**: the persistent SAirWingCombatData (12889 live, vt
  0x2A099E0, +0x10/+0x14 enemy/owner db_ids) found by wall-crack — decode its stat
  fields (kills, losses, sorties) and validate against any melt air-stat oracle. DONE =
  air-combat-data fields decoded + validated (or negatives).

### T5 — Breakthrough state + research exact fields  [minor]
- Special-project **breakthrough** state (LANE_LOCAL_PROVED) — shared-integration-ready
  offsets + semantic names, from the project pool objects (project lifecycle already
  cracked at CProject; find the breakthrough sub-state).
- **Research exact speed/modifier** engine fields (D45 is a production slice; ETA is
  delta-derived) — only if a stored per-tech speed/progress field exists that the
  replay would need; else DERIVED. Low priority.

### T6 — Scripted-state / mod-dynamic-state coverage audit
Domain 51 scripted_state_flags_variables_live is validated. AUDIT its live coverage
against the melt's full scripted-variable/flag set (country + state + global): does
D51 capture EVERY replay-visible scripted var/flag, or are there missed categories
(e.g. mod-added dynamic flags)? Report the gap (missed var categories) — a bounded
inventory, not a generic-mod-extractor build. DONE = a coverage delta (D51 live vs
melt scripted vars) + any missed-category offsets.

## ENVIRONMENT + PROTOCOL (same as prior sweeps)
- Worktree, PID 28044, SHA gate `870ebf1f...`, paused D04 1944.2.11.18, melt oracle
  `runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`.
  `PYTHONPATH=extractor/memory_reader`; probe_common + winmem (scan_module/rip_target/
  iter_regions); **capstone 5.0.7** installed. Use `open_process_with_sha_or_report`.
- DISCOVERY ONLY: write only under gitignored `runs/finalsweep_*/`; touch NO tracked
  source; NO commits/git add. Fail closed both directions (never promote unproven;
  absent != 0). The game is PAUSED — do not control it.
- Codex adversarial gate before claiming ANY crack: Mode A, xhigh, unique -o, verify
  exit 0 + non-empty; retry on the 2-min harness timeout; report honestly on os error
  10013. Gate DERIVED classifications especially (Codex catches "you missed the field").
- Per item: `runs/finalsweep_<item>/EVIDENCE.md` (chain/offsets, melt validation numbers,
  Codex verdict, CRACKED/DERIVED/NEEDS-USER). Final: a master table over T1–T6 —
  the definitive "everything left in memory that doesn't need the user."
