# Derived recompute specs (replay-side) — A1/A3/A5/A6/A7

**Purpose:** Specify replay-side recomputation for five engine-derived quantities, including
inputs, formulas, evidence grades, outputs, and honest residuals.
**Completion status:** See `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; this reference does
not track implementation or backlog state.

Each of the five quantities below is
**NOT a stored save/live field** (absence proven in the cited evidence); the engine
recomputes it at draw time. This document gives the replay layer everything needed to
reproduce each value from already-captured inputs — exact input offsets → formula →
output field — so it can implement without re-deriving.

Build: 870ebf (v1.19.2 "Operation Postern"), module base resolved at runtime. All fixed-point
scalars are i64/i32 ×100000 unless noted (divide by 100000 for the human value). Every game-file
read MUST go through `mod_resolver` keyed by the recording modset (see
`replay_interface/documents/MOD_COMPATIBILITY.md`) —
vanilla paths below are the fallback.

Historical source-evidence paths (not present in this worktree; retained for provenance):
- A1 `extractor/native/runs/cracklist_a1_supply/EVIDENCE.md`
- A3 `extractor/native/runs/cracklist_a3_attrition/EVIDENCE.md`
- A5 `extractor/native/runs/cracklist_a5_surrender/EVIDENCE.md`
- A6 `extractor/native/runs/cracklist_a6_reinforce/EVIDENCE.md`
- A7 `extractor/native/runs/cracklist_a7_landcombat/EVIDENCE.md`

Common roots: `CGameState = u64[module_base + 0x3316E90]`.

---

## A1 — Province "Supply Remaining in Province" (DERIVED; draw-time aggregate)

Not save-backed under any name (grep of the D04 melt: `remaining_supply|province_supply|
supply_available` = 0 hits). The tooltip value is aggregated at draw time by `sub_0x1939740`
over the live supply structures. Replay re-runs the same aggregation.

### Roots
```
CGameState    = u64[module_base + 0x3316E90]
CSupplySystem = u64[CGameState  + 0x3D8]
```

### Per-province source-list walk (province_ctx = the province supply context)
```
begin = u64[province_ctx + 0x60]     // source-element array base
count = i32[province_ctx + 0x6C]     // element count
element stride = 8 bytes; element = { i32 node_id @+0x0, u8 type @+0x4 }
```

### Per-node supply read (NOTE the node-array deref — Codex catch)
```
node_array = u64[CSupplySystem + 0x108]   // deref FIRST to get the array base
node       = u64[ node_array + node_id*8 ] // then index by node_id (stride 8)
p          = u64[node + 0x80]
supply     = i64[p + 0x138]                // per-node supply, ×100000 fixed-point
```

### Type-weight switch (u8 `type` at element+0x4)
```
case 1:  contribution = trunc( supply * 50000 / 100000 )   // = supply × 0.5 (HALF weight)
case 2:  contribution = supply                              // = supply × 1.0 (FULL weight)
default: contribution = 0                                   // type 0 or ≥3
S = Σ contribution over the source list   (×100000 fixed-point)
```
`case 1` is `imul ×50000` then an exact `÷100000` signed magic-divide (round-toward-zero
= integer truncation for positive values). Net = ×0.5.

### Outer post-processing (part of the displayed value; helper `sub_0x14CE920` = saturating
### clamp: if input < 0 substitute the global cap at rva 0x33717E8)
```
A        = clamp( S )
B        = clamp( -100000 )
elapsed  = i32[0x331BFE0] - i32[province_ctx + 0x94]      // globalTick − last-update tick
timeTerm = trunc( elapsed * i64[0x331D160] * 100000 / 100000 )  // consumption-since term
net      = A - timeTerm - B + carry                       // carry = formatter rbx (residual)
clamped  = max(0, net)
denom    = i64[province_ctx+0x38] + i64[province_ctx+0x20] + i64[province_ctx+0x48]
if denom <= 0:  factor = 100000
else:           ratio  = i64[province_ctx+0x48] * 100000 / denom
                factor = clamp(100000 - ratio, 0, 100000)
result   = trunc( clamped * factor / 100000 )
```

### Output
`out_field = province_supply_remaining` = `result` (i64 ×100000). Divide by 100000 for the
tooltip number.

### Residuals (do not change classification)
- `carry` (formatter rbx) provenance untraced; helper `0x14CE920` and globals
  `0x331BFE0`(globalTick)/`0x331D160`(rate)/`0x33717E8`(cap) named by role, not fully proven.
- Byte-exact decimal QA needs one same-frame supply-mapmode tooltip capture (≥8 provinces).

---

## A3 — Division attrition REASON classification (DERIVED; argmax over live scalars)

No attrition-reason enum is serialized (the 64 melt `attrition={}` occurrences are all
equipment-stat modifier fields, unrelated). The reason is a UI draw-time classification over
the division's live conditions. Taxonomy = the game's OWN six attrition categories
(`localisation/english/unit_l_english.yml`): SUPPLY, TERRAIN, STATE, WEATHER, TRAINING, TECH.
**Fuel is NOT an attrition category** (it is a separate speed/combat penalty).

### Live input scalars (CDivision, build 870ebf)
```
supply_ratio = i32 @ CDivision + 0x600     // (unit scale residual: 0..1 vs 0..100 — see below)
fuel         = f?  @ CDivision + 0x610      // AUXILIARY signal only, NOT a reason
disrupted    = i32 @ CDivision + 0x0A0
```
plus the division's current province weather + terrain (readable live; terrain via A7 §1).

### Thresholds & magnitudes (defines + installed content, modset-resolved)
| category | onset gate | magnitude source |
|---|---|---|
| SUPPLY   | `SUPPLY_THRESHOLD_FOR_ARMY_ATTRITION = 0.35` | ramps to `OUT_OF_SUPPLY_ATTRITION = 0.20` over `MAX_OUT_OF_SUPPLY_DAYS = 30` |
| TERRAIN  | province terrain base `attrition > 0` | `00_terrain.txt`: mountain .30, marsh .35, jungle .20, desert .15 (plains/forest/hills/urban 0) |
| WEATHER  | province weather modifier active | `00_static_modifiers.txt`: mud .70; extreme_cold .40 / very_cold .25; extreme_hot .20 / very_hot .10 |
| TRAINING | division on exercise | `TRAINING_ATTRITION = 0.05` |
| STATE    | state has an attrition modifier | per-state modifier (RESIDUAL: not in the 3 scalars) |
| TECH     | country tech attrition factor | per-country tech modifier (RESIDUAL) |

### Classifier (argmax by contribution — NOT fixed priority)
```
contrib_supply  = (supply_ratio < 0.35) ? lerp(0..0.20, worse supply → more) : 0
contrib_terrain = terrain_base_attrition(province.terrain)
contrib_weather = weather_attrition(province.weather)
contrib_train   = on_exercise ? 0.05 : 0
contrib_state   = state_attrition_modifier(province.state)   // RESIDUAL
contrib_tech    = country_tech_attrition_factor(owner)       // RESIDUAL
reason = argmax_category(contrib_*) ; if all 0 → NONE
```
Output: an enum tag chosen at replay time; never a stored field.

### Residuals
- **Unit scale of `+0x600`**: is it 0..1 or 0..100? The melt `army_current_supply_ratio`
  reads e.g. 17.39, hinting a non-0..1 scale; if ×100 the compare is `<35` not `<0.35`. One
  live cross-check on a known out-of-supply division resolves it.
- STATE and TECH contributions need state/country modifier reads (beyond the 3 scalars).
- Classifier reports the DOMINANT reason; game may show several simultaneously.

---

## A5 — Surrender progress fraction (DERIVED; clamp(E / L_eff))

No stored scalar (`surrender_progress=` = 0 hits in melt; live D-region scan found no [0,1]
war-state discriminator). Capitulation STATE stays authoritative from the stored bool/date
(T1 crack: `D = u64[CCountry+0xF88]`; capitulated bool @ `D+0x2E0`; capitulated_date @ `D+0x2F0`).
Only the in-progress FRACTION is recomputed (display/analytics only).

### Inputs (all replay-available)
| input | source |
|---|---|
| state owner / controller | melt `states={<id>={owner controller}}` |
| province controller overrides | melt top-level `provinces={<id>={controller}}` |
| province victory points | modset `history/states/*.txt victory_points`; save override wins |
| state→province membership | modset `history/states/*.txt provinces={...}` (NOT in save) |
| war graph (sides) | serialized diplomacy/wars (wired: e1_wire_capitulation_war) |
| surrender_limit / forced_surrender_limit / max_surrender_limit_offset | country ideas + dynamic_modifier + variables, modset-resolved |
| collaboration | country `collaboration={TAG=val}` |
| owns-any-core flag | melt state `cores` vs owner |

### Defines (`common/defines/00_defines.lua`, NDefines.NDiplomacy)
`BASE_SURRENDER_LEVEL=1.0`, `BASE_SURRENDER_LIMIT=0.8`,
`SURRENDER_LIMIT_MULT_FOR_COUNTRIES_WITH_NO_CORES=0.7`, `MIN_SURRENDER_LIMIT=0.2`,
`SURRENDER_LIMIT_REDUCTION_PER_COLLABORATION=0.15`, `UNCAPITULATE_LEVEL=0.1`.

### Formula (country C at time t)
```
OwnedProvinces(C) = { p : p ∈ provinces(S), owner(S)=C }
vp(p)             = save_override(p) if present else history_vp(p)
VP_total(C)       = Σ vp(p) for p ∈ OwnedProvinces(C)
ctrl(p)           = province_override(p) else state_controller(state(p)) else owner(state(p))
VP_enemy(C)       = Σ vp(p) for p ∈ OwnedProvinces(C) where ctrl(p) is at war against C
E                 = VP_enemy / VP_total   (E = 0 if VP_total = 0)

L0     = BASE_SURRENDER_LIMIT(0.8) + Σ surrender_limit
if C owns no core states: L0 *= 0.7
L0    -= 0.15 × collaboration                 // collab ∈ [0,1]
floor  = MIN_SURRENDER_LIMIT(0.2) + Σ max_surrender_limit_offset
L_eff  = max(L0, floor) + Σ forced_surrender_limit   // forced bypasses floor
L_eff  = clamp(L_eff, ε, 1)                   // L_eff ≤ 0 ⇒ progress = 1.0

surrender_progress = clamp(E / L_eff, 0, 1)   // capitulates at 1.0
```
Tooltip cross-check (waroverview_l_english.yml:33): `SURRENDER = E/L_eff`, `CONTROL = 1−E`,
`VAL = 1−L_eff`.

### Output
`out_field = surrender_progress` (f in [0,1]); ALSO emit `vp_enemy_frac` and `vp_not_own_frac`
(H1) so a validator can flag divergence when a non-enemy third party controls C-owned VP.

### Residuals (ranked)
1. Numerator scope: enemy-controlled (chosen) vs not-self-controlled — differ only under
   third-party control of C-owned VP.
2. Collaboration subtractive (chosen) vs multiplicative — differ only at high collab.
3. `max_surrender_limit_offset` read as raising the MIN floor.
4. no-cores mult applied post-additive.
5. VP_total=0 ⇒ E=0.
Capitulation EVENTS are never recomputed (stored bool/date authoritative).

---

## A6 — Reinforcement REASON (DERIVED delta + honest unrecoverable bound)

Hard bound: the per-event authoritative source-transaction cause (production vs stockpile vs
transfer vs attrition-replacement vs rebase) is a WRITE-TIME property, provably NOT retained in
serialized or live state, and paused-state polling cannot reconstruct it (the endpoint map is
non-injective: distinct cause decompositions produce identical S_t0→S_t1 pairs). Replay may emit
the observable delta + a confidence-BADGED source heuristic, never an authoritative cause.

### Observable delta record (per snapshot pair [t0,t1])
| field | source domain |
|---|---|
| wing/division id + owner | D30 air_wings_live / D20 divisions |
| equipment variant id/type, old/new count, Δ | D32 air_wing_loadouts_live / D26 division_equipment_live |
| location / mission region | D30 / D20 |
| window | snapshot timestamps |

### Correlation inputs (already cracked/production)
Δproduced(v) (D11 production_live `produced/active_factories/equipment_variant_index`),
Δstockpile(v) (D11 stockpile), Δdelivered(v) (D43 market/lend-lease `CPurchaseContract` progress
+0x1E8 / completed +0x25C / days +0x1D0 / delivery-tag +0x5C), losses (D42 accepted slice),
per-equipment cumulative counters (save `country.logistics.history` ring queues),
variant provenance (top-level `equipments={<v>={creator origin}}`; creator≠owner ⇒ foreign
source), attrition scalars (A3).

### Heuristic + badges
```
candidates = {}
if Δstockpile_C(v) < 0                         → += stockpile_draw
if Δproduced_C(v) > 0 and line targets v       → += fresh_production
if D43 contract delivering v to C progressed W → += transfer_delivery
if creator(v) ≠ C and Δstockpile_C(v) ≥ 0      → ∩= {transfer, capture}
if U took losses in W and Δ restores toward target → demand_tag += attrition_replacement
if U identity/base changed in W                → += rebase_or_reorg
conservation: Σ Δunit(v) ≤ max(0,−Δstockpile(v)) + Δproduced(v) + Δdelivered(v) + tol
   violation ⇒ badge unexplained_influx
```
Emit `possible_from_stockpile|production|transfer` + `possible_rebase_or_reorg`, each with
confidence HIGH (exactly one candidate + conservation balances) / MEDIUM (≥80% one candidate) /
LOW (multiple / slack → emit all). `demand_attrition_replacement` is a parallel DEMAND badge.
FORBIDDEN as exact causes: `reinforced_from_production` / `_from_transfer` / `attrition_replacement`.

### Output
`out_fields = { reinforce_delta record, source_badge[], source_confidence, demand_badge[] }`.
Never an authoritative per-event cause. `HIGH` ≠ authoritative (account for returns from
deleted/reorganized units, outbound sales, captured equipment, obsolete cleanup, same-window
stockpile masking). Rebase/reorg may be exact only as an identity/base transition if observed.

---

## A7 — Land combat 5-tuple: terrain / width / phase / planning / direction (DERIVED)

Shared chain: `CGameState = u64[module+0x3316E90]`; land-combat manager `CGameState+0x260`
(data `+0x08`, count `+0x14`); entry `u64[data+i*8]`; body `entry+0x10` (vtable RVA 0x29915B8,
type u32 @+0x08 == 62, combat_id @+0x0C); attacker `CCombatSide = u64[body+0x18]` (vt RVA
0x29913C0); registry `u64[att+0x18]`; defender `u64[registry+0x30]`. Divisions: country
`+0x290/+0x29C` wrapper vec, body = wrapper+0x10 (vt RVA 0x2943CB0), id-type u32 @+0x08 (51 or
4713 — two id namespaces, key by (type,id)), id u32 @+0x0C. `CProvince` vtable RVA 0x295AF68.
Installed content modset-resolved.

### 1. TERRAIN (CONFIRMED, oracle-proven both routes)
```
front_province_id = u32[ u64[body+0x28] + 0xA4 ]
static (replay):  map/definition.csv row front_province_id, column 7 = terrain category
live route:       u64[body+0x38] → CTerrainType (vt RVA 0x29AA2B0); key msvc-string @+0x18
```
Proof: melt `terrain` == definition.csv[front] 70/70; live CTerrainType bijective 20/20.
Output `combat_terrain`.

### 2. COMBAT WIDTH / FRONTAGE (DERIVED formula; NO numeric oracle — grade "derived (formula)")
Inputs: terrain combat_width/support_width (`00_terrain.txt`: plains/hills/desert 70/35,
forest/jungle 60/30, mountain/marsh 50/25, urban 80/40); n_directions (§5); per-side tactic
`u64[side+0x110]` → CCombatTactic (key @+0x10, DB idx @+0x98 == melt `tactic` — closes T2 PARTIAL)
→ `combat_tactics.txt combat_width` modifier; committed div count u32 @ side+0xD4.
```
total_width = (terrain.combat_width + terrain.combat_support_width × (n_directions − 1))
              × (1 + Σ tactic combat_width modifiers)
join rule (inference): divisions join while used_width ≤ total_width × 1.33
   (defines COMBAT_OVER_WIDTH_PENALTY=-1/%, _MAX=-0.33; stacking COMBAT_STACKING_START=5,
    _EXTRA=3/dir, _PENALTY=-0.02)
```
Honesty: no stored width to validate; ×1.33 is a penalty-cap inference; effective division
width needs country/unit width modifiers. Emit as formula-grade, not measured. Output `combat_width`.

### 3. PHASE (CONFIRMED — three derivables)
(a) Tactics phase: current tactic's `phase`/`display_phase` (§2 pointer→key→`combat_tactics.txt`);
    emit a non-default phase ONLY when both sides' tactics agree on the same phase, else `no_phase`.
(b) Combat age: `u32[body+0x30]`(duration) == `u32[body+0x34]`(day) == per-side `num_org_losses`
    (@side+0x1AC) — 70/70; unit = HOURS since start; UI day = floor(hours/24)+1.
(c) Per-side progress bar: `i32[side+0x418] / 100000` — 40/40 exact vs melt per-side `progress`.
Day/night (`BASE_NIGHT_ATTACK_PENALTY=-0.5`) is a clock+longitude function NOT reverse-engineered
— any night flag is an approximation, grade so. Output `combat_phase`, `combat_age_hours`,
`side_progress`.

### 4. PLANNING (CONFIRMED — per-division stored scalar)
```
planning_bonus = i32 @ CDivision + 0x460   // ×100000; melt `bonus`; 609/609 + 1280 absent-zero
dig_in         = i32 @ CDivision + 0x448   // 1494/1494; dig_in_cap = i32 @ +0x450 (1876/1876)
```
Defines: `PLANNING_MAX=0.3`(+tech), gain 0.02/day, decay 0.01/day (0.03 after manual order),
`DIG_IN_FACTOR=0.02`/level. Apply planning ATTACKER-side only; do not conflate with naval-invasion
prep. Output `planning_bonus` per participating division.

### 5. DIRECTION (CONFIRMED inputs; adjacency from static map data)
```
division_location = u32[ u64[CDivision+0x1E0] + 0xA4 ]   // CProvince id; 1888/1888 == melt location
```
Adjacency from `map/provinces.bmp` 4-adjacency + `definition.csv` RGB→id + `map/adjacencies.csv`
(straits/canals). `n_directions` = distinct current provinces of attacker participant divisions
(front + reserves); each source_province→front_province edge = one direction. Validated live:
35/35 defenders at front, 34/34 attacker sources pixel-adjacent, n_directions 1..4. Fallback to
order/source metadata when a participant's location is not adjacent (naval invasion / paradrop /
retreat / overrun). Output `n_directions`, per-direction source edges.

### New cracks (offset table)
| field | offset | type | validation |
|---|---|---|---|
| CLandCombat.terrain | body+0x38 | CTerrainType* (key @+0x18) | 20/20 bijective; melt==csv 70/70 |
| CCombatSide.tactic | side+0x110 | CCombatTactic* (key @+0x10, idx @+0x98) | 40/40, 11 tactics |
| CCombatSide.progress | side+0x418 | i32 ×100000 | 40/40 exact |
| CDivision.location | +0x1E0 | CProvince* (id @+0xA4) | 1888/1888 |
| CDivision.dig_in | +0x448 | i32 ×100000 | 1494/1494 + 395 absent-zero |
| CDivision.dig_in_cap | +0x450 | i32 ×100000 | 1876/1876 + 13 absent-zero |
| CDivision.planning_bonus | +0x460 | i32 ×100000 | 609/609 + 1280 absent-zero |
| CProvince vtable | RVA 0x295AF68 | (correction) | RTTI + 1349 objects |

---

## Cross-cutting notes for the replay implementer
- All ×100000 fixed-point: divide by 100000 for the human value; use truncating (round-toward-
  zero) division where the engine does (A1).
- All game-file lookups (defines, terrain, tactics, static modifiers, localisation, map data,
  ideas, history/states) MUST resolve through `mod_resolver` keyed by the recording modset.
- These five are DISPLAY/ANALYTICS recomputes. Authoritative STATE (capitulation bool/date,
  stored planning/dig-in scalars, combat progress) is read, not derived, where a stored field
  exists.
- Grade each emitted value by its evidence: A1/A3/A5 = derived-with-residuals; A6 =
  heuristic+badge (never authoritative cause); A7 terrain/planning/direction/progress =
  oracle-confirmed, A7 width = formula-grade (no oracle), A7 night = approximation.
