# LANE REPORT — lane/d-ui-bindings (Plan items D1 + D2)

**Scope.** Bind the game topbar UI to the recording timeline (D1) and key the
ideology-driven flag / display-name texts off the timeline (D2). Layout law was
already DONE; this lane adds the **population** seam only — no layout change.

**Seam.** New `game_ui/src/binding.rs` maps `replay_core`'s typed `WorldFrame`
views (`TopbarScalars`, `CountryIdentity`) onto the layout-only `TopbarBinding`
and the ideology-keyed flag/name candidate lists. `game_ui` now depends on
`replay_core` (no cycle: `replay_app → game_ui → replay_core`, and
`replay_app → replay_core` already existed). No recording bytes are parsed in
`game_ui`; every value comes through a typed view (Constraint 2). Absent /
unobtained leaves render the fail-closed absent form (`None`), never a default
(Constraint 3).

`replay_core`, `extractor/` UNTOUCHED (read-only, per constraints).

---

## Gate results (numeric/string-exact only)

| Gate | Test | Result |
|---|---|---|
| **G1** Topbar scalar sweep == generator value at every probed hour, 3 POVs (GER/ENG/SOV) | `game_ui/tests/topbar_binding.rs::topbar_scalar_sweep_renders_generator_values` | **PASS** (12 hours × 3 POVs; integer-valued scalars string-exact vs generator formula, 2 fractional scalars pinned ±1e-9) |
| **G2** Ideology/cosmetic flip re-keys flag + display-name candidates at the exact flip hour | `…::ideology_flip_rekeys_flag_and_name_at_flip_hour` | **PASS** (fascism→democratic@t4→neutrality@t8; flag+name candidate lists exact; hour3≠hour4) |
| **G3** Existing game_ui layout gates HELD (bindings only) | `topbar_quads`, `flag_precedence`, `widget_layout`, `topbar_rects_follow_engine_layout_law`, `acceptance` | **PASS** (unchanged) |
| **G4** Unavailable views render the fail-closed absent form | `…::absent_pov_renders_fail_closed_binding` + in-sweep `world_tension`/`supply` = `None` | **PASS** (absent POV → all-`None` scalars + tag-only flag; world_tension/supply `None` not `0`) |

**Test counts (exact):**
- `cargo build -p game_ui` — clean.
- `cargo test -p game_ui` — **62 passed / 0 failed** (lib unit 52 incl. +3 new
  `binding` unit tests; `topbar_binding` 3; `flag_precedence` 1; `markup_law` 2;
  `topbar_quads` 1; `widget_layout` 3) + `acceptance` custom harness exit 0.
- `cargo test -p replay_core` — **13 passed / 0 failed** (untouched).
- `cargo clippy -p game_ui --tests` — no warnings in the new files
  (pre-existing warnings elsewhere unchanged).

---

## Binding coverage table (topbar field → view → schema path → gate)

Cleanly bound — the `TopbarScalars`/`CountryIdentity` view path matches the
fixture/schema, gate green:

| TopbarBinding field | replay_core view field | schema path read by view | Format class | Gate |
|---|---|---|---|---|
| `political_power` | `TopbarScalars.political_power` | `politics.political_power` | UNKNOWN → raw | G1 |
| `stability_percent` | `.stability` | `politics.stability` | UNKNOWN → raw (fraction, ±1e-9) | G1 |
| `war_support_percent` | `.war_support` | `politics.war_support` | UNKNOWN → raw (fraction, ±1e-9) | G1 |
| `command_power` | `.command_power` | `politics.command_power` ‖ `military.command_power` | UNKNOWN → raw | G1 |
| `manpower` | `.manpower_available` | `military.manpower.available` | UNKNOWN → raw | G1 |
| `army/navy/air_experience` | `.xp_army/.xp_navy/.xp_air` | `military.xp.army/navy/air` | UNKNOWN → raw | G1 |
| `factory_counts` (civ/mil/naval) | `.factories_civ/.factories_mil/.factories_dockyard` | `economy.factories.civ/mil/dockyard` | struct (no string fmt) | G1 |
| `fuel` | `.fuel_current` | `economy.fuel.current` | UNKNOWN → raw | G1 |
| `nukes` | `.nukes_total` | `extra.nukes_total` | UNKNOWN → raw; **total-vs-ready ambiguous** | G1 |
| `country_ideology` | `CountryIdentity.ruling_ideology` | `politics.ruling_party` | key | G2 |
| `cosmetic_tag` | `CountryIdentity.cosmetic_tag` | `extra.cosmetic_tag` | key | G2 |
| flag key candidates | `topbar_flag_key_candidates()` | `gfx/flags/medium/…` precedence | FILE (UI_LAYOUT_LAW §2) | G2 |
| display-name key candidates | `country_name_key_candidates()` | LAB4 `<cosmetic>_<ideo>→<cosmetic>→<tag>_<ideo>→<tag>` | FILE (FIDELITY_LEDGER label law) | G2 |

---

## replay_core API gaps (drive the replay_core / fixture-gen follow-up)

These are **consumer-side view/path mismatches**, NOT new recording
dependencies — every field is already specified in
`REPLAY_DATA_REQUIREMENTS.md §9b`. The fields exist in the fixture, but the
`TopbarScalars` view either reads a different path than the generator writes, or
has no field at all. The binding faithfully renders whatever the view serves and
fails closed where the view is empty.

| Field | View reads | Generator/fixture writes | Effect on topbar | Fix owner |
|---|---|---|---|---|
| **convoys** | `military.convoy_count` then Σ`military.convoys[].amount` | per-tick `military.convoys_count` (+ base `convoys[].amount=50`) | renders **50** (base array), NOT the per-tick 61 | replay_core `leaf_convoys` path OR fixture-gen key |
| **world_tension** | `world.world_tension` / `world.tension` / `world_events.world_tension` | `world.world_tension_inputs.fixture_topbar_world_tension` | **Absent → `None`** (blocked badge) | reconcile canonical `world.world_tension` scalar (schema §9b + fixture-gen + view) |
| **supply_value / supply_ratio** | `economy.logistics.supply_value` / `.supply_ratio` | `countries[tag].scalars.supply_value` / `.supply_ratio` | **Absent → `None`** | replay_core view path OR fixture-gen key |
| **ratio bars** (`industrial_ratio`, `convoys_ratio`, `army/navy/air_xp_ratio`) | *no `TopbarScalars` field* | `countries[tag].scalars.industrial_ratio/convoy_ratio/xp_ratio` | not bound (bars stay default) | **add typed view fields** to `TopbarScalars` |
| **fuel_ratio bar** | derivable: `fuel_current/fuel_max` (both in view) | — | left unbound (derivation not wired) | game_ui follow-up (compute from view) |
| **alerts** (initiative, decisions avail/timeout, intel, prototype, market contracts) | *no view field* | `countries[tag].scalars.alerts.*`, `decisions.status_present.*` | not bound | **add typed view fields** (requirements §9b lists them) |

**No `REPLAY_DATA_REQUIREMENTS.md` / `EXTRACTION_NEEDS.md §A` change made:** all
gapped fields are already enumerated in §9b (topbar scalar set + the extension
paragraph). The gaps are view-path/typed-view reconciliation, not missing
recording data.

---

## UNKNOWN formatting rows (Constraint 3 — cited or raw)

No shipped `.gui`/localisation citation was found for topbar **number**
formatting in this decoupled context, so every scalar is rendered as the **raw
measured value** (`format!("{}")` of the typed leaf) — no invented percent
scaling, thousands separators, rounding, or +/- coloring. Replace per-field when
a citation lands.

| Field(s) | UNKNOWN aspect | Raw rendering used |
|---|---|---|
| `stability_percent`, `war_support_percent` | the game shows a `%` (×100); scaling+`%`-suffix uncited | raw fraction (e.g. `0.5`) |
| `political_power`, `command_power` | integer display vs stored f64; rounding uncited | raw f64 (`33`, `0.0`-frac if any) |
| `manpower`, `fuel` | the game uses compact `K`/`M` + grouping; uncited | raw integer / raw f64 |
| `nukes` | total-vs-ready which-count is uncited (bound to `nukes_total`) | raw integer |
| `factory_counts` `/`-join text | the `civ/mil/naval` slash join is uncited | struct fields bound; join left to compose |

FILE-cited (kept): flag folder+precedence (`UI_LAYOUT_LAW §2`,
`flag_candidates_for_size`, `gfx/flags/medium`) and the LAB4 display-name key
precedence (`FIDELITY_LEDGER` label law).

---

## Out of scope / deferred (noted, not done)

- **date + speed/pause** (topbar clock) — D6; `frame.date()` +
  `session_metadata.speed_events` exist but are intentionally not bound here.
- **compose-time `0` default** — `topbar.rs::apply_binding` still substitutes
  `"0"`/`"0%"` for a `None` field at *compose* time (pre-existing). The
  binding-level fail-closed form is `None` (gated). Making the composited text
  render an absent badge instead of `0` is a small follow-up kept out to avoid
  touching layout/compose behavior in a bindings-only lane.
- **Outliner / diplomacy panels** — D3/D4.
