# Battle Plan / Replay — design (v2)

> **v2 supersedes the original log-channel design.** The original DESIGN treated the
> in-game mod's `log=` channel as the primary data source and listed divisions,
> division counts, tech, naval tonnage, etc. as "not possible." That was true **only
> for the log channel**. Since then the data architecture pivoted to a **host-side
> external memory reader + rakaly-melted save oracle**, and live RE has *proven* most
> of that old "impossible" list is reachable. This doc reflects the current reality
> and the expanded goal: a **full game-replay/review tool**. The original is preserved
> in git history.

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

## 2. Data architecture — two channels, two jobs

```
Host running hoi4.exe (deterministic lockstep ⇒ the host holds the WHOLE world;
                       fog of war is a render filter, not a data filter)
  ├─ (A) External MEMORY READER  → live daily/hourly snapshots   [liveness/cadence]
  └─ (B) SAVE ORACLE (rakaly melt + parse)  → periodic snapshots  [truth/depth/anchor]
                       │
         Snapshot store (per game) ──► Diff engine ──► Event timeline
                       │
         Dashboard (offline, 127.0.0.1) — scrub state + browse events
```

**(A) Memory reader** — `ReadProcessMemory`, read-only, passive (no desync, any
cadence), 64-bit Python + ctypes. Sees all ~countries at once. **Role: the live
daily/hourly tick.** Weakness: offsets change every build → patch-fragile, and reads
can silently misread.

**(B) Save oracle** — melt the host's save with rakaly, parse the plaintext (stable
text keys, patch-tolerant). **Role: truth, not cadence** — (1) validate the memory
reader (fail-closed on divergence), (2) re-anchor/re-discover memory offsets each
build, (3) supply deep nested layers that aren't worth cracking in memory, (4)
durable fallback + retroactive coverage of games where the reader wasn't running.
**Periodic, not forced-daily** (daily saves stutter an MP session; normal
autosaves / end-of-session / monthly suffice for these roles).

**Neither channel alone.** Memory gives liveness but lies silently and dies on patch;
the save gives truth and patch-stability but can't be taken often in MP. The hybrid is
the design: **memory for liveness, save for truth.**

### Proven so far (this build; all memory offsets validated 1:1 against a time-aligned melt)
| datum | source | status |
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
| **active LAND combats**: enumerate + location + age + attacker + **defender** | memory (combat-sides registry chain; each side = inline small-vec, data @ +0x20 / count @ +0x2C) | ✓ 13/13 atk **and** def |
| naval / air combats | memory (same registry layout is the re-anchor template) | M\* — none active at snapshot; RE pending a live sea/air battle |
| economy scalars → TAG live (country running_index ↔ tag-db-id) | memory | M\* — in progress (last save dependency for live economy labeling) |
| divisions (full: +template/equipment), economy deep, fuel/manpower/factories | save oracle | ✓ |
| focuses, tech, production, diplomacy, state detail, active combats | save oracle (7 deep parsers) | ✓ DONE |

**Caveat — country id spaces:** owner/controller/division-owner use a 1-based
**tag-db id** distinct from the country-array running index. The tag-db-id→TAG table
is now **CRACKED in memory** (1-based std::string array, validated 1603/1603), so
owner / controller / divisions / province-control / combat are all **fully save-free**.
The one residual in this id-space work is the country-array running_index ↔ tag-db-id
bridge — needed *only* to label live economy scalars with a TAG without the save —
which is in progress.

### Build status (2026-06-24)
The offline stack is **built and composes end-to-end** (`build_replay.py`: validate →
ingest → snapshot store → diff/events → map time-lapse + scrubber):
- **save oracle** — owner/controller ground truth, divisions, full province control, +7
  deep parsers (focus/tech/production/economy/diplomacy/state-detail/combats). ✓
- **`save_oracle/validate.py`** — 23-check replay preflight (source-fingerprint coherence,
  template/equipment/combat joins, faction bidirectional, redaction). Used as the gate in
  `build_replay.py`. ✓
- **live memory cracks** — owner/controller, divisions (+org/strength/equipment),
  tag-db-id→TAG, province control, and **land combat incl. exact defenders (13/13)** — all
  validated and save-free. ✓
- **`replay/`** — schema + store + diff engine (29 event types, no-fabrication gating), 77
  tests. ✓   **`replay_render/`** — PNG time-lapse + HTML scrubber. ✓
- **In flight:** the `replay_ui/` 5-screen front-end, the economy→TAG live bridge, and the
  CODE_REVIEW fixes. **Residual:** naval/air combat (re-anchor when a live sea/air battle
  exists); a coherent `save_oracle/output` regen (the current dir is a stale 2-save mix).

> **MP continuity / resilience** — how the replay stays continuous (and honest) across
> disconnects, hotjoins, rehosts, crashes, desyncs, and rollbacks: see §11.

---

## 3. Replay model — STATE + EVENTS

A replay is two tiers:

- **STATE** — what the world looks like at time T (territory/control, divisions &
  positions & org/strength, production, factories, focus progress, tech, stockpiles,
  diplomacy). Captured by **sampling** (memory frequently; saves periodically).
- **EVENTS** — what *happened*. Mostly **computed by diffing adjacent snapshots** — no
  special hooks needed: ownership delta → conquest timeline; division-roster delta →
  builds/losses/merges; focus-tree delta → focuses completed; tech delta → research
  timeline; production/stockpile delta → output; diplomacy delta → wars/factions.
  Optionally sharpened by **exact-timed event markers** from the in-game mod for the
  few things the game exposes as `on_actions` (capitulation, war declared, focus complete).

Implementation: a **snapshot store** (time series per game) + a **diff engine** that
derives the event feed. Event resolution = the sampling interval of the underlying
layer (memory layers exact-to-the-tick; save-only layers coarser).

---

## 4. Feasibility tiers (replaces the old "impossible" list)

- **Tier A — in hand:** date, per-country PP/stability/war_support/command_power,
  state owner+controller, full province control, division enumeration with
  owner/location/template (org/strength imminent). Map time-lapse + division layer.
- **Tier B — reachable, save-parseable (stable text keys):** focuses (and completion
  timeline via diff), tech/research, production lines + gross output by equipment,
  equipment stockpiles, factories (civ/mil/dockyard), fuel/manpower, diplomacy
  (wars/factions/opinions), per-division equipment, *active* combats at snapshot time.
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
- **Hard-impossible (state it, never fake it):** Steam/player real names (no script or
  data access — identify by tag/country name, humans flagged `is_ai = no` at start);
  any sub-sample-transient micro-event (above); and a handful of pure-UI-derived
  display numbers that exist only in the renderer (reconstructed from raw inputs, not
  read).

---

## 4b. Data inventory — everything the app captures

The full target set for "all the information a reviewer could need." Source tags:
**M** = live memory (cracked), **M\*** = memory (crackable, planned), **S** = save oracle,
**D** = derived (diff / join / compute). Cadence per §5.

**Country — economy & industry:** civ / mil / dockyard factories `S/M*`; factories under
construction + what's queued `S`; construction power `S`; consumer-goods & trade-law
allocation `S`; **gross IC output by equipment type** `S/D`; equipment stockpiles by
type & variant `S`; resources produced/consumed/imported/exported per resource (steel,
oil, aluminium, tungsten, chromium, rubber) `S`; fuel current/max/gain `S/M*`; trade
agreements & convoys `S` (per-*partner* live totals not script-exposed — save-only).

**Country — military:** total divisions `M/D`; manpower deployed/reserve & army/navy/air
split `S/M*`; army/navy/air XP `S`; equipment held by type+variant `S`; ships by hull
type + tonnage `S/D`; planes by type `S`; division templates in use `M/S`; **casualties
(killed/captured) → K/D** `S/D`; command power `M`.

**Country — politics:** ideology + party popularities `S`; **stability `M`, war support
`M`, political power (+growth) `M`, command power `M`**; national spirits/ideas `S`;
cabinet (leaders, advisors, theorists) `S`; laws (conscription / economy / trade) `S`;
autonomy / subject-overlord `S`; faction membership + leader `S`.

**Country — research:** technologies researched **+ when** `S/D`; research slots, what's
in progress + % `S/M*`; research bonuses / ahead-of-time penalties `S`.

**Country — focus:** current focus + progress `S/M*`; **completed focuses + timeline**
`S/D`; full focus tree state `S`.

**Country — decisions/missions:** active decisions & categories `S`; timed decisions &
cooldowns `S`; ongoing missions `S`; one-shot clicks `D` (only when state-persisting / log-marked).

**Country — diplomacy & intel:** wars (participants, war goals, war score) `S`; factions
`S`; guarantees / NAPs / embargoes `S`; opinion & relations matrix `S`; lend-lease,
volunteers, attachés, expeditionary forces `S`; operatives, intel networks, decryption
`S`; peace-conference state `S`.

**State (×1081):** **owner `M`, controller `M`**; core/claim status `S`; buildings at
current level (infra, civ/mil/dockyard, forts, AA, radar, naval base, rocket/nuclear)
`S/M*`; resources `S`; victory points `S(static)`; manpower `S`; state category `M/S`;
resistance / compliance / occupation law `S`; supply `S/M*`.

**Province (×~13.4k):** **controller** (override→state fallback) `S/D`, `M*` for live;
supply & rail/route usage `S`; naval-base/port usage `S`.

**Division (each, ~1654):** **owner `M`, template `M`, location/province `M`, organisation
`M`, strength `M`, equipment inventory `M`**; experience `M*`; movement (moving? target
province + progress) `M`; in-combat flag `M`; encirclement/supply status `M*`.

**Combat (the marquee — Tier C, high-freq sampled):** active **land** combats (location,
attacker/defender divisions, combat width, terrain, modifiers, progress) `M`; **naval**
battles (fleets, ships engaged, sinkings) `M`; **air** engagements / region superiority
`M/S`; per-engagement casualties & outcome `D` (reconstructed from the combat lifecycle +
strength deltas).

**Map layers:** territory (owner) `M`; province control `S/D`,`M*`; supply areas &
strategic regions `S(static)`; **front lines** (derived from control adjacency) `D`;
naval/air region control `M*/S`; computed VP & controlled-industry share `D`.

**Event timeline (derived + marked):** DOWs, capitulations, faction changes, focus &
tech completions, state/province captures, peace deals, government changes, nukes, major
naval/air engagements — `D` from snapshot diffs, `S` from save deltas, exact-timed where
the mod log marks them (§8).

---

## 5. Cadence — a per-layer dial

- **Memory layers** (date, economy scalars, owner/control): hourly is essentially free
  (relocation is once per session; re-reads are milliseconds). Daily by default,
  hourly available.
- **Division layer** (memory): bounded by a ~seconds heap vtable scan per snapshot —
  daily by default; hourly feasible but **auto-coarsens at high game speed**.
- **Deep layers** (save-only: focuses/tech/production/diplomacy/combats): resolution =
  the **save interval** (host autosave / end-of-session). Not forced-daily.
- Sampling is driven by polling the in-memory date and snapshotting on the chosen
  hour/day boundary.

---

## 6. Correctness & patch-resilience (non-negotiable)

- **exe SHA-256 build gate** — refuse/relocate on an unknown build, never run stale
  offsets blind.
- **Re-discover layouts each build from save invariants** (the anchor methods —
  PP-Rosetta, manpower anchor, division org/strength anchor — all bootstrap off the
  save).
- **Validate memory against the latest save** (owner/control/division counts, field
  agreement). **Fail closed:** on divergence, stop publishing the affected layer and
  fall back to save snapshots rather than emit guessed data.
- Read consistency: date-sentinel around each snapshot (read date → read → re-read
  date); single-RPM array reads; **clamp reads to the VirtualQuery region end** (a
  fixed-size read straddling a region edge returns 0 for the whole span).

---

## 7. Dashboard / replay UI

Offline, `127.0.0.1`, Python stdlib server + vendored JS (Chart.js); fully offline.
A single **global timeline scrubber drives every view**, and a **game-library browser**
switches between the live game and any archived game.

Screens:
1. **Overview / leaderboard over time** — sortable raw-stats table of all players across
   every metric (IC output, factories, casualties/K-D, controlled VP, …), scrubbable.
2. **Country deep-dive** — any playable country → its full history: production mix and
   **army composition** over time, focuses/tech timeline, rank vs rivals.
3. **Map time-lapse** — owner view (state) + control view (province) + a **division
   layer** (positions, strength/org); play/scrub the whole game; click a state/province
   for its flip history; computed VP / industry-share overlay (from static state VP).
4. **Event timeline / feed** — the diff-derived "what everyone did" (conquests, focuses,
   tech, wars, builds/losses), filterable by country/type.
5. **Combat layer** — the Tier-C inferred combat heatmap + casualty attribution
   (explicitly inferred, not a literal battle log).

The map renderer + scrubber are already snapshot-driven (province-id raster recolored
per snapshot), so all of the above is the same data model at different cadences.

---

## 8. In-game mod (scope grows slightly)

- **Improved graphs** (kept) — the only thing visible in-game; auto-bind curves to
  humans, gross IC-output metric, autoscale, survives a major being annexed.
- **Event-marker log channel (REVIVED for replay)** — was cut as "not useful" under the
  old design; for a replay timeline it's worth it. Emit exact-timed `on_actions` markers
  (capitulation, war declared, focus completed, faction join) to `game.log` to sharpen
  the diff-derived event feed. Cheap, optional, complements (does not replace) the
  snapshot diffs. The mod can NOT export divisions/combat — that's the memory reader's job.

---

## 9. Build order (phased)

1. **Reader productionization + snapshot store** — merge the offset drafts (owner /
   economy / division) into the per-build layout; per-session relocation; SHA gate;
   fail-closed validation vs save. Persist the time series per game (the backbone every
   layer writes into).
2. **Save-parser breadth backbone** — parse the save EXHAUSTIVELY (focuses, tech,
   production, diplomacy, laws, equipment, stockpiles, resources, buildings, intel,
   active combats). This is the fastest route to "all the information": every §4b layer
   becomes available immediately at save cadence, tractable + patch-stable. Sets the
   schema the live cracks later fill in at higher cadence.
3. **Map + division time-lapse** — owner/control + divisions (positions/org/strength/
   equipment) over time; wire the existing viewer to live + archived snapshots.
4. **High-frequency COMBAT capture** (the marquee Tier-C work) — RE the live
   active-combat structures (land/naval/air); sample at max sustainable rate;
   reconstruct engagements (participants/location/duration/casualties/outcome) from the
   combat lifecycle + strength deltas; emit confidence/coverage.
5. **Live-layer cracks for cadence** — promote the slow/deep layers from save-cadence to
   live where it adds value (province-control overrides, fuel/manpower/factories,
   research & focus progress) via the proven save-anchored memory discovery; keep the
   save as validator.
6. **Diff engine → event timeline**; full **Overview leaderboard**, **Country deep-dive**,
   **Event feed**, and the **combat layer** in the UI; the (revived) **event-marker mod
   log** for exact-timed events; game library/archive polish.

---

## 10. Open defaults (decide at build time)

- Memory tick: daily default, hourly toggle; division layer auto-coarsen threshold.
- Save ingest: consume host autosaves; archive + melt + parse + delete raw on import.
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
  player real identity. Steam/real names are a §4 hard-impossible (no script or data
  access), so a seat is attributed by **TAG**, never by person; a substitution or a co-op
  partner is visible as *the tag is human-occupied*, not *which human*. Where the engine
  exposes nothing to distinguish two humans on one tag, that distinction is `null` (not
  guessed) — the no-fabrication rule, applied to seats.

The data stream is uninterrupted across a disconnect because the host never stopped.

### 11.3 Downtime gaps (crash / rehost / restart) — bridge with saves, else "no data"

The stream *does* break when the **host process** stops: a crash, a deliberate rehost, a
machine restart. Two mechanisms cover it:

1. **Saves bridge the gap.** Frequent host **autosaves** plus a **save-taken-before any
   planned rehost** give the save oracle (`S`) coverage across the downtime. The melted
   save re-anchors the world at its timestamp, so the replay resumes from real state, not
   a guess.
2. **The readers auto-reattach and self-relocate.** On the new `hoi4.exe` process the
   reader re-attaches, re-runs the **exe-SHA build gate**, and **rediscovers offsets from
   save invariants** for that session (§6). Offsets are per-session, so a rehost — even on
   a *patched* build — is handled by the normal relocation path, not a manual fix-up.
3. **Host-machine change is free.** Saves are portable, so a rehost on a **different
   person's machine** is bridged by the same save mechanism; the reader simply attaches to
   whoever is now hosting. (The build gate + offset profile is per host/build, recorded
   per segment — §11.7.)

**Honest residual:** a span covered by **neither** a memory snapshot **nor** any save is
marked **`no_data`** — never interpolated. If the host crashed at 1940.3 and the most
recent autosave was 1940.1, the 1940.1→1940.3 divergence is simply unobserved; the
replay shows the gap explicitly. Fabricating a smooth bridge would violate the core
contract.

### 11.4 Rollbacks — the SEGMENT + SUPERSEDE model

The hard case is a **reload from an earlier save**: in-game time goes **backwards**.
Naive `hour_index` ordering would interleave the discarded future with the new branch
and produce a nonsense timeline. The model:

- **`campaign_id`** identifies one playthrough. It is a **composite fingerprint**, not a
  single field: the save's `game_unique_id` / `game_unique_seed` (proven to identify a
  playthrough) **plus** start date, scenario/bookmark, mod-set checksum, and initial
  country roster. Seed match is **evidence, not identity** — a seed that reappears with
  *incompatible* start invariants is a collision and splits into a new campaign; a seed
  that changes but whose save lineage is otherwise clear gets an alias link. We store the
  **raw components**, not just the hash, so an ambiguous match can be re-decided later.
- **A SEGMENT = one continuous capture run** of a single authoritative source, bounded by
  a discontinuity (§11.5). Every snapshot is tagged
  **`(campaign_id, segment_id, reader_run_id, hour_index, captured_at, ingest_id,
  world_hash)`**.
- **Stitching:** within the canonical branch, order by `hour_index`. Across segments,
  resolve overlaps by **lineage, never by the clock** (§11.6).
- **Supersede:** when a reload's new segment **demonstrably replaces** an earlier branch's
  now-discarded future (an explicit save→load lineage edge: "segment B loaded from save X,
  which came from segment A at hour H"), the **later branch is canonical** and the earlier
  future is **demoted to an optional side-branch — kept, never deleted.** The rolled-back
  timeline stays browsable ("what would have happened"); it just isn't the main line.
- **Divergent overlap with no lineage edge = FORK, not supersede.** Same `campaign_id`,
  overlapping `hour_index`, *different* `world_hash`, and no established reload edge ⇒ we
  record a **fork conflict** and keep both branches, surfacing it for an operator/session
  call. We do **not** silently pick one. **Identical** overlap (same `world_hash`) just
  **dedupes**.

### 11.5 Detecting a segment boundary (multi-signal, not just "time went back")

`hour_index` **decrease** catches a plain rollback, but a reload can also jump **forward**
(reader stopped Jan 10 → host crashed → group reloaded a Jan 15 autosave → next memory
read is Jan 15, `hour_index` *increased* yet continuity is broken). So a boundary is
declared on **any** of:

- **process change** — new `hoi4.exe` pid / process-start-time, or first read after a
  reattach;
- **`hour_index` decrease** — classic rollback;
- **`world_hash` / checksum discontinuity at equal-or-forward time** — the world changed
  in a way forward simulation can't explain (a load, a console/save edit, manual mutation);
- **a large wall-clock gap** with missing expected ticks (host was down);
- **a save-load signal** (a fresh save appearing whose lineage doesn't continue the
  current segment).

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
2. **Save / reload lineage** — a proven load edge supersedes the branch it replaced
   (the strongest *automatic* rule).
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
- **Partial / corrupt / truncated save.** A rehost save can be half-written; rakaly can
  fail, or worse **succeed on incomplete data.** Saves move through an ingest state machine
  (`pending → stable → parsed_ok | parsed_invalid | quarantined`): **atomically copy + hash
  the file**, wait until size/mtime are **stable**, only then melt, then **validate
  invariants** (date/hour present, `game_unique_*` present, plausible country count, key
  world tables non-empty). A save that fails validation **cannot anchor offsets, cannot
  create a `campaign_id` / segment / lineage edge, and cannot supersede** — the span it
  would have bridged stays `no_data`.

### 11.8 Cadence & fail-closed (continuity knobs)

- **Frequent autosaves** + **save-before-rehost** are the continuity backbone — every save
  is a re-anchor point and a potential gap-bridge (§5 keeps daily forced saves *off*; these
  are normal autosave/end-of-session/pre-rehost saves).
- **Atomic save capture:** copy + content-hash each autosave **immediately** on appearance,
  because autosaves rotate/overwrite; the hash is the save's identity for lineage.
- **Fail-closed on patch:** if **offset rediscovery fails** after a build change, the memory
  layer **stops publishing** and the replay falls back to **save cadence** for that span
  rather than emit guessed data (§6). Continuity degrades to coarser-but-true, never to
  fabricated.
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
- **Segment record:** `parent_segment_id`, `parent_save_hash`, `segment_start_reason`,
  `authority_status`, and the per-segment **offset profile** (`exe_sha256` + mod checksum)
  so a patched rehost is self-describing.
- **Campaign record:** the **raw fingerprint components** (not just the hash) so a collision
  or alias can be re-adjudicated.
- **Store / timeline** become **segment-aware**: dedupe identical overlapping ticks by
  `(campaign_id, hour_index, world_hash)` — **segment-agnostic on purpose**, so the *same*
  state captured by two segments (or two readers) collapses to one (§11.6 step 4); resolve
  divergent overlaps by the §11.6
  precedence (supersede on lineage, else fork); expose the canonical branch to the scrubber
  while keeping rolled-back side-branches and fork branches **selectable, never dropped**.
- **`no_data` is first-class:** spans the canonical branch can't cover (no memory, no valid
  save) are explicit gaps in the timeline, surfaced in the UI — coverage, not silence.
