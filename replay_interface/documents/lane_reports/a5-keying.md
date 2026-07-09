# LANE REPORT — A5 renderer keying swap (lane/a5-keying)

Plan item **A5** (REPLAY_PRODUCTION_PLAN.md §3 Phase A): swap the renderer's
keying SOURCE so political fill, borders, map labels, and map furniture read
**owner + controller + cosmetic-tag** from the `replay_core` `WorldTimeline`
typed views instead of the static 1936 history parse.

Status: **DONE** — adapter + timeline-keyed law entry points landed; all three
plan gates green; no product blocks weakened; no `replay_app/src/main.rs`,
`extractor/`, or fixture edits.

---

## 1. The keying seam (found)

All four consumers ultimately key off one map plus two side channels:

| Consumer | Owner lookup | Controller | Cosmetic | Pure law fn | Resolver entry |
|---|---|---|---|---|---|
| Political fill | `state_owners` + `InitialStateControl.owner` | `InitialStateControl.controller` (per-state) | via `country_colors` (`apply_cosmetic_color_overrides`) | `political_texel_colors` (political_color_synthesis.rs:509) | `synthesize_political_colors` |
| Borders (GB rank) | `BorderSynthesisInputs.state_owners` | — | — | `classify_border_edge` | `synthesize_border_world_mesh_from_resolver` |
| Labels | `LabelLawInputs.owners` | — | — (caller) | `compute_country_label_placements` | dump bin (`bundle.owners`) |
| Furniture (VP icons) | `FurnitureLawInputs.state_owners` | — | — | `compute_furniture_markers` | `furniture_layer_from_resolver` |

Single choke point: `state_owners: BTreeMap<StateId, Tag>` from
`game_data::load_states`. Controller enters only political fill via
`load_initial_state_controls` (state-level owner/controller). Cosmetic color
enters only political fill via `load_country_colors_from_game_resolver` →
`country_colors`. The pure law functions already accept the maps as arguments;
only the resolver-level wrappers hard-parsed the static files. So the swap is a
**source swap at the map-construction boundary**, not a rewrite of the laws.

## 2. Adapter design

New module **`map_viewer/src/keying.rs`** — `pub struct KeyingTables`
(`state_owners`, `state_controls: BTreeMap<u32, InitialStateControl>`,
`cosmetic_by_tag`), source-neutral. Two producers:

- Static (unchanged parse) — the existing `load_states` /
  `load_initial_state_controls` / cosmetic parse still feed the original
  `*_from_resolver` functions.
- **`KeyingTables::from_world_frame(&replay_core::WorldFrame)`** — reads the
  typed `owner_by_state()` / `control_by_state()` / `country_identities()`
  cosmetic views. **Fail-closed (Constraint 3):** only `Leaf::Measured` values
  enter the tables; `Unobtained`/`Absent` owner/controller/cosmetic is omitted,
  never coerced.

`map_viewer` gains a normal dependency on `replay_core` (acyclic —
`replay_core` depends only on serde/serde_json/snapshot). This is the literal
realization of "the renderer reads from `WorldTimeline` typed views".

**Timeline-keyed law entry points (new, additive, product API):**
- `synthesize_political_colors_with_keying(resolver, &KeyingTables)` — full
  three-key path (owner + state controller + cosmetic color). Shares the
  extracted `render_political_texels` loop with the static
  `synthesize_political_colors`, so the two paths are **byte-identical by
  construction** when fed the same keying. Topology (province→state, impassable,
  base colors, raster) stays file-derived — it is not recording state.
- `synthesize_border_world_mesh_with_keying(resolver, &KeyingTables)` — swaps
  `state_owners` (border rank is owner-only).
- `furniture_layer_with_keying(resolver, language, &KeyingTables)` — swaps
  `state_owners` (via a small `..._keyed(owner_override)` refactor).
- Labels consume `KeyingTables::owner_map()` at `LabelLawInputs` construction
  (label input assembly lives in the dump bin, so there is no single resolver
  wrapper to swap — the seam is the `owners` field).

## 3. Gates (all green — table-equality / output-equality / expected-diff; no eyeballing)

Located in `map_viewer/src/keying.rs` `#[cfg(test)]` (crate-internal so they can
drive the real `replay_core` `WorldTimeline` via an inline `ReplaySource` and
load the packed flip fixtures). Fixture oracles derive from the generator
SOURCE `replay_fixture_gen/src/main.rs`, never from code under test.

- **Gate 1 — t0 equivalence** (2 tests):
  - `gate1_from_world_frame_table_equality`: `from_world_frame` on a known
    snapshot yields exactly the expected owner/control/cosmetic tables
    (incl. an explicit-null cosmetic omitted).
  - `gate1_fill_output_equivalence_same_data`: timeline-derived vs hand-built
    static `KeyingTables` carrying identical data produce **byte-identical**
    `political_texel_colors` output (class + owner + secondary color) — the
    keying→pixel law. (Full-texture parity is a compile-time guarantee via the
    shared `render_political_texels` loop.)
- **Gate 2 — divergence on the flip fixtures** (3 tests, expected-diff oracles):
  - `gate2_owner_controller_flip_divergence` (`fx_owner_controller_flip`):
    scrubbing tick4→5 changes **exactly {state 10}** owner (GER→SOV) and its
    controller; tick8→9 changes **exactly {state 20}** controller (SOV→GER)
    with **no owner change**; across the whole timeline the only owner change is
    state 10. Nothing else moves.
  - `gate2_cosmetic_flip_divergence_reaches_fill` (`fx_cosmetic_ideology`): GER
    cosmetic `null`(tick3, omitted) → `GER_civil_war_democratic`(tick4) →
    `GER_reichskommissariat`(tick8); no other tag ever carries a cosmetic; the
    flip reaches the political-fill **owner color** (base → green → red via a
    synthetic cosmetic color map), while SOV's color is stable.
  - `gate2_owner_flip_reaches_labels_and_furniture`: on a 200×100 raster,
    flipping state 10 GER→SOV drops GER's label and re-keys its VP marker
    (owner GER→SOV), leaving state 20 untouched.
- **Gate 3 — trichotomy honesty** (1 test):
  `gate3_unobtained_owner_renders_absent_not_stale`: an `Unobtained` (explicit
  null) state owner is omitted from the tables and the fill law classifies it
  `NoOwner` with **no owner color** — the absent form, never a stale/default.

## 4. replay_core additive change (coordinator ask, in scope)

`SessionMetadataView.player_countries` read the top-level `session.player_countries`
key that no recording/fixture carries — the player list lives at
`world.scenario.players` (`{tag, slot, label}`). The view returned `Leaf::Absent`
on every fixture, failing the observer-POV country picker closed.

Fix (`replay_core/src/world.rs`): `build_session_metadata` now falls through to
`world.scenario.players`, `leaf_player_countries` is generalized over
(object, key, base) and also reads `label` as a player-name fallback. Trichotomy
preserved: genuinely absent recordings still report `Absent`;
domain-unavailable still reports `Unobtained`. New round-trip gate
`rt_player_countries_from_world_scenario_players` in `tests/roundtrip_gates.rs`
(oracle = generator `base_snapshot`), asserting the view over all 12 positive
fixtures at every hour.

## 5. Gate numbers

- `cargo test -p replay_core` — **31 passed / 0 failed** (was 30; +1
  player-countries gate; roundtrip_gates 17→18).
- `cargo test -p map_viewer` — **637 passed / 0 failed / 1 ignored** (+6 A5
  keying gates; lib binary 624 passed).
- `cargo build -p map_viewer` / `cargo build -p replay_app` — clean, no new
  warnings in touched files.
- t0 equality: owner/control/cosmetic tables and `political_texel_colors`
  output byte-equal across paths.
- Divergence sets: owner `{10}`@tick5 + control `{10}`@tick5, control `{20}`@tick9
  (owner unchanged); cosmetic `{GER}` only; label/marker re-key on the flipped
  state only.

## 6. Requirements / extraction deps

**No new recording-side dependencies.** All keying reads existing schema fields
(owner_by_state / control_by_state = requirements items 1/2; cosmetic_tag = 7b;
players = item 4). No `REPLAY_DATA_REQUIREMENTS.md` / `EXTRACTION_NEEDS.md` edit
needed.

## 7. Open residuals

- **Furniture capital keying** is still static (`history/countries capital =`).
  Recording-driven capital moves (`fx_capital_moves`, requirements diplomacy
  `capital_state`) are available in the timeline but not yet wired into
  `capital_states`; `furniture_layer_with_keying` swaps owner only. Small
  follow-up: source `capital_states` from `WorldFrame.diplomacy` capital view.
- **Province-granular controller.** `control_by_province` exists in the view but
  no current consumer keys per-province; the political secondary (occupation)
  color is state-controller-keyed as in the static path. Province-granular
  occupation rendering would consume it — deferred (no consumer today).
- **Label resolver wrapper.** Labels are keyed via `KeyingTables::owner_map()`
  at input construction; there is no standalone `_with_keying` resolver because
  label assembly lives in the dump bin. Product wiring of the label input from
  the timeline is a Phase D/E integration step.
- Product-path wiring into `replay_app`'s render loop is intentionally NOT done
  here (main.rs product blocks are off-limits); the `*_with_keying` functions
  are the earned seam the app will call.
