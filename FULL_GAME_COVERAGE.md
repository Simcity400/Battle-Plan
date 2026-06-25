# Full-game coverage matrix — "the whole game, no gaps"

> Goal: the replay reproduces **everything** the game tracks/shows, at 100% fidelity.
> This is the master checklist: every the game system → capture status, data source, and the
> replay window that displays it. **No domain is "abandoned"** — anything not yet captured
> is either an active crack/parse, a planned step, or one of the three documented
> hard-impossibles (§Z). Legend: ✓ done · ⏳ agent live · 📋 planned · ⛔ hard-impossible.
> Source: **S**=save oracle · **M**=live memory · **D**=derived · **A**=game asset.

## 1. Country — economy & industry
| system | status | source | replay window |
|---|---|---|---|
| factories civ/mil/dock (+offmap, under-construction) | ✓ | S | topbar, production |
| construction queue | ✓ | S | production / construction |
| resources produced/exported/available | ✓ | S | logistics / trade |
| fuel (current/max) | ✓ | S(+M\* live) | topbar, logistics |
| trade deals / routes | 📋 | S | trade map mode + logistics |
| gross IC output by equipment (delta) | ✓/D | S/D | stats (Layer 1) |

## 2. Country — military & equipment
| system | status | source | window |
|---|---|---|---|
| equipment stockpile, convoys | ✓ | S | military / logistics |
| manpower (available/deployed/army·navy·air), conscription law | ✓ | S | recruit & deploy |
| army/navy/air XP, num_ships, ships-by-hull | ✓/📋 | S/D | military |
| casualties (killed/captured) → K/D | ✓ | S/D | military, stats |
| **designs**: division templates (full regiments), equipment/ship/plane variants, MIOs | ⏳ | S (parse_designs) | the designers (division/equipment/ship/plane), MIO panel |

## 3. Country — politics, research, focus, decisions
| system | status | source | window |
|---|---|---|---|
| stability / war support / PP / CP | ✓ | M | topbar, politics |
| ruling party, party popularities, govt | ✓ | S | politics |
| laws (econ/trade/conscription), ideas/spirits, cabinet/advisors | ✓ | S | politics |
| research: tech, in-progress (+live progress), slots, bonuses | ✓(+M\*) | S/M\* | research queue |
| national focus: tree, completed, current+progress | ✓ | S | focus tree |
| decisions: active/timed/missions | ✓ | S | decisions |

## 4. Country — diplomacy & intelligence
| system | status | source | window |
|---|---|---|---|
| wars, war goals, war score | ✓ | S | diplomacy, war overview |
| factions, subjects/autonomy, guarantees, NAP, embargoes, opinion | ✓ | S | diplomacy, diplomatic map mode |
| lend-lease, volunteers, attaches, expeditionary | ✓ | S | diplomacy |
| **intel agency, operatives (status/missions/traits), operations, decryption, peace conference** | ⏳ | S (parse_intel) | intelligence, operations |

## 5. Commanders & characters
| system | status | source | window |
|---|---|---|---|
| **country leader, generals/FM/admirals (skill/traits/level/xp), advisors, theorists, high command** | ⏳ | S (parse_characters) | character/recruit screens, unit commander panels |
| **commander → unit assignments** | ⏳ | S/D | OOB, unit panels |
| character **portraits** | 📋 | A (portrait extraction) | everywhere a character appears |

## 6. Order of battle & plans
| system | status | source | window |
|---|---|---|---|
| **army groups / armies / corps → divisions** | ⏳ | S (parse_oob) | OOB tree, map |
| **navy: fleets → task forces → ships** | ⏳ | S (parse_oob) | navy screen, OOB |
| **air: wings, missions, airfields** | ⏳ | S (parse_oob) + M (positions) | air screen, OOB |
| **battle plans / fronts / theaters / orders** | ⏳ | S(persisted) + M(live exec) | map (plan arrows/fronts) |

## 7. Territory — states, provinces, map
| system | status | source | window / map mode |
|---|---|---|---|
| state owner/controller, cores/claims | ✓ | M/S | political + occupation modes |
| state buildings, VP, resources, manpower, category, occupation/resistance | ✓ | S(+M\*) | state view; building/VP/resource/manpower modes |
| province controller | ✓ | M/S/D | controller mode |
| **supply (full per-province), railway level+usage** | ⏳ | M (logistics_intel) +S | supply / railway modes |
| **intel level on others** | ⏳ | M (logistics_intel) +S | intel map mode |
| front lines, strategic regions, terrain | ✓/📋 | D / S-static / map_pipeline | front-line / region / terrain modes |

## 8. Units & combat (the battlefield)
| system | status | source | window |
|---|---|---|---|
| divisions: enumerate, owner, location, template, org, strength, equipment | ✓ | M | unit counters, division view |
| **division live-state: movement (arrows), in-combat, encircled, supply, experience** | ⏳ | M (division_live) | map counters/arrows, division view |
| **land combat: enumerate, location, attackers, defenders** | ✓ | M | combat bubbles, combat view |
| land combat: width/terrain/progress/casualties | ✓/M\* | M/S | combat view |
| **naval & air combat** | ⏳ | M (naval_air) | combat bubbles |
| **naval region control, air superiority, fleet/wing positions** | ⏳ | M (naval_air) | naval / air map modes |

## 9. Environment
| system | status | source | window |
|---|---|---|---|
| **weather (precip/storm/mud/frozen)** | ⏳ | M (weather) | weather map mode |
| terrain (static), rivers, adjacency | 📋 | map_pipeline static | terrain mode |

## 10. Meta / game / timeline
| system | status | source | window |
|---|---|---|---|
| players (is_human) & which TAG each played | ✓ | S/M | lobby/overview |
| game rules / scenario / mod | 📋 | S/mod | overview |
| event timeline (29 types) + markers | ✓ | D/marker | event ticker |
| statistics ledger (production/casualties/IC over time) | ✓/📋 | D (series) | stats (Layer 1), ledger |
| score / victory conditions | 📋 | S/D | overview |

## 11. Identity / cross-cutting (cracked enablers)
| tag-db-id→TAG ✓M · live province control ✓M · country running_index↔tag-db-id (economy→TAG) ⏳M · build/exe gate ✓ · source-fingerprint preflight ✓ |

---

## Orchestration state (2026-06-24)
**Agents live:** fix-agent (CODE_REVIEW); asset extraction (#16); economy→TAG bridge; naval/air;
division-live; weather; supply/rail/intel; commanders+intel (save); designs (save); OOB+plans (save).
**Done:** save oracle + 7 deep parsers; map pipeline/viewer; live cracks (owner/control/divisions/
tag-table/province/land-combat); replay backbone (schema+store+diff) + renderer + validator +
`build_replay.py`; Layer-1 stats UI; `replay_game/PLAN.md` (window+map-mode spec).

## Remaining orchestrator tasks (me, as inputs land)
1. **Extend `replay/SCHEMA.md` + `schema.py`** with the new domains: `commanders`, `designs` (templates/
   variants/ships/planes/MIO), `order_of_battle` (armies/fleets/wings/plans), `intel` (agency/ops),
   `naval_air` (region control/air superiority/positions), `weather`, division live-state, supply/rail/intel.
   Each schema-present, null until its reader/parser feeds it (no fabrication).
2. **Wire** each new parser/reader output into `store.py` ingestion + add diff/event types where they apply.
3. **Portrait asset pass** (extend `game_assets/` after #16): character portraits + leader/advisor art.
4. **Extend `replay_game/PLAN.md`** with the full window set (commanders, designers, OOB, navy/air, intel
   ops, ledger) — every game window has a home.
5. Then launch the **Layer-2 game-authentic UI build (#17)** against the complete PLAN + assets.

## Z. Hard-impossible (DESIGN §4 — never faked, documented as limits, not gaps)
- Steam / player **real names** (humans are TAG + `is_human`).
- **Sub-sample micro-events** — a combat or one-shot click that begins *and* ends between two samples.
- **Pure-UI-only display numbers** that exist only in the live renderer and are never stored/computed in state.
