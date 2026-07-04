# Engineering queue — all remaining no-user integration work (2026-07-04)

Memory MAPPING is done (every no-user category cracked/derived/dispositioned).
This is the ENGINEERING phase: wire the cracked domains into the .battleplan
container, fix the one structural blocker, and close the replay-layer + infra
items. All no-user (validated on the paused D04 save). Worked ONE packet at a
time by a single Fable agent, gated by the coordinator per packet.

> **SCOPE UPDATE (2026-07-04): PURE WIRING ONLY.** All reverse-engineering was
> moved to a separate crack agent (`CRACK_LIST_FINAL_2026-07-04.md`). Do NOT crack
> anything here — only wire already-verified offsets. Specifically:
> - **E2**: wire the validated CCombatSide fields only; the land-combat TACTIC
>   crack (table decode) is the crack agent's (B1) — skip it here.
> - **E7**: do the DECOUPLE half only (give operations/diplo/intel their own
>   independent emitting records with own source-availability bits, so they emit
>   regardless of the d53 base). Do NOT attempt to crack/validate the d53 base
>   relation root — that is the crack agent's (B4).
> - The active-war casualty magnitude semantic (E1 note) and breakthrough state
>   are the crack agent's (B3/B2) — wire only the validated capitulation/war
>   fields; leave breakthrough out.

## THE RHYTHM (mandatory — do NOT deviate)
For each queue item: BUILD it (lane emit + collector read/schedule + recorder
decode + world_fold + tolerance + tests) → run `cargo test --workspace` green →
run the paused-capture proof (rows>0, no worker_panic/source_unavailable,
`pack_replay verify` OK; cross-check the folded values vs the melt) → then STOP
UNCOMMITTED and report "E# (name) ready for gate — <what changed, files, test
counts, capture rows, melt cross-check>". Do NOT run your own Codex. Do NOT
commit. The coordinator runs the independent adversarial Codex gate + commits,
then tells you to proceed. This rhythm caught two real bugs across the last 6
packets (ace roster-wipe, operations staleness) — keep it.

## TEMPLATES (study these committed packets)
- Trailer-on-existing-record: PLIC (licensing c6e969d39), PLCF (projects
  05e9a0af1), OPIN (operations e1800b3ef). Strict TLV: magic + version +
  exact-end length; absent trailer → null (not empty).
- Row-growth (fixed→+field): disrupted_supply (6c495fd92) — row-len lockstep
  across collector/recorder/world_fold, tail-append, presence flag on a free bit,
  prove no stale old-len framing.
- Variable-length tri-state per row: cosmetic_tag (5f7c75666).
- Discovery-cache heap scan (never per-frame sync): operations (e1800b3ef) rides
  the d53 cache; naval 49/50 has its own. Background refresh + generation counter
  + join-on-Drop + stale-majority, separate staleness attribution per population.
- Fold onto §7 combat snapshots: land_combats_live (D21) / naval (D49/50).

## QUEUE (priority order)

### E1 — Capitulation / war state  [HIGHEST replay value — political map]
Offsets in `FINAL_SWEEP_RESULTS_2026-07-04.md` §T1 (orchestrator-verified 15/15).
`D = u64[CCountry+0xF88]`: capitulated u8 @D+0x2E0, capitulated_date @D+0x2F0,
last_surrender_date @D+0x2B8, faction_join_date @D+0x2D0, legitimacy i32×1e5
@D+0x1B0, exile_army_leaders @D+0x1B8. War pool (D53 war rows): tags @+0x00/+0x04,
date @+0x08, casualties @+0x40/+0x48. Fold per-country capitulation into §3/§4
country state; wars into §7b `diplomacy_world.wars`. surrender_progress=DERIVED
(skip), peace_conference=absent (skip). NOTE the war-pool rows may ride the d53
family — if so, keep them independently foldable (see E7) or on a healthy domain.

### E2 — Land combat detail  [50 live battles]
`FINAL_SWEEP_RESULTS` §T2. CCombatSide (vt 0x29913C0): committed @+0xD4, reserve
@+0xEC, units @+0x2C; i64×1e5 damage anti_air @+0x138, air str/org @+0x148/+0x150,
ground str/org @+0x158/+0x160, prevented str/org @+0x168/+0x170; air_kills @+0x140,
org_loss_summary_index @+0x1A8, num_org_losses @+0x1AC (100/100 exact). Extend
domain 55 `land_combat_detail_events_live`; fold onto §7 combat snapshots. tactic
= PARTIAL (skip or badge unresolved); width/terrain/phase = DERIVED (skip).

### E3 — Naval combat detail  [4 live battles]
`FINAL_SWEEP_RESULTS` §T3. CNavalCombatant (vt 0x29CD298), i64×1e5, 8/8:
positioning @+0x138, dominance_bonus @+0x148, anti_air @+0x150,
total_initial_strength @+0x158, total_damage_dealt @+0x160. Fold onto the naval
combat snapshots (D49/50). Per-sink killer = NEEDS-USER (skip).

### E4 — Strategic redeploy + division losses/manpower  [D20 extension]
`WALL_CRACK_RESULTS_2026-07-04.md`. Row-growth on D20 (template: disrupted_supply):
strategic_redeploy bool `CDivision+0x29F` (220 live == melt), current manpower
`CDivision+0x3D0→u32@+0x4` (2359/2359 exact), cumulative losses `CDivision+0x668`
i32 (value-validated). Presence flags on free bits, lockstep row-len, cross-check
vs melt (SR count 220, manpower per-division).

### E5 — Combat attribution matrix  [country granularity]
`WALL_CRACK_RESULTS`. CStatsObserver (vt 0x2946F68) → SDamageDone @+0x38 (vt
0x2946EC8): 0x20-stride {dealer db_id @+0x08, victim db_id @+0x10, damage @+0x18}.
Heap-scan via a discovery-cache (NOT per-frame sync). Fold a dealer→victim→damage
matrix onto combat/war snapshots. Loss stores CSunkConvoyInfo (0x295D008)/
CSunkShipInfo (0x2941608) optional. Per-entity killer label = NEEDS-USER (skip).

### E6 — Market per-country attribution  [unblocks market stockpile]
`CRACK_QUEUE_RESULTS_2026-07-04.md` §C1 (verified 70/70). Chain: entry=u64[CCountry
+0xFB8], wrapper=u64[entry], pool=wrapper+0x38 (vt 0x27330A0); invariants
u64[entry+0x40]==CCountry, u32[pool-0x18]==owner db_id. Decode pool entries
(begin @+0x20, packed @+0x28 hi32=count, stride16 {variant_ptr, i64 amt/1e5},
type-70 variant id @+0x0C). Wire per-country market_stockpile into the market
domain with a fail-closed content cross-check vs the serialized market_stockpile.

### E7 — d53 base root: validate OR decouple  [unblocks a whole category]
`task #33`. The d53 base extract is SOURCE_UNAVAILABLE+PARTIAL on 870ebf → the d53
record fails closed, so operations (e1800b3ef, blocked-additive), diplo queue (E8)
and intel (E9) can't emit. Do ONE of: (a) find + validate the d53 base relation/
opinion live root on 870ebf so the record emits (a bounded crack from the paused
save — the operatives discovery already works, so part of d53 is reachable), OR
(b) give operations/diplo/intel their OWN independent compact records with their
own source-availability bits (decouple from the d53 base). Prefer (a) if the base
root falls in a bounded probe; else (b). This is investigation + wiring.

### E8 — Diplo action queue  [after E7]
`CRACK_QUEUE_RESULTS` §C3 (verified 5/5 recipients, 7/7 actions). queue=u64[CCountry
+0xFF8], data @+0x08, size=u32[queue+0x10]&0xffffffff; action vt 0x2A209A0, handle
@+0x08 (type==4713), body=u64[action+0x18]: actor @+0x14, recipient @+0x1C, date
@+0x30 (the game hours +5000y epoch). Emit {recipient, action_id, actor, date} on an
emitting record (per E7's decision). Fold into §7b diplomacy.

### E9 — Intel network graph  [new domain, after E7]
`CRACK_QUEUE_RESULTS` §C4 (verified 66/66, ENG-14/SOV-43). CStrategicOperative (vt
RVA 0x298B448) +0x10 → CCountryIntelNetwork*[] (vt 0x298AB48, 66) +0x18 size; per
net: target tag @+0x08, graph @+0x18 → vertices @G+0x40, state array @G+0x38
(16B stride, state_id @+0). Discovery-cache heap scan. Fold a spy-network domain
(per owner-operative → target → covered-state set) for the intel map mode.

### E10 — N2 naval supremacy port  [replay R6]
`REPLAY_REQUESTS_RECONCILIATION` N2. Python-SOLVED 33/33 in
`extractor/memory_reader/naval_air` (region obj MODULE+0x29468B8 →+0x58 id, +0xE8
container MODULE+0x295DF30 → entries MODULE+0x295DEE0 stride 0x50, per-TAG dominance
{current,target,base_target,previous,decline_from,individual_ratio}). Port to a
native compact domain; fold per strategic-region supremacy (the sea-zone borders).

### E11 — N4 localisation static bundles  [replay R3; shipped-file work]
`REPLAY_REQUESTS_RECONCILIATION` N4. Extend the static-artifact family (like
idea_tech_loc_xref) with focus-name, state/VP-name, country-name (incl. cosmetic
overrides) localisation joins, mod-over-vanilla, keyed by recording mod identity.
Shipped game-file parsing → hashed static_artifact packed by the recorder. No live
process needed.

### E12 — N5 naval per-hour fold verify  [replay R9.1]
`REPLAY_REQUESTS_RECONCILIATION` N5. Confirm the folded naval rows give the replay
{task_force_id, owner, location, ship_count, dominant_hull_class} per hour from
D56 + D29; if not, aggregate D29 ship rows onto D56 task-force rows in world_fold.

### E13 — Systemic: measured-country identity per frame  [infra]
`task #31` (Codex Q4 from the projects gate). The capture resolves ~356 of ~437
countries per frame (systemic country-roots bound, all country-rooted domains).
Expose the measured country count/identity per frame so downstream distinguishes
"country absent = no data" from "country absent = not resolved this frame". A
capture-infra field, benefits every country-rooted domain.

### E14 — Throughput measurement  [infra; no-user part only]
Measure per-reader per-frame time on the paused capture with everything wired;
identify which readers exceed the ~100ms/hour speed-5 budget and flag them for the
discovery-cache treatment. (The full zero-skip-at-speed-5 proof needs a user
unpause window — do the measurement + candidate list here, note the unpause part.)

## ENVIRONMENT + INVARIANTS
- Worktree, PID 28044, SHA gate `870ebf1f...`, paused D04, melt oracle
  `runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`.
- One shared-hot-file packet at a time (collector/src/lib.rs, recorder_host/src/
  {main,world_fold}.rs, compact_contract/src/lib.rs). Stage NOTHING — the
  coordinator commits. Never `git add -A`. Discovery agents' gitignored runs/ files
  stay out of commits.
- Fail closed both directions (absent != 0/empty; presence flags/tri-state). Never
  revert the D47 appointed-advisor disable or UNIT_LEADER_TRAIT_VTABLE_RVA=0x292F160.
- Never touch `replay_interface/` (schema deltas go via the reconciliation doc).
