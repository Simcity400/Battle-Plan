# R0 TRACEABILITY MATRIX v0 — what "nothing missed" MEANS (plan v2 Wave R; G4/plan-review finding 7)

Demand side of record: `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md`
(self-described complete list; the replay layer keeps it current; last full review
2026-07-09 with its own reconciliation table §"Coverage status"). Supply side: the
extractor's 55-domain compact roster + static catalogs. THIS file is the extractor-side
join of the two, and the release rule from the plan review binds: F2 validates the dress-
rehearsal artifact against the FROZEN version of the requirements + this matrix; drift is
a defect or an explicit USER scope decision — never a doc edit to match the artifact.

## A. Demand items fully mapped to extractor supply (extractor-side LANDED per the
## replay side's own reconciliation — F2 asserts rows exist in the artifact)
| req | surface | extractor supply |
|---|---|---|
| 1 | hourly decoded snapshots in container | recorder deferred fold (R1) |
| 2 | owners/controllers | d22 map_control (rides 1) |
| 3 (core) | divisions + land combats | d20/d26 divisions, d21 land_combats, d55 detail events |
| 5 | naval supremacy per region | d56 naval_missions_status |
| 6 (core) | weather stream | d24 weather_live |
| 7 | cosmetic tags | D18 politics stream |
| 7b | ruling party/ideology | d14 politics (§3.3 shape) |
| 8 (air) | air presence | d31 air_region_activity |
| 8b | pair relations/factions/capitals | d53 relation flags (+fold) |
| 10 | building levels + damage | d48 construction |

## B. Demand items with OPEN EXTRACTOR WORK (the true "nothing missed" gap list —
## each needs: implement + gate + live validation, OR an explicit USER decision to
## ship the release without it, recorded here)
| req | what is missing extractor-side | size guess |
|---|---|---|
| 3 (ext) | battle-bubble display fields: progress/bombardment/air-superiority (C3) | S–M |
| 4 (ext) | session speed/pause timeline + DLC display-names | S |
| 6 (ext) | SnowMud accumulators (parity confound already documented) | S–M |
| 8 (naval) | naval PRESENCE rows + active naval combats (C3 naval bubbles) | M |
| 9 | localisation static-artifact ships in catalog contract (verify) | S (verify) |
| 9b (ext) | extended topbar scalar set + outliner | M |
| 13 | **per-country FoW/visibility + spotted units per hour (PROMOTED REQUIRED — observer POV is core UX)** | L (needs RE + capture pairs B10/CAP10) |
| 14 | fired-event stream {event_id, target_tag, chosen_option_index, fire_date} (A18) | S–M (on-change, tiny volume) |
| 15 | deep-panel folds into hourly snapshots (A22–A33: research/focus/politics-detail/production/templates/intel/trade/occupation/leaders/power-balance/naval-air-combat/scripted-GUI-vars) | L (many are existing domains needing FOLD wiring, not new readers) |
| 11 | rail/supply dynamic state | deferred by replay side |
| 12 | battle-plan arrows | deferred by replay side ("do not build yet") |

## C. Certification asks (travel via EXTRACTION_INTERFACE_REQUESTS.md → F4; a one-way
## note does NOT close them)
1. Replay layer certifies that REPLAY_DATA_REQUIREMENTS.md covers EVERY surface of the
   standing everything-in-game ruling (all map modes, every UI panel incl. DLC/scripted,
   particles/arrows triggers) — i.e. that "not in this list = shipped-files/GPU-capture"
   holds for the full scope, not just the reviewed surfaces.
2. Replay layer accepts the disposition semantics (C1) + modset binding (M2) contract.
3. Freeze SHA of the requirements doc recorded at F2 time; both sides sign the same SHA.

## D. Release consequence (binds §7 DONE)
The B-list is release-gating BY DEFAULT: each row ends CLOSED (landed+gated+validated) or
USER-DEFERRED (explicit decision recorded in this file). d69's populated-restore (active-
count RE) and d17 native capture join the same discipline via their queue entries.

## E. Status reconciliation — 2026-07-12 (v0 → v1)
Closures this session (move OUT of any "populate/restore" debt; the B-list rows above are
UNCHANGED — they are genuine remaining capture scope):
- **d69 tech-sharing groups: CLOSED (populated + oracle-validated).** The chronic-PARTIAL
  (fixed-9 OOB walk) is fixed by the [0..size@CGameState+0x3AC) bound (commit eef77a0e7).
  MELT-ORACLE PASSED on D04_1944 (live 9 groups / 34 members == parse_world melt), and the
  <9 improvement live-proven (frozen 4 groups CLEAN, was PARTIAL). Only owed: the Codex
  re-gate formality (blocked by a service outage; auto-fires on recovery). Feeds req-8b
  research-sharing display.
- **d47 characters blocker-1: CLOSED.** The latent persistent-ambiguity defect that could
  zero the whole domain is fixed (quarantine + `ambiguous_root_rows`, commit a4994ab95),
  live-clean on the frozen save.
- **P4 pace verdict + P1b:** RESOLVED (not R0 rows, but they were "open experiments").
The B-list remains the authoritative "nothing-missed" gap list: reqs 3ext/4ext/6ext/8naval/
9b/13(FoW)/14(events)/15(deep-panel folds) are the real remaining extractor capture scope.
The largest is #15 (A22–A33 fold wiring) and #13 (per-country FoW, needs RE + capture pairs).
Mod-robustness of every landed domain is separately gated by R2 (bounds audit) + R3 (cross-
modset). Nothing here is claimed CLOSED without a live/oracle proof recorded in the queue.
