# Lane M1b — States / Ideology / Players map modes

**Branch:** `lane/m1b-states-ideology-players`
**Deliverable:** `replay_interface/native/map_viewer/src/map_mode_admin.rs` (NEW,
sole owner) + `lib.rs` exports. New-map-mode sibling of `naval_supremacy_tint`.
Mission: derive the three "law-known" engine hard-coded layer modes from
`MAPMODE_COVERAGE_MATRIX.md` §2 — **States (13)**, **Ideology (16)**,
**Players (15)** — as replay-side data law, decoupled from `replay_core`.

## Gate numbers

- `cargo build -p map_viewer` — **green**.
- `cargo test -p map_viewer --lib` — **709 passed / 0 failed / 1 ignored**
  (baseline 698 + **11 new** `map_mode_admin` tests). With `BATTLE_PLAN_GAME_DIR` set the
  vanilla file gate runs and passes (byte-match shipped files); without it that
  gate skips — both configs green.
- `cargo test -p replay_core` — **green** (36 tests; unchanged, READ-ONLY).
- All `map_viewer` test targets compile (`cargo test --no-run` clean).

The 11 new tests: `defines_parse_matches_vanilla_constant`,
`ideology_colors_parse_matches_vanilla`, `states_fill_owner_color_fail_closed`,
`states_fill_equals_political_owner_color_bytewise` (drives `political_texel_colors`
and asserts States fill == political owner colour BYTE-WISE, incl. unowned→NoOwner),
`state_boundary_outline_marks_only_state_edges`,
`state_boundary_outline_single_state_has_no_edges`,
`ideology_fill_law_and_fail_closed`, `players_fill_fade_law_and_fail_closed`,
`gate_fixture_ideology_flip_reaches_fill` (opens `fx_cosmetic_ideology` via
replay_core), `gate_fixture_players_session_and_fade` (opens
`fx_owner_controller_flip` via replay_core),
`gate_vanilla_admin_defines_and_ideology_colors` (BATTLE_PLAN_GAME_DIR).

## Per-mode law + citations

### States (mode 13, `game_map_mode_states`)
- **Fill = FILE.** Owner country colour — the SAME owner colour the political fill
  emits (`political_texel_colors` owner branch; COL3). `game_map_mode_states`
  layer `type = state` "will render using state provinces"
  (`common/map_modes/documentation.md`). Owner from the `owner_by_state` view
  (item 2) via the A5 keying seam (`keying.rs` `KeyingTables::state_owners`) —
  reused, not re-parsed.
- **State outlines = FILE (NEW).** A state-boundary edge is a 4-neighbour pixel
  pair whose provinces resolve to *different* states, derived purely from
  `map/provinces.bmp`→province id + `history/states/*.txt` province→state. Sea/
  lake/unknown provinces (no state) never start a state boundary (matches the
  political `NoState` transparency). Deterministic pure fn `state_boundary_outline`.
- **Render/pixel-parity = capture-gated (deferred)**, same posture as naval C6.

### Ideology (mode 16, `game_map_mode_ideology`)
- **4 colours = FILE** (`common/ideologies/00_ideologies.txt`):
  democratic `{0,0,255}` (:30), communism `{255,0,0}` (:106),
  fascism `{150,75,0}` (:173), neutrality `{124,124,124}` (:237).
- **Alpha = FILE** `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY`
  (`common/defines/00_graphics.lua`, vanilla `1`). Its comment is the confirming
  citation: *"the colors on the map are taken from the scriptable file
  00_ideologies.txt for each group … a variable to control the transparency of
  the color."*
- **Keying = FILE.** Each state coloured by its owner's ruling ideology
  (`CountryIdentity::ruling_ideology`, item 7b, which the view derives from
  `countries.<tag>.politics.ruling_party`). The four ruling-party names ARE the
  ideology group names, so ruling_party keys the colour table directly.
- Fail-closed on absent owner / absent ruling ideology / unknown ideology name.

### Players (mode 15, `game_map_mode_players`)
- **Player↔country = FILE** from the session view
  (`SessionMetadataView::player_countries`, item 4 = `world.scenario.players[*].tag`).
- **Colour rule = FILE.** A player-held country keeps its country colour at full
  opacity; a country **not** held by any player is faded to
  `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY` (`00_graphics.lua`, vanilla
  `0.15` → alpha byte 38). Its comment: *"How much is the country colors faded out,
  for countries that are not occupied by the any player."* The players mode is the
  political fill with non-player countries alpha-faded — **not** a distinct
  per-player palette.
- **UNKNOWN (not used):** any MP-lobby per-slot palette beyond the country colour
  is engine/lobby-runtime state, not shipped-file law, and is not needed for the
  file-derivable mode. Documented, not fitted.

## Fixture-gate provenance (expectations from generator SOURCE, not code-under-test)
- Ideology: `replay_fixture_gen/src/main.rs::fx_cosmetic_ideology` mutates GER's
  `politics.ruling_party` by tick — tick<4 fascism, 4..=7 democratic, ≥8
  neutrality; GER owns state 10 (base_snapshot). The gate opens the real fixture
  through `replay_core`, reads `ruling_ideology`, feeds my fill, and asserts the
  state-10 colour flips fascism→democratic→neutrality at ticks 3/4/8 with the FILE
  ideology colours.
- Players: `base_snapshot` sets `world.scenario.players = [GER,SOV,ENG]` (`is_human`
  true only for GER/SOV/ENG). The gate opens `fx_owner_controller_flip`, confirms
  the session view yields {GER,SOV,ENG}, then exercises the 0.15 fade with a
  synthetic non-player owner (FRA, `is_human=false`).

## Recording dependencies
No NEW recording deps. All three modes key off already-listed views: owner
(item 2), ruling ideology (item 7b), player↔country session (item 4). No
`REPLAY_DATA_REQUIREMENTS.md` / `EXTRACTION_NEEDS.md` additions required.

## Docs updated (same change)
- `MAPMODE_COVERAGE_MATRIX.md` — rows 13/15/16 → **law-done** with citations;
  §4 count summary (`law-done = 3`, `law-known = 5`).
- `FIDELITY_LEDGER.md` — new §1 row "Admin map modes (M1b) 6/0/0/1" + detailed
  §2.20 subsection (6 FILE items + 1 UNKNOWN MP-palette).

## replay_core = READ-ONLY (no gaps found)
Every view this lane needs already exists and is public:
`WorldFrame::owner_by_state()`, `WorldFrame::country_identities()` →
`CountryIdentity::ruling_ideology`, `WorldFrame::session_metadata()` →
`SessionMetadataView::player_countries`. No edits made; no missing view.

## Cross-lane note (concurrent worktree collision — RESOLVED)
Mid-session a parallel process created three sibling files
(`mapmode_states.rs` / `mapmode_ideology.rs` / `mapmode_players.rs`) — an
independent, more-integrated implementation of these SAME three modes — and wired
them into `lib.rs`, colliding on the `ideology_colors_from_resolver` re-export.
That process then **retracted** the three files and their `lib.rs` declarations,
leaving `map_mode_admin` as the sole implementation; I restored my clean export
names and reconfirmed green (708). Worth flagging: the sibling implementation
independently derived the **identical** 4 ideology colours, the same two
`00_graphics.lua` defines, and the same "state outline = cross-state boundary"
law — strong independent cross-validation of this lane's citations. Its approach
reused `border_world_synthesis` `BorderMaterial` chains for outlines +
`synthesize_political_colors_with_keying` for fill; mine derives outlines from the
raw province raster and keeps the module fully decoupled from `replay_core`. If the
orchestrator later prefers the chain-based outline it is a drop-in swap behind the
same `map_mode_admin` API. (The now-deleted sibling `mapmode_states` shipped-map
vanilla gate was itself failing at its line 287 — another reason mine is the safer
sole implementation.)
