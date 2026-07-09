# REPLAY PRODUCTION PLAN — plan of record for the replay layer

**Status: PLAN OF RECORD (2026-07-09).** This document supersedes ad-hoc lane
queues as the authoritative statement of what remains between the current state
of `replay_interface/` and a production replay application. It is maintained by
the replay-layer orchestrator; every merged lane that closes an item updates its
row here in the same commit.

---

## 0. Mission and operating constraints

**Mission.** A production application that opens a `.battleplan` recording of a
multiplayer campaign and plays it back as a full-fidelity, read-only recreation
of the game map and UI — every visual parameter engine-derived (binary RE of
`hoi4.exe` + shipped installed content), no fitted guesses left unlabelled.

**Constraint 1 — FULL EXTRACTOR DECOUPLING (user directive 2026-07-09).**
The extractor agent will not have a usable recording any time soon. The replay
layer must therefore be finished **completely independently**:

- Every recording-gated surface (political ownership over time, units, weather,
  UI scalars, diplomacy, …) is developed and numerically gated against
  **synthetic fixture recordings** shaped exactly like `REPLAY_SCHEMA.md` /
  `REPLAY_DATA_REQUIREMENTS.md` prescribe.
- The only contract with the extractor is
  `documents/REPLAY_DATA_REQUIREMENTS.md` (standing task: every newly
  discovered runtime-state dependency is added there in the same commit that
  discovers it).
- **Integration is a separate, final phase** (Phase H) that begins only when
  the user hands over a recording worth using. If the fixtures are faithful to
  the schema, integration is a data-source swap at the `ReplaySource` seam in
  `replay_core`, not a rewrite.

**Constraint 2 — fixture-first, seam-respecting design.** All consumers read
world state through `replay_core` (`ReplaySource` → `WorldTimeline` →
typed accessor views such as `owner_by_state()`). No renderer or UI module may
parse recording bytes directly. Fixtures and real recordings are
indistinguishable to consumers.

**Constraint 3 — fail-closed honesty (standing).** `replay_app` blocks any
product path whose law has FIT/UNKNOWN residuals (13 `*_product_path_blocked`
gates in `main.rs` today). Production ≠ "unblock everything"; production =
close residuals with evidence until the blocks are *earned* off, and keep
fail-closed behavior for genuinely absent data. **No-eyeball rule:** only
pixel/byte parity vs engine-mined frames or oracle gates counts as verified.

**Constraint 4 — delegation.** All substantive work goes to Codex
(Mode A/B per `CODEX_PROTOCOL.md`); the orchestrator verifies every job
independently (own test runs, own pyghidra decompiles for binary claims, diff
scope review) before commit. Codex often cannot commit in linked worktrees
(sandbox excludes the main-repo `.git/worktrees/`); the orchestrator commits.

**Constraint 5 — mod compatibility.** All game-file reads go through
`mod_resolver` keyed by the recording's modset (`MOD_COMPATIBILITY.md`); no
spec may introduce a bare vanilla path join.

---

## 1. Current state (2026-07-09, branch `claude/replay-engine-extraction-18b40f`)

- **Tests:** 820 passed / 0 failed across `map_viewer`, `replay_app`,
  `game_ui` (incl. all pool/oracle gate suites).
- **Fidelity ledger** (`FIDELITY_LEDGER.md` §1): **76 FILE / 23 CAPTURE /
  38 FIT / 43 UNKNOWN** parameter rows. FIT+UNKNOWN = the burn-down (§3 there).
- **Parity oracle:** 38 vanilla political RenderDoc frames mined
  (zoom ladder); scoreboard mean |err| 0.014–0.03 at h>500, degrading to 0.135
  at h=50 — the close-zoom gap **is** the unrendered dynamic content (units,
  buildings, rails, shadows), i.e. Phase C in this plan.
- **Per-track completion estimate:** static map ~85–90 %; data spine (ingest →
  timeline → render keying) structurally present but thin; dynamic war content
  ~10–15 % (ledger row "Units/counters/rail/arrows": 1 FILE / 6 UNKNOWN); UI
  chrome layout law strong (8 FILE / 2 UNKNOWN) but binding/population barely
  started; playback UX minimal (timeline.rs + scrub tooling). Overall ~60 %.
- **Existing fixture substrate:** python `replay_interface/replay/` package
  (schema.py, store.py, diff_engine.py, snapshots/) — the natural fixture
  *generator*; Rust `replay_core` (source.rs 304 ln, world.rs 955 ln,
  tests/world_timeline.rs 709 ln) — `Leaf<T>` trichotomy,
  `owner_by_state`/`control_by_province` maps, `MapLayerSource`
  mirror-vs-walk. This seam is real and tested; Phase A widens it.
- **Extractor side (context only, not a dependency):** R1 hourly snapshots,
  D56 naval supremacy, D24 weather, D18 cosmetics, D31 air activity, D48
  building damage already landed in `extractor/` per the 2026-07-09
  reconciliation sweep — evidence the schema in REPLAY_DATA_REQUIREMENTS.md is
  being honored, which de-risks fixture fidelity.

---

## 2. The decoupling architecture (how every phase stays extractor-free)

```
                     ┌────────────────────────────────────────────┐
   extractor agent → │  .battleplan (real)     … arrives LATER    │
                     └────────────────┬───────────────────────────┘
                                      │  Phase H swap point
  fixture generator ─→ .battleplan (synthetic, REPLAY_SCHEMA-shaped)
                                      │
                              ReplaySource (replay_core/source.rs)
                                      │
                              WorldTimeline (world.rs)
                        typed views: owner/controller/cosmetic/
                        ideology/war/faction/weather/scalars/units…
                                      │
        ┌──────────────┬──────────────┼───────────────┬────────────┐
   political fill   borders GB    labels/furniture   units/counters  UI chrome
   (COL3 law)       (BORD/GB law) (NAME/FURN law)    (Phase C)       (game_ui)
```

**Fixture-recording rules:**
1. Fixtures are written by a deterministic generator (extend the python
   `replay/` package; seeded, versioned, committed as small binary/JSONL
   fixtures or regenerated in-test) — never hand-edited blobs.
2. Every fixture exercises a *specific consumer contract* and carries a
   scenario name (e.g. `fx_owner_flip_barbarossa`, `fx_cosmetic_raj`,
   `fx_war_matrix_1941`, `fx_topbar_scalars_sweep`).
3. A fixture is only trusted after a **schema-conformance gate**: the same
   validation code that will later reject a malformed real recording must
   accept every fixture (shared validator, not parallel implementations).
4. When REPLAY_DATA_REQUIREMENTS.md gains a field, the generator + a fixture +
   a conformance test gain it in the same commit.

---

## 3. Phases

Phases A–G are extractor-independent and can overlap; the dependency graph is
in §6. Phase H is deferred by directive.

### Phase A — Data spine: fixture generator + full-schema ingest (FOUNDATION)

Goal: `replay_core` can ingest a schema-complete recording (synthetic) and
serve **every** field in REPLAY_DATA_REQUIREMENTS.md through typed views, with
the trichotomy (value / inherited / unavailable) preserved.

| # | Item | Gate | Status |
|---|---|---|---|
| A1 | Fixture generator v1: extend `replay_interface/replay/` to emit schema-complete synthetic recordings (hourly snapshots + compact deltas, all container roles) | shared-validator conformance pass on every fixture | OPEN |
| A2 | Scenario fixture set: owner/controller flips, cosmetic-tag + ideology changes, war/faction matrix evolution, capital moves, weather sweep, topbar scalar sweep, division movement, naval/air activity, building damage | one named fixture per consumer contract; enumerated in a manifest | OPEN |
| A3 | `replay_core` ingest of every requirements item (1–10 + 7b/8b/9b): parse, index by time, expose typed views | round-trip gates: generator value == view value at every probed hour, incl. delta-reconstruction == snapshot cross-check | PARTIAL (owner/controller views exist, world_timeline tests green; most fields unserved) |
| A4 | Time semantics: hourly resolution, seek/scrub to arbitrary hour, forward/backward, mid-file start (snapshot + delta replay) | seek(h) == linear-play-to(h) equality gate on all fixtures | PARTIAL (timeline.rs scrub exists for captured moments) |
| A5 | Renderer keying swap: political fill/borders/labels/furniture read owner+controller+cosmetic from WorldTimeline instead of static history parse | pixel-identical output at t0 vs current static path on the 1936 baseline; then divergence gates on flip fixtures (expected-diff oracles) | OPEN — **this is the single highest-leverage item in the plan** |
| A6 | Fail-closed data absence: every view distinguishes "field absent from recording" and renderers surface it per Constraint 3 | absence-injection fixture gates | PARTIAL (Leaf trichotomy exists) |

### Phase B — Static-map residual closure (burn-down of FIT/UNKNOWN)

Goal: the FIDELITY_LEDGER §3 top-10 items that are *offline-crackable* go to
FILE/CAPTURE-LAW; capture-gated ones get their capture executed (§4) and then
closed. Product blocks earned off one by one.

Offline (Codex jobs, no user session needed):
| # | Item | Gate | Status |
|---|---|---|---|
| B1 | 60 water-strip residuals (sea|sea smoothing tail) + ~277 island loops + fade interior curve → unblock SeaRegion + `--borders-from-files` | pool gate 8,560/8,560 water, 27,018 land held; loops traced | OPEN (BORD9 got 8,500/8,560) |
| B2 | Label residuals: component-emission limits, kerning table, anchor rule; glyph atlas mips | oracle63 float-exact held + new held-out capture rows | OPEN (arc/size/tracking laws closed) |
| B3 | Building `.mesh` importer or map_objects_pass derivation → replace preview blocks | byte parity vs captured object pass | OPEN |
| B4 | Tree scatter anchor/rotation/model-selection + season CB1 from files | parity vs live_trees_measure corpus | OPEN |
| B5 | River class-3 width ramp + palette semantics | defines/shader citation or capture | OPEN |
| B6 | Remaining ledger tail (#16–21): moon `.z`, height scale, PCF/shadow path, occupation stripes, camera interpolation shape | per-row cited closure | OPEN, lower priority |

Capture-dependent (blocked on §4 sessions, then a closing Codex job each):
B7 terrain vSnap frustum (needs §4-C1) → 170/176 target; B8 night `FoW.w`
path + night `.x/.yzw(hour)` (§4-C3); B9 unclamped exposure branch (§4-C4);
B10 label owner-vs-controller keying + per-label `Transp_OffsetX.x` (§4-C5);
B11 VP color columns 1/3 confirmation (§4-C2); B12 no-FOW sea base look;
B13 snow/mud — **recording-fed** (SNOW_LAW §6), so its product path rides
Phase A fixtures, not a capture.

### Phase C — Dynamic war content (the biggest remaining body)

Goal: everything that makes a *war* replay legible — currently 1 FILE /
6 UNKNOWN. All of it is fixture-drivable (positions/strengths come from the
recording); the RE work is the *rendering law*, which is engine-derived and
extractor-independent.

| # | Item | Law source | Gate | Status |
|---|---|---|---|---|
| C1 | Unit counters (NATO/sprite): quad geometry, atlas, strength/org bars, stacking/aggregation-by-zoom | hoi4.exe RE + interface gfx files + one live counter capture (§4-C6) | pixel parity vs captured counter frame; fixture-driven placement gates | OPEN |
| C2 | Division/fleet/wing positions over time served from recording; interpolation between hourly samples matches engine hourly snap (no invented smoothing without evidence) | REPLAY_DATA_REQUIREMENTS units items + fixtures | seek==play equality; placement matches fixture ground truth | OPEN (schema listed) |
| C3 | Battle bubbles / combat indicators | RE + capture | parity gate | OPEN |
| C4 | Front lines + movement/plan arrows (or explicit descope for v1 if not in recording — arrows are player-plan state; **decision needed, tracked in REPLAY_DATA_REQUIREMENTS**) | RE + requirements | parity or descope note | OPEN |
| C5 | Rail/supply overlay | RE + defines | parity gate | OPEN |
| C6 | Sea-zone naval-supremacy tint (recording R6/D56 shaped) | GB-field law + fixtures | parity vs captured supremacy frame | OPEN |
| C7 | Close-zoom 3D pass completion (units/buildings/rails/shadows) measured by the existing oracle: h=50 scoreboard mean |err| 0.135 → ≤0.03 | all of C1–C6 + B3/B4 | RenderDoc-oracle scoreboard at low h | OPEN — **this is the definition-of-done metric for Phase C** |

### Phase D — UI chrome completion (game_ui)

Goal: the full read-only UI shell rendered and *populated from the timeline*.
Layout law is strong (GetGlobalPosition verbatim, listbox/gridbox/scrollbar
law); the gap is bindings + population + remaining widgets.

| # | Item | Gate | Status |
|---|---|---|---|
| D1 | Topbar bindings: politics/stability/war-support/factories/convoys/fuel/nukes/tension… bound to WorldTimeline scalar views (9/9b) | fixture sweep renders every scalar; layout unchanged vs .gui law gates | OPEN (layout law done) |
| D2 | Ruling-party/ideology-driven texts + flag/portrait keying (7b, LAB4 `<TAG>_<ideology>` localisation) | fixture ideology-flip changes rendered text correctly | OPEN |
| D3 | Outliner (army/navy/air lists) populated from unit views | listbox law gates + fixture population counts | OPEN |
| D4 | Country/diplomacy panels (read-only): relations, wars, factions from 8/8b views | fixture war-matrix renders correct rows | OPEN |
| D5 | Scrollbar thumb-travel law (UNTRACED, fail-closed) + any remaining widget primitives (checkbox/tab states as static chrome) | RE citation or capture (§4-C7 open-panel frame) | OPEN |
| D6 | Date/speed clock — replay-native (shows recording time; speed = playback speed) | unit gates | PARTIAL (freeze-clock tooling exists) |

### Phase E — Playback UX (the product shell)

| # | Item | Gate | Status |
|---|---|---|---|
| E1 | Transport controls: play/pause, speed steps, seek bar with date ticks, jump-to-date | seek==play gate reused from A4; input-latency budget | OPEN (scrub tooling exists, product-blocked) |
| E2 | Free camera with engine pan/zoom law (CAMDEF1 clamps landed; interpolation shape pending B6) | camera law gates | PARTIAL |
| E3 | Map-mode switching (political first; terrain/naval as laws allow) — colors computed replay-side from inputs (NEVER recorded; standing override) | per-mode parity gates | PARTIAL (political is the lane) |
| E4 | Recording open/validate UX: schema validation errors, modset mismatch surfacing (mod_resolver), version handshake | malformed-fixture rejection matrix | OPEN |
| E5 | Session ergonomics: window resize, monitor DPI, save-position — plain Win32 hygiene | smoke gates | OPEN |

### Phase F — Performance & hardening

| # | Item | Gate | Status |
|---|---|---|---|
| F1 | Playback frame budget on the weak-PC mandate: full political map + labels + furniture + counters at interactive rates; measure, then optimize (no speculative work) | frame-time budget gate in headless bench | OPEN |
| F2 | Timeline memory: bounded working set for multi-year hourly recordings (snapshot spine + delta windows, LRU) | synthetic 3-year fixture under memory ceiling | OPEN |
| F3 | Long-run soak: 8-hour scrub/play loop, zero leaks/device-removed | soak harness | OPEN |
| F4 | Crash/error policy: fail-closed startup diagnostics already exist; extend to mid-playback data anomalies (log + hold-last-good, never invent) | anomaly-injection fixtures | OPEN |

### Phase G — Acceptance & release QA

| # | Item | Gate |
|---|---|---|
| G1 | Full-ladder pixel parity vs the 38-frame RenderDoc oracle: mean |err| ≤0.03 at ALL heights incl. h=50 (non-snow frames per the winter confound) | scoreboard |
| G2 | Fixture end-to-end: open synthetic 1936–1945 recording → play → scrub → every surface updates; recorded as a scripted headless run with frame hashes | e2e hash gate |
| G3 | Fidelity ledger: zero unlabelled FIT in any product-path parameter; every remaining UNKNOWN either product-blocked or explicitly descoped in this doc | ledger audit |
| G4 | Full test suite green + Codex adversarial release review (read-only, xhigh) | 100 % pass + review disposition log |

### Phase H — Integration (DEFERRED until a real recording exists)

Explicitly out of scope until the user delivers a worthwhile `.battleplan`:
swap `ReplaySource` input to the real file; run the same conformance validator;
diff real-vs-fixture field coverage; reconcile gaps back into
REPLAY_DATA_REQUIREMENTS.md; re-run G1–G4 on the real recording. If Phases A–G
held the seam, H is days, not weeks.

---

## 4. Consolidated capture-session asks (user-run live sessions; batch them)

Each ask is one frame/short session; several can share a session. These gate
Phase B/C/D closures but nothing in Phase A/E/F.

| # | Ask | Closes |
|---|---|---|
| C1 | Single-frame, NO-pan, wide-frustum mixed-LOD terrain capture recording render-view side planes / tessellation visibility flags (signed-byte owner map) | B7: vSnap 146→170/176 |
| C2 | Player-POV political frame with enemy-owned + same-faction VPs on screen | B11: VP diplomatic color columns 1/3 |
| C3 | Game-speed night capture with `FoW.w≠0` + multi-hour night sweep | B8: night look + `.x/.yzw(hour)` |
| C4 | Bright close-up `restore_pass` frame exceeding the 0.5 exposure floor | B9: unclamped exposure branch |
| C5 | Multi-zoom label captures over occupied territory (owner≠controller) | B10: label keying + per-label alpha |
| C6 | Close-zoom frame with land units + counters visible (and a naval stack) | C1: counter geometry/atlas ground truth |
| C7 | Frame with an open scrollable panel (thumb positions at known content sizes) | D5: scrollbar thumb travel |
| C8 | No-FOW sea frame | B12: sea base look referee |
| C9 | Tree instance buffer + cities_pass (standing ledger asks) | B3/B4 |

---

## 5. Standing extractor-requirements process (unchanged by decoupling)

`REPLAY_DATA_REQUIREMENTS.md` remains the single handoff artifact. Rules:
every lane/job that discovers a new runtime-state dependency adds it (plus an
EXTRACTION_NEEDS §A row) in the same commit; Codex job prompts for render/UI
work carry this rule verbatim; the fixture generator (A1) mirrors the doc, so
schema drift is caught by the shared validator, not by integration day.

## 6. Dependency graph / execution order

- **A1→A2→A3→A5** is the critical path; nothing in C/D/E ships without A.
  A4/A6 ride alongside A3.
- B offline items (B1–B6) are parallel Codex lanes, independent of A.
- B capture items (B7–B12) wait on §4 sessions — schedule the session early;
  the closing jobs are quick once evidence exists.
- C needs A3 (unit views) + its own RE lanes; C7 (the h=50 oracle metric)
  additionally needs B3/B4.
- D needs A3 (scalar/diplomacy/ideology views); D5 needs §4-C7.
- E needs A4 (seek) and is otherwise independent; F needs A+C volume to be
  meaningful; G last; H deferred.
- Suggested wave 1 (now): A1+A2 (one Codex lane), A3 unit/scalar views
  (second lane), B1 water residuals (third lane), C1 counter RE prep from
  installed content (fourth lane) — all disjoint files.

## 7. Definition of production

1. All Phase G gates green on synthetic recordings.
2. Every `*_product_path_blocked` gate either earned off with cited law or
   documented here as a v1 descope with the block retained.
3. Ledger: 0 unlabelled FIT on product paths; UNKNOWN only where descoped or
   recording-fed-and-fixture-proven.
4. Weak-PC frame budget met (F1) and 8-hour soak clean (F3).
5. REPLAY_DATA_REQUIREMENTS.md current — so Phase H can start the day a real
   recording lands.

## 8. Top risks

| Risk | Mitigation |
|---|---|
| Fixture drift from what the extractor actually writes | shared validator (A1 rule 3) + the reconciliation sweeps against `extractor/` source (read-only) like 2026-07-09's |
| Units/counters RE turns out capture-heavy | front-load §4-C6 into the next live session; game interface gfx files first per game-files-first directive |
| Arrows/plans not in recording (player intent, not world state) | C4 decision: descope for v1 unless the extractor confirms availability |
| Close-zoom parity target (≤0.03 at h=50) unreachable without shadows/particles | treat particles as v1 descope candidate; shadows are ledger #21 with an engine-law lane |
| Capture sessions delayed | nothing on the A/C/D/E critical path waits on them; only B residual closure does |
