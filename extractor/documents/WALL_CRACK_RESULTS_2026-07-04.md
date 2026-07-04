# Wall-crack results — residual fixture wall, determined no-user pass (2026-07-04)

Result of the determined structural attempt on every item previously filed
"needs the user" (`WALL_CRACK_ATTEMPT_2026-07-04.md`). Run by a Fable discovery
agent, key claims INDEPENDENTLY re-verified by the orchestrator against the melt
oracle + live process. Per-item evidence: gitignored `runs/wallcrack_<item>/EVIDENCE.md`.

**Bottom line: the irreducible user-fixture floor shrank from ~12 to 4.**
3 CRACKED, 1 already-RESOLVED, 4 DERIVED (replay reconstructs — off the floor),
4 genuinely NEEDS-USER (of which 1 needs a 2nd player; the other 3 are ~10s SP fixtures).

## CRACKED (no user) — orchestrator-verified

| item | offset / chain | independent verification |
|---|---|---|
| Strategic-redeploy mode | `CDivision+0x29F` u8 bool | **Verified: melt `strategic_redeployment=yes` count = 220 == live count 220** at +0x29F; agent 185/185 recall, 0 FP on 2359 id-validated divisions. |
| Division current manpower | `CDivision+0x3D0 -> u32@+0x4` | 2359/2359 exact vs melt `army_manpower_value` (this pass + prior wave-2). |
| Division cumulative losses | `CDivision+0x668` i32 | Value-credible cumulative counter (0..131347), same-tick validated; dynamics two-poll optional (not a value blocker). |
| Combat attribution CONTAINERS | see below | **Verified: independent heap scan counted exactly 441 `CSunkConvoyInfo` (RTTI confirmed); sampled instance fields [GER,USA,ROM] match.** |

**Combat attribution (item 8) — container-level CRACKED, attribution-label UNVALIDATED.**
Persistent, populated stores exist on the PAUSED save (no live combat needed to
find/read them) — this removes the old "needs live combat" blocker:
- `CSunkConvoyInfo` vt RVA `0x295D008` — **441 instances (I independently reconfirmed 441)**; 3 country-db_id slots @+0x0C/+0x10/+0x14 (sampled = GER/USA/ROM), +0x18 timestamp, +0x20 region/route name.
- `CSunkShipInfo` vt `0x2941608` — 10416; `SAirWingCombatData` vt `0x2A099E0` — 12889 (+0x10/+0x14 enemy/owner db_ids).
- `NCombatLog::CManpowerLoss/CEquipmentLoss/CStatsObserver`, `CCombatHistoryEntry` (236) — combat-log breakdowns.
- Only the per-RESOLVE side-objects (`CNavalCombatResults/Side`) are empty (transient during a combat tick; redundant with the persistent history).
- **HONEST RESIDUAL (not live-combat-blocked):** which of the 3 country slots is attacker-credit vs victim vs trade-partner is NOT provable from this save (the melt's `sunk_convoys=N` is a per-trade-route stat, not a per-country attacker total — no oracle). Closes via (a) disasm of the sink-recording writer, or (b) ONE controlled before/after event. Codex: "NEEDS-MORE ... CRACKED requires proving attacker credit, not just loss ownership + contextual enemy ids." So: **containers wireable now; attribution label carries an `unvalidated` badge until the writer-disasm or 1-event confirm.**

## RESOLVED already
- **Garrison child disambiguation** — closed by the wave-3 garrison crack (+0x88 manpower_need 280/280, +0xF0 strength_ratio 288/288, +0xF8 garrison_required 282/282; clean diagonal). The old B6 unpause fixture is unnecessary.

## DERIVED — replay reconstructs; OFF the user-fixture floor
- **Exact province supply** — CONFIRMED-DERIVED (Codex). The tooltip formatter (`SUPPLY_FLOW_PROVINCE_ACCUMULATED`, RVA 0x1623536) calls a `CSupplySystem` method (`[CGameState+0x3d8]`, rip-verified to CGameState 0x3316E90) per supply-route hop and SUMS contributions (×100000). Replay recomputes from CSupplySystem per-province inputs over the supply graph.
- **Resource flow (produced/imported/exported/used)** — no per-country array (1550-object 2-level BFS negative); lives in `NInternationalMarket` (global) + `STemporaryResourceData@CState` (RVA 0x2920670). produced=Σ state outputs; import/export=international-market rows. A save oracle now exists (melt resources + trade) → no user needed. (Codex NEEDS-MORE = deeper RE, not a user action.)
- **Attrition reason** — melt division `attrition={ }` is empty; no serialized reason enum exists. Reason is recomputed from the cracked scalars (supply_ratio +0x600, fuel +0x610, disrupted +0xA0) + province weather/terrain. No user can reveal a field that isn't stored.
- **Offmap factory total** — DERIVED-candidate: no standalone offmap scalar found; serialized inputs (per-state buildings + production-line `active_factories`) reconstruct it. Codex refuted a *clean* DERIVED (a stored counter isn't ruled out) — but this is a memory-RE task, NOT a user delta (old B1 unneeded).

## NEEDS-USER — the true irreducible floor (4)
| item | why irreducible | minimal user action |
|---|---|---|
| Hub motorization level | all live hubs read one constant level; melt node-motorization all 0 — no discriminating oracle | set two hubs to DIFFERENT levels (e.g. 1 and 3), pause (~10s) |
| Focus-paused bit | all 113 melt focuses `paused=no`; an always-false bool can't be validated | pause one national focus, then pause the game (~10s) |
| Convoy/port/disrupted-supply PRESSURE | disrupted SCALAR cracked (CDivision+0xA0), but pressure records at `CCountrySupplySystem+0x1E0` (0x70-stride) all read zero — save has zero disrupted anchors | create one disrupted situation (blockaded island / cut pocket), save+pause |
| MP chat/messages + hotjoin/tag-switch (D57) | only populate in multiplayer; no SP container | same-build 2-player session w/ controlled join + unique chat text |

## Optional confirmations (NOT blockers)
- Division-losses DYNAMICS two-poll (item 9) — confirms monotonicity under live combat; the value is already correct.
- Combat attacker-vs-victim field LABEL (item 8) — writer disasm or 1 event.

## Newly wireable from this pass (beyond the current wiring queue)
- Strategic-redeploy bit (CDivision+0x29F) — a D20 division-row bit (same pattern as the disrupted_supply packet).
- Division manpower/losses (CDivision+0x3D0->+4, +0x668) — D20 fields.
- Combat-attribution containers (sunk convoy/ship, air-wing combat) — a new domain, with the attribution-label `unvalidated` badge until the 1-event/writer-disasm confirm.
