# Battle Plan / Replay — design (v3)

> **v2 supersedes the original log-channel design.** The original DESIGN treated the
> in-game mod's `log=` channel as the primary data source and listed divisions,
> division counts, tech, naval tonnage, etc. as "not possible." That was true **only
> for the log channel**. Since then the data architecture first pivoted to a
> **host-side external memory reader + rakaly-melted save oracle**, and live RE has
> *proven* most of that old "impossible" list is reachable. The current product
> decision is stricter: **runtime replay capture has one authoritative source, the
> live host `hoi4.exe` process**. Saves may be used as optional development
> fixtures and validation artifacts, but they are not required to record, replay,
> bridge, or complete a live session. This doc reflects the current reality
> and the expanded goal: a **full game-replay/review tool**. The original is preserved
> in git history.

## Current status note

This is the product vision and architecture document, not the current completion
ledger. Current status lives in:

- `RECORDING_LAYER_COMPLETION_PLAN.md`
- `IN_PROCESS_COLLECTOR_PLAN.md`
- `replay_ui/MASTER_PROGRESS.md`
- `coverage_audit/memory_manifest.json`
- `coverage_audit/COVERAGE_REPORT.md`

Do not treat older build-status tables below as a remaining-work list unless
they agree with the current master progress document and manifest.

---

## 1. Purpose & vision

A passive, offline, host-side tool for the group's the game multiplayer games, through
three lenses — and one unifying goal.

1. **Live** — watch the game unfold on a second screen as it happens.
2. **Post-game review** — study finished games in depth, across a whole library.
3. **Competitive integrity = transparency, not refereeing** — all the data on hand,
   comparable and browsable; the group eyeballs it and makes its own calls. No rule
   engine, no auto-flagging.

**Unifying goal — a near-complete REPLAY of how a game played out:** territory,
divisions and their movement/combat state, production and economy, focuses, tech,
diplomacy, wars — scrubbable over the whole game, with a derived timeline of *what
everyone did*. "Everything you'd see in a save game" is the honest mental model,
because the data is essentially the game's own state over time.

**Design stance — MAXIMAL DEPTH.** Capture *everything obtainable*, at the highest
fidelity the engine permits, so the replay is realistic, accurate, and in-depth and
the app holds every piece of information a reviewer could want. This means: every
layer in §4b, both channels pushed to their limits (memory at high cadence incl.
high-frequency combat sampling; the save parsed exhaustively), and — because
"accurate" is a hard requirement — **no fabrication**: where a datum is genuinely
unobtainable (§4 hard-limits) we say so rather than guess. The bar is "as complete and
correct as the game's own data allows," not "looks complete."

---

## 2. Data architecture — one runtime source, optional validators

```
Host running hoi4.exe (deterministic lockstep ⇒ the host holds the WHOLE world;
                       fog of war is a render filter, not a data filter)
  └─ (A) In-process COLLECTOR DLL → live structured snapshots/events
                       |
       External recorder / .battleplan writer → Snapshot store → Diff engine
                       |
          Native replay app — scrub state + browse events

Optional development inputs, never required at runtime:
  - external read-only memory probes for research and comparison
  - save_oracle parsers for fixtures, historical validation, and RE anchoring
  - static game/mod definition parsers for names, localisation, and assets
```

**(A) In-process collector DLL** — injected into the host's `hoi4.exe`, gated by
exact build hash and mod checksum, and limited to direct reads plus small hooks at
known game boundaries. **Role: the live authoritative replay source.** It captures
date/hour, country identity, object roots, deltas, and high-cadence domains from
the same process the group is playing from. It emits structured records through a
pipe/shared-memory/file queue to an external recorder. It must not mutate game
state, call unsafe game logic, or run heavy work inside hook callbacks.

**External recorder** — owns durability, segment metadata, `.battleplan` writing,
schema validation, skipped-hour diagnostics, and native-app compatibility. The
recorder is outside the game process so storage, compression, UI, and replay
inspection cannot crash or stall HOI4.

**Language split** — Rust owns the runtime path that must keep up with the game:
the injected collector, bounded object reads, transport, backpressure, replay
packing/reading, and the native UI. Python remains the research, oracle, profile,
audit, and harness layer while the Rust runtime matures. Runtime speed-five
capture must not depend on Python doing per-frame heap traversal.

**Optional validators** — the existing external `ReadProcessMemory` readers and
`save_oracle` parsers remain valuable for research, fixtures, and field validation.
They are not runtime dependencies and not fallback truth. If the collector cannot
capture a field on a supported live build, that field is marked unavailable or the
affected layer stops publishing. It is never silently filled from a save.

The product decision to allow injection/hooking is now explicit for this private,
host-side, observational tool. See `RESEARCH_in_process_collector.md` for the
HOI4-IC-Graphs evidence and `IN_PROCESS_COLLECTOR_PLAN.md` for the attack plan.

### Current recording-layer checkpoint (2026-06-30)

The recording layer is now a two-channel model:

- **Canonical channel:** hourly-or-better snapshots from the live collector. This
  is the replay timeline and must survive speed five. Daily rows are auxiliary
  rollups only and never satisfy replay coverage.
- **Production compact channel:** the recorder preserves accepted DLL records in
  a compact binary stream with typed domain records/counters, sequence ids,
  `game_hour_raw`, domain ids, record types, payload lengths, and Rust-side
  ordering/decode/skipped-hour validation. JSON root frames are debug/proof
  visibility only; they are not the production throughput target. The current
  snapshot store still dedupes to one canonical snapshot per game hour, while
  the raw compact stream is the bridge for sub-hour combat, movement, and
  event-stream extraction.

Implemented in the current bridge:

- bounded collector worker proof at 10 ms sampling with speed-five hourly
  coverage and zero skipped requested hours;
- session wrapper with lifecycle frames, durable snapshots, `.battleplan`
  finalization, cadence/transport diagnostics, and raw-frame sidecar retention;
- live country identity and country-scalar bridge through a preflight
  `tag_db_id:country_object_ptr` / `tag_db_id:country_record_ptr` control
  contract, with same-process country-array, country-object, and
  country-record caches, explicit cold object/record-scan bootstrap only, warm
  cache validation during recording, and `country_object_scan = false`;
- DLL-native country object rooting through the `CGameState+0x310` `CCountry*`
  table: object-backed country scalars now run without country preflight, with
  per-row validation by heap range, country vtable, object-owned `tag_db_id`,
  tag-table decode, stable name, and duplicate checks;
- DLL-native political power through `CCountry+0xF90 -> CPoliticalStatus`:
  the collector validates the political-status vtable and country backlink,
  then reads `CPoliticalStatus+0xE0` without a country-record preflight bridge;
- DLL-native scheduled politics through the same validated `CPoliticalStatus`:
  `politics_live` emits ruling party, party popularities, ordered active idea
  tokens, typed law buckets, broad non-law `ideas_other`, narrower
  `national_spirits`, counts, election frequency, confidence diagnostics, and
  active country leader rows from
  `CPoliticalStatus -> ruling_party -> CCountryLeader -> CCharacter`, folded
  into `countries.*.commanders.leader`;
  and `politics_residuals` emits `last_election`, `elections_allowed`,
  `appointed_advisors`, `appointed_advisor_details`, and
  `appointed_advisor_slots` when validated static contracts are supplied. The
  advisor catalog comes from `game_defs` / `common/characters` and supplies
  `{slot, ledger, character}` metadata; the law/spirit classifier uses
  `common/ideas` group metadata; live state comes from the active-ideas vector.
  Missing or invalid token contracts omit the relevant leaves, while a measured
  empty `[]` is real. The hot path does not sweep the `CAdvisor` heap. The live
  speed-five proof
  `collector_live_politics_decisions_probe` supplied a 3722-token advisor
  contract and 6474 idea-group rows, advanced 2 game hours with zero skipped
  hours, emitted 3 scheduled politics frames, and emitted zero worker-poll
  politics payloads. Follow-up active-ruler proof
  `replay/snapshots/_collector_session_politics_leader_gate/20260629T171216Z_pid43412`
  advanced 12 game hours, stored 13 verified snapshots, emitted 13 scheduled
  `politics_live` frames, captured 4095 non-ambiguous `commanders.leader` rows
  with up to 315 active rulers per scheduled frame, and kept politics payloads
  off worker-poll frames. Multi-entry ruling-party leader vectors are omitted
  until the active element is proven;
- DLL/Rust compact country cadence proof for `focus_live`, `tech_live`,
  `politics_live`, and `politics_residuals`: release run
  `country_core_no_raw_clock_sample0_speed5_v1` used `sample_ms=0`, advanced
  864 game hours in 45 seconds, emitted 865 scheduled root frames, supplied
  6474 idea-group rows and 3722 advisor-token rows, and skipped 0 game hours.
  This proves the debug compact-country raw-stream cadence for that slice. It
  does not finish `.battleplan` packing;
- DLL/Rust compact binary extraction gate: the native host now supports
  `--stream-format compact --extraction-perf-gate`, writes
  `collector_stream.bin`, and validates packed record ordering, byte volume,
  decodability, frame timing, game hours advanced, every scheduled domain on
  every frame hour, worker-panic status, and skipped game hours without full
  JSON root frames. The DLL keeps one collector/coordinator, one clock source,
  and one sequence/order authority, while the compact gate internally splits
  country extraction into two read-only lanes with separate dirty caches. Live
  speed-five proof `compact_all_country_parallel_coverage_speed5_v2` advanced
  332 game hours in 30 seconds, emitted 333 frame-end records, skipped 0 game
  hours, verified all 8 scheduled country domains on every frame hour, saw no
  worker panic, and measured avg frame 38.151 ms / p95 48.743 ms / max
  156.203 ms / 213,120 bytes. This is the current production-shaped extraction
  gate; JSON results must not be used to decide feasibility. The first
  replay-payload step now adds typed `fuel_live` data records with raw
  fixed-point `i64` fuel values and explicit `32768` scale metadata, and the
  Rust host packs the raw compact stream plus recorder manifest into
  `native_session.battleplan` under manifest-covered roles
  `collector_compact_stream` and `recorder_manifest`. Live speed-five proof
  `compact_fuel_replay_container_speed5_v1` advanced 291 game hours in 30
  seconds, emitted 292 frame-end records, skipped 0 game hours, verified all 8
  scheduled country domains on every frame hour, emitted 292 fuel data records
  for 292 fuel frames with 11,611 typed fuel rows, saw no worker panic, measured
  avg frame 44.869 ms / p95 57.064 ms / max 116.131 ms / 667,672 raw stream
  bytes, and independently verified the 86,012-byte `.battleplan` container;
- follow-up live speed-five proof
  `compact_division_equipment_wide_speed5_20260630_v2` scheduled 14 compact
  domains together for 60 seconds, advanced 113 game hours with 0 skipped,
  verified ordering, decodability, replay packing, and per-hour domain coverage,
  emitted 114 typed `division_equipment` data records through the DLL-native
  anchored division root, captured 155,040 division rows and 348,368
  equipment-entry units, saw no source-unavailable/partial/bridge/worker
  failures, measured avg frame 26.648 ms / p95 28.680 ms / max 74.934 ms /
  5,777,247 raw stream bytes, and verified a 1,694,343-byte `.battleplan`;
- DLL-native scheduled `decisions_live` through
  `CCountry+0xFA0 -> CDecisionStatus`: the collector validates the status
  vtable and owner back-pointer, emits non-empty status-vector counts, and
  decodes active `CTimedDecision` rows from the `+0xA0` list. The same proof
  measured Portugal as `timed_live = []` and decoded non-empty active timed
  decisions for 7 other countries;
- DLL-native scheduled `diplomacy_live`: `CCountry+0x598` supplies opaque
  per-country diplomatic faction groups, folded into
  `countries.*.diplomacy.faction_group` and reconstructed
  `diplomacy_live.faction_members` with `source = "C"`. War relations use a
  one-time preflight bridge from the live `war_relation` heap discovery, then
  the DLL hot path revalidates pinned rows and rereads casualties on scheduled
  frames only. The bridge accepts both observed date-vtable variants in the war
  pool (`CGameDate` RVA `0x26F2398`, `CGregorianDate` RVA `0x26F21A0`) and can
  fall back to live tag-table identity if the old country-object identity scan
  fails. The current faction pid 43412 proof advanced 12 game hours in 48.328
  wall seconds with 14 scheduled diplomacy frames and zero worker-poll
  diplomacy payloads. The current war/casualty pid 43412 proof
  `replay/snapshots/_collector_session_diplomacy_war_gate/20260629T163650Z_pid43412`
  advanced 12 game hours in 43.203 wall seconds, stored 13 verified snapshots,
  observed 14 scheduled diplomacy war frames, captured up to 1033
  `war_relations` per frame, folded 1022 country `at_war` rows and 1022
  country casualty rows, and kept diplomacy payloads off worker polls;
- DLL-native raw `land_combats` lifecycle stream at 10 ms worker cadence: the
  collector emits combat identity, location, age, attacker/defender division
  lists, and lifecycle events into the packaged raw collector stream. The
  current pid 43412 speed-five proof reused the same replay-verified 12-hour run
  and explicitly passed the raw-domain gate with 56 `land_combat_started`, 6695
  `land_combat_updated`, and 11 `land_combat_ended` events, with no heavy
  scheduled payloads leaking onto lightweight frames;
- DLL-native raw `air_wings` telemetry stream: a one-time preflight bridge
  locates live air-wing bodies after live arena discovery, then the DLL hot path
  revalidates pinned bodies and samples owner, wing id, count, mission mask,
  mission region, and effective active planes. The current pid 43412 proof
  `replay/snapshots/_collector_session_air_wing_gate/20260629T164933Z_pid43412`
  advanced 12 game hours in 52.891 wall seconds, verified replay output,
  supplied 652 no-cold cached bridge rows, observed raw `air_wings`, captured
  652 `air_wing_observed` and 9128 `air_wing_updated` events, and reported zero
  raw gaps or heavyweight payloads on lightweight frames. This proves wing-level
  region/mission inputs, not the exact displayed air-superiority percentage;
- DLL-native scheduled `focus_live` through validated `CCountry` child roots:
  `CCountry+0x1370 -> CFocusTree` supplies `focus_tree`, while
  `CCountry+0x1380` supplies completed focuses, completed count, current
  national focus, current continuous focus, and progress days. The frame
  adapter preserves measured-empty `completed = []` by measured TAG and leaves
  `paused` save-only until a live byte is pinned. Live proof
  `collector_live_focus_probe` advanced 2 game hours with zero skipped hours,
  emitted 3 scheduled `focus_live` frames, emitted 1065 country focus rows,
  kept focus payloads off worker polls, and measured Portugal on
  `portuguese_focus` with 22 completed focuses and current
  `POR_standardization`;
- DLL-native scheduled `tech_live` first slice through validated `CCountry`
  child roots: `CCountry+0xF60 -> CTechnologyStatus` supplies completed
  technology count, current research slot capacity, and raw active-research
  slot rows. The collector validates the technology-status vtable, owner
  backlink, completed-tech vector bounds, and the bounded
  `CTechnologyStatus+0xA0` `CResearchSlot*` vector before emitting, and the
  Python adapter folds the values into `countries.*.research.tech_count`,
  `.research_slots`, and `.in_progress_live`. Tech-token resolution is now
  proven through the cache-first live tech-definition bridge. Moving progress
  comes from `live_progress_raw/live_progress_units`
  (`CResearchSlot+0x18 -> CTechnology+0x198`), while the base denominator comes
  from the same active tech through
  `CTechnology+0x160 -> CTechnologyTemplate+0x3e0` as
  `base_required_points_raw/base_required_points_units`; this is not an
  effective-cost modifier claim. The
  `replay/snapshots/_collector_session_research_denominator_gate/20260629T192125Z_pid43412`
  proof advanced 36 game hours in 48.125 wall seconds, used a no-cold cache hit
  for a 552-row tech-definition bridge, emitted 37 scheduled `tech_live`
  frames, captured 15687 named active research-slot rows and 15687 live base
  denominator rows, verified replay output, and kept research payloads off
  worker polls. The manifest recorded 15263 persisted live-progress rows, 15263
  DLL-base progress rows, 346 ETA rows after a daily progress movement, and the
  static denominator audit `{path=game_defs/out/technologies.json, tech_count=557,
  no_research_cost_count=10, base_research_points_per_cost=110}`. The older
  `progress_units` field remains source-badged to `CResearchSlot+0x38` and is
  not accepted as moving progress;
- collector-frame adapter contract for `state_ownership`, `state_control`, and
  `province_control`, including TAG validation and session row-count summaries;
- static DLL map-control path through `CGameState+0x2B0` `CProvince*` rows:
  state owner/controller rows are derived through `CProvince+0xC0`, and province
  controller rows through `CProvince+0x188`, gated to canonical hourly frames
  rather than 10 ms worker polls;
- DLL-native scheduled `state_detail` resource deposits on the same state root:
  the compact proof `compact_state_detail_probe_speed5_20260630_v2` advanced 38
  game hours with 0 skipped, emitted 39 ordered/decodable `map_detail` data
  records, captured 42,159 state-detail rows, saw no
  source-unavailable/partial/parse failures, and kept p95 extraction at 2.363 ms.
  The DLL resource reader now follows the verified state resource vector shape
  where a non-zero capacity lower than size is the logical row count;
- DLL-native scheduled `buildings_live` on the same static map spine: state
  building vectors at `STATE+0x158` and province building vectors at
  `CProvince+0x1C8` are emitted only on scheduled hourly frames and folded into
  `states.*.buildings` plus `provinces.*.buildings`. The current pid 43412
  proof advanced 12 game hours, stored 13 verified snapshots, observed
  `buildings_live` on all 13 scheduled frames, captured 1081 state rows and
  10272 province rows per scheduled frame, and kept building payloads off
  worker polls;
- DLL-native scheduled construction queue rows inside `production_live`: the
  collector reads the production manager general-lines vector at
  `CCountry+0xF68 -> mgr+0x70`, accepts only type-59 construction lines, and
  folds live line id, template id, assigned factories, cost, produced progress,
  and queue priority into `countries.*.production.construction_queue`. The
  proof
  `replay/snapshots/_collector_session_construction_gate/20260629T172526Z_pid43412`
  advanced 12 game hours in 40.219 wall seconds, observed 14 scheduled
  `production_live` frames, folded 3661 construction queue rows with up to 262
  rows per scheduled frame, and kept production payloads off worker polls.
  Target state/location remains intentionally unavailable until the target
  handle is pinned;
- state-root map preflight cache retained as corroboration/fallback evidence.

Still open:

- widen typed compact data records beyond `fuel_live` and `division_equipment`
  to production, focus, tech, politics, decisions, diplomacy, combat, movement,
  and orders, then re-run live speed-five proof for each segment through the
  DLL/Rust `.battleplan` packing path;
- replace repeated bounded-worker attaches with a true long-running collector
  session;
- classify any remaining country-record-only fields and add a speed-safe
  DLL-resident root only if replay-critical fields still need the record array;
- promote remaining economy trade/market/lend-lease, active-research slot detail, diplomacy
  participant/war-goal/war-score detail, naval/air combat, and combat
  battle-detail from historical proof/schema coverage to live collector output
  or explicit unavailable badges. Active land-combat identity, location, age,
  attacker/defender division lists, and lifecycle events now have a live raw
  collector stream with speed-five 12-game-hour proof; focus still needs
  pause-state proof and optional event markers; combat app ingestion and richer
  battle-detail enrichment remain open.

### Historical feasibility evidence (memory-reader and save-oracle research snapshot)

All memory offsets in this snapshot were validated 1:1 against a time-aligned
melt. Keep the evidence, but treat current progress and remaining work as living
in `replay_ui/MASTER_PROGRESS.md` and the coverage manifest. Save-backed rows
below are evidence that the field exists and has a known schema; they are not a
runtime-source commitment. Every replay-critical row must either move to the
live collector, be derived from live collector inputs, or remain explicitly
unavailable.

| datum | source | evidence at snapshot |
|---|---|---|
| date (hours-since-epoch) | memory | ✓ |
| political power, stability, war_support, command_power | memory (country object, i32/100000) | ✓ |
| num_ships | memory | likely |
| state **owner + controller** | memory (per-state runtime obj +0xC8/+0xCC) | ✓ 1081/1081 |
| **divisions**: enumerate + owner + location(province) + template | memory (vtable scan) | ✓ 1603/1603 |
| division **org/strength** | memory (inline u32/100000 @ +0x410/+0x418) | ✓ 1603/1603 exact |
| division **equipment inventory** | memory ({ptr,amount} array @ body+0x358) | ✓ matches save |
| **tag-db-id → TAG** table (the shared identity unlock) | memory (1-based index into contiguous std::string array, stride 0x20, idx0="---") | ✓ 1603/1603 |
| live **province control** (final merged) | memory (dense `u64[base + pid*8]` → controller country obj, bridged save-free) | ✓ 10270/10272 (2 = real post-save drift) |
| province control fallback (override→state-ctrl→owner) | save oracle + province→state map | ✓ all land provinces |
| **active LAND combats**: enumerate + location + age + attacker + **defender** | memory (`CGameState+0x260` active manager; validate body/side/registry chain; each side = inline small-vec, data @ +0x20 / count @ +0x2C) | ✓ 13/13 atk **and** def; later live proof 22/22 manager rows matched heap-scan cross-check |
| naval / air combats | memory (same registry layout is the re-anchor template) | M\* — none active in that snapshot; required a live sea/air battle fixture |
| economy scalars → TAG live (country running_index ↔ tag-db-id) | memory | M\* — open in that snapshot (last save dependency for live economy labeling) |
| divisions (full: +template/equipment), economy deep, fuel/manpower/factories | save oracle | ✓ |
| focuses, tech, production, diplomacy, state detail, active combats | save oracle (7 deep parsers) | ✓ DONE |

**Caveat — country id spaces and collector status:** owner/controller/division-owner
use a 1-based **tag-db id** distinct from the country-array running index. The
tag-db-id->TAG table is cracked and the current collector emits live `tag_db_id`
identity rows without a save. Object-backed country scalar capture is now rooted
inside the DLL through the `CGameState+0x310` country table; political power is
read through the validated `CCountry+0xF90 -> CPoliticalStatus+0xE0` path. The
older country-object/country-record preflight cache bridge is fallback evidence,
not the normal source for current scalar capture. State owner/controller and
province-control scheduled collector emission are wired
through the static `CProvince` map spine with live speed-five proof and no map
preflight bridges required.

### Historical build status (2026-06-24; superseded)

The following snapshot is retained only as historical context. Current status is
the native replay app state in the master progress document and coverage
manifest named above.

The old offline prototype stack once composed end-to-end (validate →
ingest → snapshot store → diff/events → browser map preview), but that browser
runtime has been removed. The retained/current pieces are:
- **save oracle** — historical/dev validation coverage for owner/controller,
  divisions, full province control, and +7 deep parsers
  (focus/tech/production/economy/diplomacy/state-detail/combats). ✓
- **`save_oracle/validate.py`** — 23-check replay preflight (source-fingerprint
  coherence, template/equipment/combat joins, faction bidirectional, redaction). ✓
- **live memory cracks** — owner/controller, divisions (+org/strength/equipment),
  tag-db-id→TAG, province control, and **land combat incl. exact defenders (13/13)** — all
  validated and save-free. ✓
- **`replay/`** — schema + store + diff engine (29 event types,
  no-fabrication gating), 77 tests. ✓
- **Then in flight, now historical:** the old `replay_ui/` web front-end,
  economy-to-TAG bridge work, and review fixes. Current native-app status is
  tracked in the master progress document and coverage manifest named above.

> **MP continuity / resilience** — how the replay stays continuous (and honest) across
> disconnects, hotjoins, rehosts, crashes, desyncs, and rollbacks: see §11.

---

## 3. Replay model — STATE + EVENTS

A replay is two tiers:

- **STATE** — what the world looks like at time T (territory/control, divisions &
  positions & org/strength, production, factories, focus progress, tech, stockpiles,
  diplomacy). Captured by **sampling the live host process** through the collector.
- **EVENTS** — what *happened*. Mostly **computed by diffing adjacent snapshots** — no
  special hooks needed: ownership delta → conquest timeline; division-roster delta →
  builds/losses/merges; focus-tree delta → focuses completed; tech delta → research
  timeline; production/stockpile delta → output; diplomacy delta → wars/factions.
  Optionally sharpened by **collector hooks or exact-timed event markers** for known
  engine/update boundaries and cheap script-exposed `on_actions`.

Implementation: a **snapshot store** (time series per game) + a **diff engine** that
derives the event feed. Event resolution = the collector cadence for the underlying
layer. There is no save-cadence runtime layer.

---

## 4. Feasibility tiers (replaces the old "impossible" list)

- **Tier A — collector-backed today / bridge-proven:** date/hour, country
  identity, country scalars through the preflight cache bridge, speed-five
  hourly proof, raw collector frame preservation, and schema adapters for
  state/province map-control frames. State owner/controller roots are fast once
  cached and scheduled DLL emission is wired. Full province control remains open
  until the dense province array root is fast and validated.
- **Tier B — proven by save parsing, must become live for runtime:** focuses (and
  completion timeline via diff), tech/research, production lines + gross output by
  equipment, equipment stockpiles, factories (civ/mil/dockyard), fuel/manpower,
  diplomacy (wars/factions/opinions), per-division equipment, *active* combats at
  snapshot time. The save parsers prove structure and fixtures; the shipped recorder
  must source these from the live collector or badge the gap.
- **Tier C — transient combat: PURSUED at maximum fidelity (per the maximal-depth
  stance).** Battles aren't stored as *history* — only as *active* combats at the
  snapshot instant — so we capture them by **high-frequency sampling of the live
  active-combat structures in memory** (land combats, naval battles, air engagements)
  plus per-division strength/org deltas, and reconstruct each engagement (location,
  participants, duration, casualties, outcome) from the appear→persist→disappear
  lifecycle. This is the heavy path, but it's the one that turns combat from a sketch
  into a real battle log. Cadence here is the highest we can sustain (auto-coarsening
  only when game speed outruns the sampler). **Honest residual even at max effort:**
  an engagement that fully begins *and* ends *between* two consecutive samples is
  unobservable — so combat is *near-complete at high fidelity*, not provably
  tick-perfect; very short skirmishes at max game speed may be missed or merged. We
  surface confidence/coverage rather than pretend completeness. Pure one-shot
  **decision clicks** are recovered only when they leave persistent state or fire a
  log marker (§8); we mark the rest as not individually attributable.
- **Hard-impossible (state it, never fake it):** true Steam account identity/name
  (persona/display names may be available when captured, but verified Steam
  identity is not a replay datum; identify by tag/country name and captured
  occupancy when needed);
  any sub-sample-transient micro-event (above); and a handful of pure-UI-derived
  display numbers that exist only in the renderer (reconstructed from raw inputs, not
  read).

---

## 4b. Data inventory — everything the app captures

The full target set for "all the information a reviewer could need." Historical
source tags are retained to show current evidence: **C** = collector-fed or
collector-frame contract exists, **M** = live memory already cracked externally,
**M\*** = live memory/collector crackable, **S** = currently proven only by save
parser, **D** = derived (diff / join / compute). In v3, `S` does **not** mean an
acceptable runtime source. It means the row must be promoted to the live
collector, derived from live collector inputs, or honestly badged as unavailable.

**Country — economy & industry:** civ / mil / dockyard factories `S/M*`; factories under
construction + what's queued `S`; construction power `S`; consumer-goods & trade-law
allocation `S`; **gross IC output by equipment type** `S/D`; equipment stockpiles by
type & variant `S`; resources produced/consumed/imported/exported per resource (steel,
oil, aluminium, tungsten, chromium, rubber) `S`; fuel current/max/gain `S/M*`; trade
agreements & convoys `S` (legacy save-proven; promote to live or badge unavailable).

**Country — military:** total divisions `M/D`; manpower deployed/reserve & army/navy/air
split `S/M*`; army/navy/air XP `S`; equipment held by type+variant `S`; ships by hull
type + tonnage `S/D`; planes by type `S`; division templates in use `M/S`; **casualties
(killed/captured) → K/D** `S/D`; command power `M`.

**Country — politics:** ruling ideology + party popularities `M/S`; **stability `M`,
war support `M`, political power (+growth) `M`, command power `M`**; active
national spirits/ideas `M/S`; cabinet (leaders, advisors, theorists) `S`;
laws (conscription / economy / trade) `S/M*`; autonomy / subject-overlord `S`;
faction membership + leader `S`.

**Country — research:** technologies researched **+ when** `S/D`; completed-tech
count and research-slot capacity `C/M/S`; what's in progress + % `S/M*`;
research bonuses / ahead-of-time penalties `S`.

**Country — focus:** current focus + progress `S/M*`; **completed focuses + timeline**
`S/D`; full focus tree state `S`.

**Country — decisions/missions:** active decisions & categories `S`; timed decisions &
cooldowns `S`; ongoing missions `S`; one-shot clicks `D` (only when state-persisting / log-marked).

**Country — diplomacy & intel:** war pairs / at-war sets / side casualties
`C/M/S`; war goals, war score, participant-level casualties `S`; factions
`C/M/S`; guarantees / NAPs / embargoes `S`; opinion & relations matrix `S`; lend-lease,
volunteers, attachés, expeditionary forces `S`; operatives, intel networks, decryption
`S`; peace-conference state `S`.

**State (×1081):** **owner/controller `C/M/S`** (`C` contract + preflight cache
proven; scheduled DLL emission path wired and live proof pending); core/claim status `S`; buildings at
current level (infra, civ/mil/dockyard, forts, AA, radar, naval base, rocket/nuclear)
`S/M*`; resources `S`; victory points `S(static)`; manpower `S`; state category `M/S`;
resistance / compliance / occupation law `S`; supply `S/M*`.

**Province (×~13.4k):** **controller** `C/S/D/M*` only when a collector
`province_control` frame is present; historical save fallback exists for
fixtures, but live dense-array relocation/cache remains open for normal
recording. Supply & rail/route usage `S`; naval-base/port usage `S`.

**Division (each, ~1654):** **owner `M`, template `M`, location/province `M`, organisation
`M`, strength `M`, equipment inventory `M`**; experience `M*`; movement (moving? target
province + progress) `M`; in-combat flag `M`; encirclement/supply status `M*`.

**Combat (the marquee — Tier C, high-freq sampled):** active **land** combats
(location, age, attacker/defender division ids) `M`; land battle-detail
(combat width, terrain, tactics/modifiers, GUI progress) `M*`; **naval**
battles (fleets, ships engaged, sinkings) `M`; **air** wing telemetry / region
mission inputs `C/M/S`, displayed air-superiority percentage `M*/S`; per-engagement casualties & outcome `D` (reconstructed from
the combat lifecycle + strength deltas).

**Map layers:** territory (owner) `C/M`; province control `C/S/D/M*` with the
same province-array caveat above; supply areas & strategic regions `S(static)`;
**front lines** (derived from control adjacency) `D`; naval/air region control
`M*/S`; computed VP & controlled-industry share `D`.

**Event timeline (derived + marked):** DOWs, capitulations, faction changes, focus &
tech completions, state/province captures, peace deals, government changes, nukes, major
naval/air engagements — `D` from snapshot diffs, `S` from save deltas, exact-timed where
the mod log marks them (§8).

---

## 5. Cadence — collector-owned, per-layer

- **Clock and segment sentinels:** captured at every collector boundary and used
  to reject torn snapshots.
- **Raw collector samples:** preserved in `collector_frames.jsonl` so 10 ms /
  sub-hour observations survive even when the canonical store dedupes to one row
  per game hour.
- **Light state layers** (date, country identity, ownership/control, economy
  scalars): canonical hourly-or-better snapshots, with millisecond-level
  collector work as the target. Daily-only capture is insufficient for replay
  coverage.
- **Heavy object graphs** (divisions, production, diplomacy, focus/research,
  buildings, OOB): captured on scheduled hourly boundaries when they affect the
  replay timeline. Daily aggregates may exist only as auxiliary rollups.
- **Transient combat and movement:** highest sustainable cadence. Prefer event
  hooks plus bounded live reads; record explicit coverage/skipped-boundary
  diagnostics when game speed outruns capture.
- **No save interval layer.** Host autosaves are not part of normal runtime
  fidelity. A field that is only save-backed today remains open until the live
  collector can emit it or the UI badges it unavailable.

---

## 6. Correctness & patch-resilience (non-negotiable)

- **exe SHA-256 build gate** — refuse on an unknown build, never run stale
  offsets/hooks blind.
- **Mod checksum / data-def profile** — record the exact mod set and static
  definition hashes used to decode IDs, names, sprites, localisation, and schema
  joins.
- **Live-layout validation** — validate collector roots with sentinels, vtables,
  object counts, tag tables, and cross-domain invariants. Saves may be used in
  development to discover or audit a layout, but the runtime collector cannot
  require them.
- **Fail closed:** on layout mismatch, torn reads, impossible counts, or hook
  install failure, stop publishing the affected layer or stop recording. Do not
  fall back to saves.
- **Hook discipline:** callbacks do the minimum work needed to identify a safe
  boundary and enqueue work. Heavy traversal, compression, and disk writes happen
  outside the game-thread hook path.
- **Read consistency:** capture boundary sentinel around each snapshot; reject
  if the in-game hour/tick changes before the snapshot is durably accepted.

---

## 7. Native Replay UI

The active UI is the native Rust/wgpu replay app in `replay_native/`, not a
browser dashboard. A single **global timeline scrubber drives every open
inspection surface**, and the app opens snapshot stores or portable
`.battleplan` recordings.

Screens:
1. **Overview / leaderboard over time** — sortable raw-stats table of all players across
   every metric (IC output, factories, casualties/K-D, controlled VP, …), scrubbable.
2. **Country deep-dive** — any playable country → its full history: production mix and
   **army composition** over time, focuses/tech timeline, rank vs rivals.
3. **Native map** — owner view (state) + control view (province) + weather,
   resources, units, route/front overlays, and badged proxy layers; scrub the
   whole game; click a state/province/unit/combat bubble for read-only drilldown.
4. **Event timeline / feed** — the diff-derived "what everyone did" (conquests, focuses,
   tech, wars, builds/losses), filterable by country/type.
5. **Combat layer** — the Tier-C inferred combat heatmap + casualty attribution
   (explicitly inferred, not a literal battle log).

The native map renderer + scrubber are snapshot-driven, so all of the above is
the same data model at different cadences.

---

## 8. In-game mod (scope grows slightly)

- **Improved graphs** (kept) — the only thing visible in-game; auto-bind curves to
  humans, gross IC-output metric, autoscale, survives a major being annexed.
- **Event-marker log channel (REVIVED for replay)** — was cut as "not useful" under the
  old design; for a replay timeline it's worth it. Emit exact-timed `on_actions` markers
  (capitulation, war declared, focus completed, faction join) to `game.log` to sharpen
  the diff-derived event feed. Cheap, optional, complements (does not replace) the
  snapshot diffs. The mod can NOT export divisions/combat — that's the collector's job.

---

## 9. Build order (phased)

1. **Collector skeleton + transport** — build the smallest injected DLL that
   proves build gating, safe attach/detach, date/hour capture, country identity,
   and a pipe/shared-memory stream consumed by the external recorder.
2. **Schema parity** — write collector output into the existing snapshot schema
   and `.battleplan` container with `backend = "in_process_collector"` provenance.
3. **IC-Graphs-class economy proof** — reproduce country MIL/CIV/NAV-style
   output from live process state, then prove whether it can be sampled hourly
   for replay rows. Daily output is a validation/rollup aid, not acceptance.
4. **Light-state live closure** — owner/control, province control, country
   scalars, buildings/resources/factories, and country identity with no save
   dependency.
5. **Division/OOB closure** — divisions, templates, equipment, org/strength,
   movement, army/fleet/air command graphs, and UI counter anchors.
6. **Deep-domain promotion** — focus/research, production, stockpiles, diplomacy,
   intel, decisions/missions, project depth beyond the first live catalog
   slice, market, and special systems move from save-proven schema to live
   collector output or explicit unavailable badges.
7. **High-frequency combat capture** — hook/sample land/naval/air combat
   lifecycles at the highest sustainable cadence, reconstruct events from live
   state deltas, and emit coverage confidence.
8. **Diff engine → event timeline**; full **Overview leaderboard**, **Country
   deep-dive**, **Event feed**, and the **combat layer** in the UI; game
   library/archive polish.

---

## 10. Open defaults (decide at build time)

- Collector tick targets: hourly-or-better snapshot mode, combat/movement
  high-rate mode, optional daily rollups, and explicit overload/coarsening
  diagnostics.
- Save ingest: development-only fixture/import path, never required for runtime
  recording or replay.
- Map coloring: country map-colors, faction-grouping toggle. Live refresh ~15s.
- Archive retention: keep everything; manual prune.
- Combat-inference thresholds (strength-drop / control-flip sensitivity).

---

## 11. MP continuity / resilience

Real MP is messy: players drop and rejoin, the host crashes and someone rehosts, a
desync forces a reload, the group rolls back to an earlier save after a disaster. A
replay has to stay **continuous** across all of that — and where it genuinely can't, say
"no data" rather than invent a bridge (the §1 / §4 no-fabrication rule applies to *time*
too). The design principle: **continuity is a lineage problem, not a time-ordering
problem.** We never collapse ambiguity into false continuity by sorting on the clock.

### 11.1 The read-the-host advantage (why we start ahead)

the game MP is **deterministic lockstep**: every client re-simulates the same orders, and
the host holds the authoritative whole-world state (fog of war is a render filter, not a
data filter — §2). On a **desync the clients reload the host's state**, so the host is by
definition never "wrong" relative to the table. Reading the host is therefore
**desync-immune**: a desync that scrambles a client changes nothing we capture, because
we never read a client. We read the one machine the whole game is forced to agree with.
This removes the single hardest continuity problem (which copy of the world is true)
*for the steady state* — the remaining work is bridging the **gaps and restarts** around
that authoritative stream.

### 11.2 Disconnect / hotjoin = EVENTS, not gaps

A player dropping or rejoining is **not** a hole in the data. The world keeps simulating
on the host regardless; control of the abandoned country flips to AI and back. We capture
this as a normal diff-derived event, the same machinery as everything else:

- A drop/hotjoin surfaces as an **`is_human` transition** (`is_ai` toggling) on the
  affected country — a `player_seat_change` event (§9 family), not a `no_data` span.
- **Player-seat occupancy is modeled separately from country AI/human control.** A seat
  event is not always a clean disconnect: a human may **switch country**, **co-op a
  single country** (multiple humans, one tag), **observe/spectate**, or **sub in** for
  someone. So the seat layer tracks *who occupies which tag* distinct from *is this tag
  AI-run right now*; `is_human` alone can't express co-op or a hand-off. Both are derived
  from the same per-country flags the save/memory already expose (`S/M`, `D` for the
  transition).
- **Source-of-truth & honest limit.** The seat layer is **occupancy-by-tag**, observed
  from the per-country human/AI flags (and any save-exposed seat/co-op markers) — *not*
  verified Steam account identity. Captured persona/display names can be shown
  when present, but they are not proof of a Steam account identity. A seat is
  therefore attributed by **TAG** and captured occupancy metadata, never guessed
  as a person; a substitution or a co-op partner is visible as *the tag is
  human-occupied*, not *which verified human*. Where the engine exposes nothing
  to distinguish two humans on one tag, that distinction is `null` (not guessed)
  — the no-fabrication rule, applied to seats.

The data stream is uninterrupted across a disconnect because the host never stopped.

### 11.3 Downtime gaps (crash / rehost / restart) — live segments, else "no data"

The stream *does* break when the **host process** stops: a crash, a deliberate rehost, a
machine restart. The live collector handles this as segment discontinuity:

1. **The collector/recorder auto-reattach.** On the new `hoi4.exe` process the
   injector/recorder re-runs the build gate, mod checksum, hook install, and live
   root validation. A patched build is supported only when its collector profile is
   known.
2. **Continuity is explicit.** If the new process continues the same world, the
   next segment is linked by observed hour/world-hash continuity or by an explicit
   operator/session lineage decision. If continuity cannot be proven, the timeline
   records a boundary.
3. **Host-machine change is operational, not data-magical.** A different host can
   continue the replay only if a collector is running on that host and uploads the
   next live segment. The replay model can join segments, but it does not infer
   missing world state from saves.

**Honest residual:** a span covered by no live collector output is marked
**`no_data`**. Optional saves may be imported later as comparison artifacts or
debug fixtures, but they do not fill the canonical runtime timeline.

### 11.4 Rollbacks — the SEGMENT + SUPERSEDE model

The hard case is a **reload from an earlier save**: in-game time goes **backwards**.
Naive `hour_index` ordering would interleave the discarded future with the new branch
and produce a nonsense timeline. The model:

- **`campaign_id`** identifies one playthrough. It is a **composite fingerprint**, not a
  single field: collector session id, start date/hour, scenario/bookmark, mod-set
  checksum, initial country roster, and any live-exposed unique seed/identity values.
  A matching seed is evidence, not identity: incompatible start invariants split into
  a new campaign. We store the **raw components**, not just the hash, so an ambiguous
  match can be re-decided later.
- **A SEGMENT = one continuous capture run** of a single authoritative source, bounded by
  a discontinuity (§11.5). Every snapshot is tagged
  **`(campaign_id, segment_id, reader_run_id, hour_index, captured_at, ingest_id,
  world_hash)`**.
- **Stitching:** within the canonical branch, order by `hour_index`. Across segments,
  resolve overlaps by **lineage, never by the clock** (§11.6).
- **Supersede:** when a reload's new segment **demonstrably replaces** an earlier branch's
  now-discarded future (an explicit operator/session lineage edge, collector load marker,
  or other proven continuity signal), the **later branch is canonical** and the earlier
  future is **demoted to an optional side-branch — kept, never deleted.** The rolled-back
  timeline stays browsable ("what would have happened"); it just isn't the main line.
- **Divergent overlap with no lineage edge = FORK, not supersede.** Same `campaign_id`,
  overlapping `hour_index`, *different* `world_hash`, and no established reload edge ⇒ we
  record a **fork conflict** and keep both branches, surfacing it for an operator/session
  call. We do **not** silently pick one. **Identical** overlap (same `world_hash`) just
  **dedupes**.

### 11.5 Detecting a segment boundary (multi-signal, not just "time went back")

`hour_index` **decrease** catches a plain rollback, but a reload can also jump **forward**
(collector stopped Jan 10 → host crashed → group continued from a Jan 15 rehosted
state → next collector read is Jan 15, `hour_index` *increased* yet continuity is broken). So a boundary is
declared on **any** of:

- **process change** — new `hoi4.exe` pid / process-start-time, or first read after a
  reattach;
- **`hour_index` decrease** — classic rollback;
- **`world_hash` / checksum discontinuity at equal-or-forward time** — the world changed
  in a way forward simulation can't explain (a load, a console/save edit, manual mutation);
- **a large wall-clock gap** with missing expected ticks (host was down);
- **an explicit reload/authority signal** from the recorder/operator workflow.

Each segment records a **`segment_start_reason`** (`new_process`, `hour_decrease`,
`save_loaded`, `reader_restart`, `gap_after_timeout`, `manual_rehost`, …). A forward jump
across a reload is **not** a rollback but is still a **new segment**; the unobserved span
between is `no_data`.

### 11.6 Which branch wins — explicit precedence (never "latest capture")

"Later-captured supersedes" is **too blunt and can pick the wrong branch.** Counterexample:
an abandoned old host keeps simulating to 1941 while the real group rehosts from a 1940
save and plays to 1940.5; the stale host's reader uploads *later wall-clock* data from
1941 — "latest capture" would crown the dead branch. So canonicity is decided by
**precedence**, top wins:

1. **Explicit authority** — an operator/session marks the canonical host (the "this is the
   real game" pin).
2. **Reload / authority lineage** — a proven collector/operator load edge
   supersedes the branch it replaced (the strongest automatic rule after an
   explicit authority pin).
3. **Same-source continuous segment** — within one `reader_run_id`, higher `hour_index`
   wins (normal forward play).
4. **Identical-state dedupe** — equal `world_hash` at the same `hour_index` collapses.
5. **Otherwise → fork conflict, no automatic supersede.**

Wall-clock `captured_at` is only a **provenance clue** (it hints which branch was *observed*
later), never a truth rule on its own.

### 11.7 Bad inputs — torn reads and corrupt saves never become canonical

- **Mid-tick / torn memory read.** A single date read is *not* enough — the engine can
  advance between the date read and the body read, yielding an impossible hybrid (date from
  tick N, armies from N+1). Mitigation: a **before/after sentinel** — read
  `hour_index` + a tick/checksum sentinel **before and after** the full snapshot and accept
  only if **unchanged**, plus structural sanity checks (object counts, pointer validity, no
  impossible negatives, stable country table). A snapshot carries a **quality status**
  (`complete` / `torn_rejected` / …); **only `complete` enters the canonical diff stream**
  — a torn read is discarded/retried, never stored as truth. (This extends the §6
  date-sentinel and region-clamp rules.)
- **Partial / corrupt / truncated save fixture.** Optional saves can still be used
  during development and offline audits. They must move through an ingest state
  machine (`pending → stable → parsed_ok | parsed_invalid | quarantined`) before
  being used as fixtures. A save artifact can never create canonical runtime
  state, fill a `no_data` span, or supersede a live segment.

### 11.8 Cadence & fail-closed (continuity knobs)

- **Collector uptime** is the continuity backbone. The recorder tracks expected
  boundaries, missed boundaries, process changes, hook status, and upload/flush
  health.
- **No save fallback:** autosaves may be archived for manual review, but they are
  not a replay bridge and not a lower-cadence truth channel.
- **Fail-closed on patch:** if build gating, hook install, or live-layout
  validation fails after a build change, the collector stops publishing the
  affected layer or stops recording. Continuity degrades to explicit `no_data`,
  never to fabricated or save-filled data.
- **Paused ≠ gap:** a long wall-clock stretch at an unchanged `hour_index` is the game
  **paused**, not missing data — distinguished from a true downtime gap by the
  process/sentinel signals (§11.5), so we don't mark a coffee break as `no_data`.

### 11.9 Data-model implications (schema + store + timeline)

These are the concrete extensions the model needs (additive to `replay/SCHEMA.md` §0–§2;
`schema_version` stays additive):

- **Snapshot identity** gains, alongside `hour_index`:
  `campaign_id`, `segment_id`, `reader_run_id`, `capture_source_id`, `captured_at`,
  `ingest_id` (store-generated, the only *total* ingest order), `world_hash`, and a
  snapshot **`quality_status`**. `capture_seq` is **monotonic only within a
  `reader_run_id`** (it resets on reader restart), so it never serves as a cross-restart
  order key — `ingest_id` / `captured_at` do.
- **Segment record:** `parent_segment_id`, `parent_lineage_evidence`,
  `segment_start_reason`, `authority_status`, and the per-segment **collector profile**
  (`exe_sha256` + mod checksum + hook/layout profile) so a patched rehost is
  self-describing.
- **Campaign record:** the **raw fingerprint components** (not just the hash) so a collision
  or alias can be re-adjudicated.
- **Store / timeline** become **segment-aware**: dedupe identical overlapping ticks by
  `(campaign_id, hour_index, world_hash)` — **segment-agnostic on purpose**, so the *same*
  state captured by two segments (or two readers) collapses to one (§11.6 step 4); resolve
  divergent overlaps by the §11.6
  precedence (supersede on lineage, else fork); expose the canonical branch to the scrubber
  while keeping rolled-back side-branches and fork branches **selectable, never dropped**.
- **`no_data` is first-class:** spans the canonical branch can't cover with live
  collector output are explicit gaps in the timeline, surfaced in the UI — coverage,
  not silence.
