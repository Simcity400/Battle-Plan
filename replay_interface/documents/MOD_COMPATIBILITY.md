# Mod compatibility — the seamless-mods contract

Standing requirement (user, 2026-07-04): replays of modded games (map
changes, terrain, UI, colors — e.g. Kaiserreich) must look right with no
manual steps. This doc is the contract every replay-layer change follows.

## The one rule

**Every read of a game file goes through the mod-aware resolver
(`replay_interface/native/map_viewer/src/mod_resolver.rs` `FileResolver`), configured from the
RECORDING's mod identity — never a bare vanilla path.**

The resolver already implements Paradox override semantics: ordered mod
content layers over vanilla, file-level override wins, `replace_path`
directory exclusions, `dlc_load.json` playset input, override stats and
warnings. The recording carries the modset (`REPLAY_SCHEMA.md` §2.1
`provenance.mod`, written capture-side); generated `active_modset.json` files
are local inspection artifacts, not runtime authority. Resolution is therefore
fully determined by the recording or by explicit resolver inputs — seamless by
construction.

## Why files-first is also mods-first

- FILE-DERIVED features (political colors, seasons, placements, terrain
  sources, UI chrome, fonts, flags) adapt to any mod AUTOMATICALLY when
  read through the resolver — the mod's files simply win. This is a core
  reason capture-reliance must shrink: capture evidence is bound to the
  modset it was captured under.
- CAPTURE-DERIVED evidence divides into:
  - **Mod-invariant** (until a mod touches the source): packing/format
    laws (TerrainIDMap layout, CB layouts, pass structure) — reusable
    across mods, re-validate only if the mod ships its own `gfx/FX`.
  - **Mod-variant content** (chunk templates, composed GB textures,
    ColorCube, shader bytecode when a mod edits shaders): valid only for
    the captured modset. The session manifest must state the modset it
    was captured under; replay fails closed on a modset mismatch rather
    than silently mixing evidence (checksum_manifest.txt dirs = what mods
    can change; anything a mod CAN change needs this guard).

## Current state (audited 2026-07-04)

- THROUGH the resolver: bundle_gen (all map/terrain/gfx bundle inputs),
  game_data.rs parsers used by the bundle, game_assets extraction.
- BYPASSING the resolver (to fix — job M1): `seasons.rs`
  `seasons_table_from_game_dir` (raw `map/seasons.txt` join),
  `map_static_data.rs` `*_from_game_dir` (raw joins for buildings/
  unitstacks/railways/supply/weather/common-buildings),
  `replay_interface/native/replay_app/src/font.rs` (raw game-root font-page joins; F1 must land
  resolver-aware or be fixed in M1). `FileResolver` is `pub(crate)` —
  M1 exposes a public resolution surface for other crates.
- Constants baked from vanilla shader text (constants.fxh values wired
  into code): acceptable ONLY with a guard — if the active modset
  overrides the source shader file, the replay must warn/fail closed,
  not render vanilla laws silently. M1 adds the guard hook; the session
  modset manifest carries the shader-override flag.

## Rules for new work (binding on every future job spec)

1. New file readers take a resolver (or a type that carries one), not a
   bare game dir. `_from_game_dir` helpers may exist for tests only.
2. Every parser gets a mod-override test: a synthetic mod layer
   overriding the file, asserting the override wins and `replace_path`
   is honored.
3. Capture-session manifests record the active modset; consumers verify
   it against the recording's `provenance.mod` and fail closed on
   mismatch (with a precise error naming both modsets).
4. Job specs for anything touching installed content MUST cite this doc and the
   one rule. The orchestrator rejects diffs that add bare vanilla joins.

## Archive mods: the collision matrix that has never been captured

Mods can ship content inside archives rather than as loose files, and nothing
in this repo has ever exercised that path — **the installed playset has zero
archive mods**, so no row below can be captured without a human changing
launcher state. Treat archive resolution as unproven, not as working.

Six launches settle it. Use one virtual path with archive bytes and loose bytes
that differ, so the winner is unambiguous:

| Row | Layers, low to high | Required result |
|---|---|---|
| A | archive only | normalized member, archive bytes returned |
| B | loose only | loose bytes returned |
| C | archive, loose | winner bytes |
| D | loose, archive | order-reversal winner bytes |
| E | archive, loose with exact-directory `replace_path` | replacement result |
| F | missing archive descriptor | exact warning/skip, fatal, or silent-empty |

Add one mixed-case and one backslash-named archive member to exercise key
normalization. Retain descriptor bytes and hash, resolved archive path,
central-directory ordinal, normalized key, layer ordinal, collision winner,
returned bytes and hash, and the exact warning or failure text.

Do not let this block a launch that has no archive mods: per-launch admission
needs authenticated layer/order and collision-winner rows joined to the exact
member and consumed-file bytes for *that* capture, not a general law. Missing or
contradictory joins still reject the playset.
