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
| Combat attribution (country granularity) | `CStatsObserver→SDamageDone` matrix | **Verified: 113 CStatsObserver live (exact); dealer→victim→damage matrix read live, SOV→GER=72690600 reproduced, 193+ valid country-pair records.** Finest per-entity killer = NEEDS-USER (1-event). |

**Combat attribution (item 8) — country-granularity dealer→victim→damage matrix CRACKED; finest per-entity killer NEEDS-USER.**
A deeper follow-up pass found the actual attribution MATRIX (not just loss stores),
and I INDEPENDENTLY VERIFIED it live:
- **`NCombatLog::CStatsObserver` (vt RVA `0x2946F68`, 113 live — I reconfirmed 113
  exactly) → `SDamageDone` @+0x38 (vt `0x2946EC8`)**: a 0x20-stride who→whom→damage
  matrix `{+0x08 dealer tag_db_id, +0x10 victim tag_db_id, +0x18 damage}`. I read it
  live: 193+ records with BOTH endpoints valid country db_ids, and **reproduced the
  exact value SOV→GER = 72690600** (plus GER→SOV, ITA→SOV, TAN→GER, ...). This gives
  per-country damage attribution directly, validated structurally (both-endpoints-country
  rules out pool garbage). Backed by `NCombatLog::CManager` (vt `0x29470F8`, country-indexed
  0x30-stride: +0x20 owning tag_db_id, +0x28 log data).
- Loss stores also persist on the paused save: `CSunkConvoyInfo` vt `0x295D008` (441,
  independently reconfirmed) with 3 country slots + timestamp + route name; `CSunkShipInfo`
  vt `0x2941608` (10416, sink-date CGameDate @+0x50/+0x60 + country tags);
  `SAirWingCombatData` vt `0x2A099E0` (12889). None serialized in the melt → runtime-only.
- Only per-RESOLVE side-objects (`CNavalCombatResults/Side`) are empty (transient; redundant).
- **NEEDS-USER residual (fine granularity only, NOT the old live-combat blocker):** the
  finest per-convoy / per-sunk-event / individual-per-wing KILLER label is not separable on
  a paused frame (`CSunkConvoyInfo` past +0x18 is heap spill; `SRegionalConvoyData` is a
  per-region timeseries). Closes via ONE controlled before/after event (known A sinks known
  B) or a disasm of the sink-recording writer. **Country-granularity attribution is wireable
  now (SDamageDone matrix); only the per-entity killer label needs the 1-event fixture.**

## RESOLVED already
- **Garrison child disambiguation** — closed by the wave-3 garrison crack (+0x88 manpower_need 280/280, +0xF0 strength_ratio 288/288, +0xF8 garrison_required 282/282; clean diagonal). The old B6 unpause fixture is unnecessary.

## DERIVED — replay reconstructs; OFF the user-fixture floor
- **Exact province supply** — CONFIRMED-DERIVED (Codex + deeper disasm). The "Supply
  Remaining in Province" tooltip (loc key `SUPPLY_CAP_AVAILABLE`, xref rva 0x1623899) is a
  DRAW-TIME aggregation, not a stored scalar: formatter fn rva 0x1623536 reads
  CGameState=`u64[module+0x3316E90]` → CSupplySystem=`u64[CGameState+0x3D8]`, then
  `sub_0x1939740` SUMS per-source-node supply over the province's in-range source list
  (`[province_ctx+0x60]` begin / `+0x6c` count, each `{u32 node_id, u8 type}`), each node
  looked up in `[CSupplySystem+0x108]`, per-node supply `[node+0x80]+0x138`, type-weighted
  (×50000) in /100000 fixed-point. The melt has zero serialized remaining-supply field.
  Replay recomputes from the (live-cached) CSupplySystem per-province inputs — no single
  offset exists. (Exact-decimal QA would still want a tooltip capture, but the DERIVED
  verdict is structurally proven.)
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
- Combat-attribution country-granularity matrix (`CStatsObserver→SDamageDone` dealer→victim→damage; loss stores CSunkConvoyInfo/CSunkShipInfo/SAirWingCombatData) — a new domain, wireable now at country granularity; the finest per-entity killer label carries a NEEDS-USER (1-event) badge.
