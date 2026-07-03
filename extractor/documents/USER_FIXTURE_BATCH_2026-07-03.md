# User fixture batch — 2026-07-03 (D04 1944 session)

These are the in-game actions that need YOU. The game is paused on `D04_1944_02_11_18`.
Grouped by whether they can be done on THIS save/session vs need a fresh setup. Each item
names the exact action + what the probe then validates. Ordered by value.

## A. Can be done on the CURRENT paused D04 save (no reload)

### A0. Naval combat 49/50 live validation — TIME-SENSITIVE (keep game paused)
The full side->CFEXGroup->member->CShip chain is cracked and cross-checked (13/13 ships,
4/4 battle locations match the save). To PROMOTE it I need to port the chain into the naval
lane and re-run the Rust `active_fixture_proof` gate against these 4 live battles. As long as
the game stays paused on this save, the battles persist and I can validate. **Please keep the
game paused until I say the naval validation is done.** No manual action from you beyond that.

### A1. Division losses two-poll (validates loss + manpower memory offsets)
Candidates found: cumulative losses `CDivision+0x668`, current manpower `CDivision+0x3D0->+0x4`
(both 100% same-frame oracle match). To confirm they TRACK combat, I need a 2-hour delta during
active land combat. The save has 50 active land combats.
- Action: UNPAUSE at speed 3-4 for ~60-90 real seconds (≈2 in-game hours), then PAUSE. Keep the
  same battles running; do not disband/move the fighting divisions.
- I re-probe and require losses to rise + manpower to fall for fighting divisions, stable for
  controls.

### A2. Strategic redeploy (validates SR-mode field)
- Action: pick one division I name from the assignment probe, command STRATEGIC REDEPLOYMENT
  of it to a rear province, then PAUSE.
- I re-probe: accept only a field that changes for that division and not for ordinary movers.

### A3. Resource-flow UI oracle (paused; you just read numbers)
- Action: for IRE, GER, ENG, SWE, TUR, USA — open the resource topbar tooltip and read
  produced/imported/exported/used per resource. I give you a template to fill.
- Validates the resource-flow candidate containers (currently 0/8 without a UI oracle).

### A4. Supply tooltip parity (paused; read numbers)
- Action: supply map mode, hover provinces 1, 100, 500, 1000, 2000, 3324, 3544, 6421, 9244,
  12000, 13413; read remaining/hub/state supply. Template provided.
- Validates the exact-supply CProvince candidate against >=8 anchors.

### A5. Attrition reason (paused; read tooltips)
- Action: find one division with a visible attrition icon + 2 nearby controls without one;
  give me their ids/provinces + the visible reason text.
- Validates/rejects the attrition-flag candidate.

## B. Need a controlled in-game change on this session

### B1. Offmap factory delta (economy row 58)
- Action: change ONE country's offmap factory count by a known amount (e.g. a decision/console
  that adds N civilian factories to IRE) WITHOUT changing state buildings/occupation/trade.
- I re-probe for the offset that moved by exactly N. If none moves, offmap total is
  recomputed-only (proof-backed rejection).

### B2. Active production license (licensing row)
- Action: create/accept ONE production license between two known countries for a unique
  equipment type; pause immediately. Tell me licensor/licensee/equipment.
- Validates a license-detail root or rejects.

### B3. Hub motorization 2-level toggle (hub motorization row)
- Action: set two supply hubs to DIFFERENT motorization levels (ideally level 1 and level 3);
  pause. Tell me the two hub province ids + levels.
- The save oracle is single-level only, so this is the only way to disambiguate the level field.

### B4. Ace assignment (ace identity/assignment row)
- Action: pick an owner country with >=1 ace and an eligible wing; assign the ace to the wing
  (pause), then remove it (pause). Tell me owner tag / wing id / ace id.
- Validates the assignment side-effect field `body_assigned_ace_id_0x2DC`.

### B5. Disrupted-supply fixture (convoy/port/disrupted pressure rows)
- Action: create a disrupted-supply situation — e.g. an island garrison whose convoy route is
  blockaded (or cut a friendly pocket off from its supply hub) — then SAVE and pause.
- The current save has ZERO disrupted-supply anchors, so all disrupted/convoy/port pressure
  candidates FAIL by fixture absence (2732 FAILs each; foreign-homebase control passed 953).
  One disrupted anchor unlocks all three categories in a single probe run.

### B6. Garrison suppression drain (garrison row)
- Action: brief 6-24h unpause without changing occupation law/template/selected country, then
  pause; note any state whose garrison demand visibly changed.
- Tries to separate the ambiguous garrison child candidates (strength_ratio / garrison_required
  / manpower_need all currently tie).

## C. Need a fresh single-player fixture (disposable mod/console) — lower priority

- C1. Delayed-event decoder: the D04 save HAS 88 positive delayed-event rows but the live probe
  found no event-like RTTI container (candidates_rejected_no_event_like_rtti). Needs the
  2nd-pass field decoder (code job) OR the controlled `ssfv_delayed_probe` fixture. Not a
  quick action.
- C2. Focus-paused oracle, project/reward lifecycle, scripted-GUI, operations/spy/action-queue:
  each needs a specific controlled before/after fixture (see master contract rows).

## D. Need a multiplayer session (2+ players) — separate event

- Hotjoin/tag-switch + chat/messages (domain 57). Same-build MP session with a controlled
  join and unique chat text. Cannot be validated from single-player.

---
Fastest high-value path on THIS session: A0 (keep paused) -> A3/A4/A5 (paused reads) ->
A1/A2 (one short unpause) -> B-items as you have time.
