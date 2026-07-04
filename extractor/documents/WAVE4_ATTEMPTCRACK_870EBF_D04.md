# Wave-4 "attempt-crack everything without game access" — 870ebf, paused D04 (1944.2.11.18)

A Fable discovery agent was given EVERY open ledger row (including fixture-blocked
ones) and told to crack what it could from the CURRENT paused save + melt oracle,
with NO in-game action by the user, and to report the genuinely-blocked with the
exact reason. Result: SIX items cracked (several were assumed to need fixtures),
plus a precise user-dependency list. Orchestrator-verified the key offsets against
the oracle (unique full-population diagonals, decoy encodings ruled out). Codex-
reviewed (Mode A, exit 0). Run evidence: `runs/attemptcrack_*/` (gitignored).
These are VALIDATED OFFSETS ready to become integration packets — not yet wired
(except delayed-event due-date, handed to the concurrent wiring agent).

## CRACKED (no user needed)

### Naval missions / home base / repair  (was FIXTURE_BLOCKED)
192 task forces / 153 fleets matched vs melt.
- `CTaskForce+0x364` i32 = mission enum (192/192; 0,1,2,3,4,8,9)
- `CTaskForce+0x384` i32 = navy_engagement_rule (192/192)
- `CTaskForce+0x38C` i32 = hours_in_mission (134/134; read only when a mission is active)
- `CTaskForce+0x454` i32 = repair_mode (observed values only)
- `CTaskForce+0x1E0` = current-location CProvince* (pointer form of domain-56's +0x18)
- `CTaskForce+0x4A8` i32 = detached_activity (repairing=1/refitting=3/reinforcing=4/none=0)
- `CTaskForce+0x4B0` = repair_target CProvince* (nullable; 3/3 set + 189/189 null)
- **`CFleet+0xD8` = home_base CProvince*** (119/119)
Blockers on sub-fields with no D04 variance: automated_homebase (all no), underway_replenishment (all no), retreat (no naval retreat field in schema).

### Ace identity + wing assignment + death record  (was FIXTURE_BLOCKED)
258 roster aces / 28 dead / 162 assigned wings.
- CAceEntry: `+0x0C` id, `+0xF0/+0x110/+0x130` name/surname/callsign, `+0x154` portrait, `+0x158` u8 alive (1/0), `+0x164` killer ace id, `+0x168` killer country tag-db-id.
- ENCODING CORRECTION (wiring, 2026-07-04, live capture `runs/wire_ace_lifecycle_proof`): the
  strings are UTF-8 in live memory (byte-identical to the melt), NOT cp1252. The original
  "cp1252 strings" note came from the discovery oracle JSON being read as latin-1 (its own
  mojibake); decoding live as cp1252 reproduces exactly that mojibake. Production decodes
  strict UTF-8 — all 258 names exact vs melt (747 name fields exact, 27 oracle-side
  double-encodings, 0 unexplained).
- **`CAirWing+0x2DC` u32 = assigned ace id** (162/162 + 478/478 null) — WITHOUT the 0x2FB5 command fixture.
- Key by ace id (`+0x0C`), not roster order (Codex). Residual blockers (no D04 variance): kill_type vs handled (both read 1-on-dead), wounded flag (0 wounded in D04).

### Production licensing  (was ROOT_DISCOVERY_OPEN)
- `CCountry+0xF68` -> production manager; **`manager+0x198` u64 data / `+0x1A0` u32 cap / `+0x1A4` u32 count**; elements = stride-8 pointers to type-70 CEquipmentVariant objects (gate u32@+0x08==70; variant id u32@+0x0C).
- 4/4 licensee countries exact id-set; 433/437 negative control clean-zero. Foreign-lease confound excluded (GER holds a licensed id in its lease block but mgr count 0). `CCountry+0x1070/+0x10A0` stay rejected.

### Delayed-event DUE-DATE  (closes the wave-3 residual)
- **`CDelayedEvent+0xC4` u32 = total remaining hours = months*720 + days*24 + hours** (30-day months). 17/17 exact, unique full-population offset; decomposed days/hours/months and absolute-timestamp forms all matched nothing. Reader: `due_remaining_hours = u32[CDelayedEvent+0xC4]`. HANDED to the wiring agent (delayed-event now ships complete: owner + event_id + scope + due_remaining_hours).

### Disrupted supply  (was thought to need a disrupted-supply fixture)
- **`CDivision+0xA0` i32 /100000 = disrupted supply** — 6/6 exact + clean 6-of-3168 negative control. It is a per-DIVISION field, not a supply-node pressure field.

### Attrition supply signal  (re-validated)
- `CDivision+0x600` = army_current_supply_ratio, `CDivision+0x5F8` = supply_gain (3162/3162 each).

## NOT CRACKED — exact reason

### Genuinely need the user (in-game action)
- **Hub motorization level** — every hub in D04 is level 2; the level offset can't be disambiguated from a constant. Needs a save with >=2 distinct hub levels (toggle one hub). (Also the live {tag->level} container is still unfound in bounded scans.)
- **Focus paused state** — 113 countries have a focus in progress, ALL paused=no. No positive control. Needs a state with >=1 paused national focus.
- **Convoy combat losses / route** — involvement is cracked (naval lane); a "loss" is a before/after delta and per-member combat state drifts live-vs-melt. Needs live/advancing combat.
- **Air sortie / kill / last_killer / reinforcement reason** — no per-wing kill/loss/last_killer field in the save schema (only air_accidents, already cracked; air_kills is per-combat-side). Needs live air combat.
- **Convoy / port pressure** — port_pressure/convoy_pressure appear ZERO times in the melt. Needs a naval-interdiction fixture.
- **Exact province supply tooltip** — "remaining supply" is UI-computed, not serialized (0 melt hits). Needs same-frame supply-map tooltip capture (user/OCR).

### Not user-blocked — need deeper reversing (a future discovery pass, no fixture)
- **Operations / spy-network-intel-map / diplomacy action queues** — operative identity re-confirmed (COperativeLeader id +0x0C, 80/80) but mission-target not in the operative body/one-hop; the running-COperation + intel-graph + diplo-queue container roots are deferred domain-53 work.
- **Market stockpile owner chain** — container decode solved (`CEquipmentVariantPool` 0x27330A0, data +0x20, stride-16 {variant*, fx100000 amount}, 22-29/70 by content) but no CCountry inline/one-hop back-ref (33432 pools hang off a market manager, not per-country). Needs a market-manager->pool root.
- **Projects / rewards** — identity re-confirmed (CProject vtable 0x296C270, 21531 live); completion flags population-plausible (u8 +0x35/+0x6D) but not per-record validated (melt project ids restart per-country vs global live ids). Needs the program_status container root for the owner-join.
- **Resources produced/import/export & factory totals** — both STATE-DERIVED, not per-country scalars (only 1/5 GER produced values appears in the CCountry body; factory totals confirmed no cached scalar). Reconstruct in the replay from the state domain + trade routes — a derivation, not an offset.
