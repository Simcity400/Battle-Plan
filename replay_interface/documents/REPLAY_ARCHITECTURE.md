# Replay Layer Architecture (decision of record)

Owner: replay-layer agent. This document is the active architecture for
everything under `replay_interface/`. Where it conflicts with older documents,
this document wins; the older documents remain valid for the specific standards
they define (grounding rules, fidelity evidence, container integrity).

## Product goal

Open a `.battleplan` recording and play the game back so it looks and feels
like the game itself — the real map, the real UI chrome, real fonts,
real icons — except strictly read-only, with time scrubbing. Not a dashboard.

## The one diagram

```text
.battleplan (from extraction layer)
      |
      v
replay_core  (Rust)  — container open, integrity, ingest -> WorldTimeline
      |                 state_at(hour) scrub API, absent/null/measured preserved
      v
replay_app   (Rust)  — D3D11 swapchain window (the product surface)
      |                 map render-to-texture + UI composited over it
      +-- map renderer    : province-id texture + GPU recolor (map modes),
      |                     terrain underneath (captured-pass replay when ready)
      +-- ui renderer     : .gui layouts + .gfx sprites + bitmap fonts + loc,
      |                     extracted from the effective game/mod files
      +-- overlays        : unit counters, fronts, combat bubbles, arrows
      v
   the user, scrubbing a war
```

Supporting toolchains (not in the runtime path):

- `map_viewer` — map bundle generation from the game install, CPU reference
  renderer, captured D3D11 pass validation/replay. Feeds `replay_app` with
  grounded static inputs; keeps its fail-closed capture program.
- `terrain_capture` — one-time capture of engine-generated GPU inputs
  (TerrainIDMap, water/river/border passes). Captures are asset extraction,
  not a per-frame runtime dependency.
- Python (`replay/`, `models/`, `derive/`, `game_assets/`) — development,
  validation, fixture generation, and asset extraction tooling. Never in the
  runtime playback path. `game_assets/extract.py` output IS a runtime input
  (generated once, loaded by `replay_app`).

## Ingest boundary (the extraction-layer seam)

The extraction layer's final shape is not settled. The replay layer therefore
binds to a narrow boundary and keeps everything behind it swappable:

- `replay_core::source::ReplaySource` is the only place recording bytes enter
  the replay layer. Everything downstream consumes `WorldTimeline`/`WorldFrame`.
- Backend 1 (works today): **snapshot-flavor container/dir** — hourly world
  snapshots per `REPLAY_SCHEMA.md`, read through the container rules of
  `HOI4REPLAY_CONTAINER_SPEC.md`. This is the proposed production contract
  (see `EXTRACTION_INTERFACE_REQUESTS.md`).
- Backend 2 (fail-closed stub): **compact-stream-flavor container** — the
  current production recorder packs only the raw compact domain stream
  (`role: collector_compact_stream`) with an empty snapshot index. Until the
  extraction layer either emits snapshots into the container or exposes its
  decoders as a reusable crate, this backend refuses playback with a precise
  error instead of guessing record layouts. No duplicated decoder logic.

Rules at the boundary:

- Absent (domain not captured) vs explicit null (captured-but-unobtained) vs
  measured value (including measured 0) survive into `WorldFrame` as a typed
  trichotomy. No coercion, no imputation, ever.
- Unknown domains/fields are preserved raw and reachable, never dropped, so
  the model tolerates schema growth without code changes.
- Typed accessors exist only where a UI surface consumes them; they are added
  per surface, not speculatively.

## Crate layout (target)

`replay_interface/native/` workspace:

| Crate | Role |
| --- | --- |
| `replay_core` | Container/dir ingest, `WorldTimeline`, scrub, typed views, provenance/coverage. Depends on `extractor/native/snapshot` (path dep) for the verified container reader — reuse, not a fork. |
| `replay_app` | The product: Win32 + D3D11 swapchain window, frame loop, camera, map-mode GPU recolor, hit testing, timeline control, and (growing) the game UI chrome. |
| `map_viewer` | Bundle tooling + CPU reference + captured-pass validation/replay backend. To be split from one 28.9k-line `lib.rs` into modules; behavior unchanged. |
| `terrain_capture` | Unchanged; capture DLL + attach helper. |

House style: raw `winapi` D3D11/Win32 (already the codebase norm), no new
GUI frameworks. `minifb` remains only inside `map_viewer` debug/reference
viewers and is not the product surface.

## Map rendering strategy

1. Base identity layer (now): `province_ids.r32` from the map bundle uploaded
   as an `R32_UINT` texture; political/control/other map modes are GPU lookups
   (province -> state -> owner -> country color) through small buffers updated
   on scrub. CPU mirror of the same buffer owns picking. This is the Phase-2
   architecture of `MAP_REPLICA_PLAN.md`, driven by replay data.
2. Terrain look (parallel track): the captured-pass D3D11 replay program
   continues (terrain proven; water/rivers/borders/trees pending real
   captures; MSAA blocker documented). It renders beneath the identity layer
   inside `replay_app`'s pass graph. Until it lands, the base layer renders
   over the grounded `map_visual.rgbx` scaffold and the gap is badged, per the
   grounding rules.
3. Nothing invented: colors, icons, fonts, layouts come from installed content or the
   recording. Gaps are badged unavailable, never faked.

## Sequencing decision (supersedes the old gate)

The old rule — no UI work until the map is 1:1 — is retired. It killed mock-UI
drift when that was the risk; the risk now is a photoreal ocean with no replay.
New sequencing:

1. **Spine**: `replay_core` ingest + scrub against snapshot-flavor recordings.
2. **Minimum believable replay**: political map recolor over the grounded
   scaffold, date readout, timeline scrub, speed/pause playback, picking.
3. **Parallel parity tracks** (each independently gated, none blocking 2):
   terrain/water/rivers/borders captured-pass replay; `.gui` UI chrome
   (topbar first) + bitmap fonts + localisation; unit counters/fronts/combat
   overlays; map modes/borders/labels.
4. **Visual QA harness**: side-by-side screenshots vs the real game, per-layer
   diff ledger (unchanged from `MAP_REPLICA_PLAN.md` Phase 6).

`MAP_REPLICA_PLAN.md` grounding rules and stop points remain binding for every
visual claim. `UI_SOURCE_OF_TRUTH_REPORT.md` remains binding for UI surfaces.

## What was reclassified

- Python `replay/` (schema/store/diff) — semantic reference + dev tooling +
  fixture generator. Its JSON snapshot shape is the ingest Backend 1 format.
- Python `models/`, `derive/` — formula/derivation reference; ported into
  `replay_core` per-surface when a UI panel needs a computed value.
- `map_pipeline/assets/*.json` — legacy compatibility data for extractor-side
  consumers; not a replay runtime input; removal belongs to the extraction
  owner.
- `game_assets/` — active runtime-asset pipeline; its generated bundle
  (`gui/*.json`, `sprites.json`, fonts, loc) must be regenerated per machine
  and is gitignored.

## Extraction-layer asks

Tracked in `EXTRACTION_INTERFACE_REQUESTS.md` (suggestions only — the
extraction agent owns that side).
