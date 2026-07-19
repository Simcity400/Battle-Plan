# LANE REPORT — lane/m1c-diplomacy-factions (map modes Diplomacy + Factions)

Plan: `REPLAY_PRODUCTION_PLAN.md` Phase E3 / wave **M1c**; matrix rows 14
(Diplomacy) + 11 (Factions) of `MAPMODE_COVERAGE_MATRIX.md`. New modules
`map_viewer/src/mapmode_diplomacy.rs` + `map_viewer/src/mapmode_factions.rs`
(registered in `lib.rs`; `pub use` re-exports). `replay_core`/`extractor`
untouched (read-only); `map_furniture.rs` read-only (relation law reused via the
`pub` seam, not modified).

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p map_viewer` | **Finished, 0 errors** (no new warnings from these modules) |
| `cargo test -p map_viewer --lib` | **714 passed / 0 failed / 1 ignored** (+16 new over the 698 baseline) |
| Diplomacy tests (`mapmode_diplomacy`) | **8 passed** |
| Factions tests (`mapmode_factions`) | **8 passed** |
| `fx_war_faction_matrix` diplomacy gate | fills flip @ **tick 3** (faction) + **tick 7** (war), verified under **2 POVs** (GER, ENG) |
| `fx_war_faction_matrix` factions gate | membership + fill flip @ **tick 3** (faction formation) |
| Vanilla FILE gate (`gate_vanilla_factions_defines`, BATTLE_PLAN_GAME_DIR set) | **PASS** — `FACTIONS_COLOR_NOT_MEMBER = {0.7,0.7,0.7,0.7}` parsed from the shipped `00_graphics.lua` |
| FURN-REL dedup guard | `classification_agrees_with_furn_rel_icon_law` — `classify_relation` proven column-for-column equal to `map_furniture::vp_icon_relation_column` across the full input space |

## Diplomacy (row 14) — what is solved vs UNKNOWN

- **Relation CLASSIFICATION = solved (FILE/RE).** Extends the engine-traced
  POV. Priority: missing owner/viewed → neutral; self → self; same-faction →
  friendly; at-war → enemy; else neutral. Input `DiploPairInput` mirrors the two
  `DiplomaticPairState` booleans the traced law reads (fail-closed).
- **Fill RGB = UNKNOWN / fail-closed.** No file source exists. I decoded the
  shipped `onmap_victorypoints_strip.dds` (the icon strip that shares this
  classification): all five "colour columns" are **byte-identical grayscale**
  (star frames 0..4 have identical opaque histograms) — the strip carries shape,
  the engine applies the relation colour as a runtime *modulate*. `00_graphics.lua`
  has no `MAP_MODE_DIPLOMACY_*` define (`FRIEND/ENEMY/NEUTRAL_COLOR` are for unit
  counters, a different consumer). `DiplomacyPalette::unknown_fail_closed()`
  returns `None` for every class; `relation_fill` paints nothing rather than
  invent. Residual = a captured diplomacy frame OR binary RE of the layer's
  hardcoded modulate constants.

## Factions (row 11) — colour law

- **Non-member = FILE:** `FACTIONS_COLOR_NOT_MEMBER {0.7,0.7,0.7,0.7}` +
  `FACTIONS_MEMBER_TRANSPARENCY 1.0` (`00_graphics.lua`, `NDefines.NMapMode`),
  resolver-parsed (mod-override) and gated against the real shipped file.
- **Member = faction-LEADER country colour (DERIVED):** a faction is keyed by
  its leader TAG, so members read the leader's colour (bloc view; matrix rule
  "colour = faction/leader"). Uses the same raw tag→RGB table as the political
  and naval fills. Fail-closed to `None` when the leader tag or its colour is
  missing. Two colour-only residuals flagged (capture): leader-colour-vs-own,
  and raw-colour-vs-COL3-HSV-modified. Membership classification is independent
  of both.

## Decoupling + dedup notes

- Both modules follow the `naval_supremacy_tint.rs` pattern: no `replay_core`
  dependency; input structs mirror the `DiplomacyView` shape; live wiring belongs
  at the `replay_app` seam.
- The relation predicate lives once in `map_furniture` (`vp_icon_relation_column`);
  this lane reuses it via the crate `pub` seam and a cross-check test rather than
  copying it. **Follow-up dedup note:** the FURN-REL comment in `map_furniture.rs`
  (lines ~346-360) states the VP strip's 5 columns are "a diplomatic color
  palette (red, blue, green, yellow/gold, gray)". Decoding the shipped DDS shows
  the columns are **identical grayscale** — the colour is an engine modulate, not
  in the texture. That comment should be corrected in a `map_furniture`-owning
  lane (not touched here per wave-5 conflict rules).

## Docs updated (own rows only)

- `MAPMODE_COVERAGE_MATRIX.md` rows 11 + 14 → **impl**.
- `FIDELITY_LEDGER.md` §2 — new §2.20 (Diplomacy) + §2.21 (Factions); §1 totals
  untouched.
- `REPLAY_PRODUCTION_PLAN.md` §6 M1 wave line → M1c IMPL note.
