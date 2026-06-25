# replay_render — map / division time-lapse renderer

Offline renderer that turns a sequence of per-timestamp map states into a
**time-lapse** of how a HOI4 game's map evolved: state owner/controller over
time, derived **front lines**, and an optional **division-marker** layer. Two
outputs from the same frames:

1. a **PNG frame sequence** (for video / GIF assembly), and
2. a **self-contained interactive HTML scrubber** (timeline slider that redraws
   the map at each timestamp) that drops into the dashboard later.

**Pure offline Python — the game does NOT need to be running.** numpy + stdlib
only; Pillow and ffmpeg are *optional accelerators* (a stdlib PNG decoder and a
"frames-only, GIF/MP4 skipped" path keep it working without them).

This package **reuses** the existing map assets read-only and never recomputes
geometry: the province→pixel raster and state mapping come from
`map_pipeline/assets/`, and the recolor semantics mirror `map_view/app.js`.

## Quick start

```
# synthetic 3-frame demo (no real data needed)
python -m replay_render.demo

# also render the real save_oracle snapshot, with the division layer
python -m replay_render.demo --real --divisions

# portable HTML (raster inlined; open from anywhere)
python -m replay_render.demo --embed
```

Outputs land under `replay_render/output/` (**gitignored** — see below). Open
an HTML scrubber via the existing loopback server (it roots at the repo so the
`/map_pipeline/...` assets resolve):

```
python map_view/serve.py
# then: http://127.0.0.1:8777/replay_render/output/synthetic_scrubber.html
```

(`--embed` HTML needs no asset server — the raster + province map are inlined.)

## What it produces

- `output/<name>/frame_####.png` — one RGB PNG per timestamp (half-res
  2816×1024 by default; `--full-res` for 5632×2048).
- `output/<name>/<name>.gif` — animated GIF (if Pillow present; else skipped,
  frames still written).
- `output/<name>/<name>.mp4` — H.264 MP4 (only if `ffmpeg` is on PATH; else
  skipped). Enable with `make_mp4=True`.
- `output/<name>/manifest.json` — frame list + timestamps + sizes + gif/mp4
  paths (or skip reasons), for a viewer/dashboard to pick up.
- `output/<name>_scrubber.html` — the interactive time-lapse scrubber.

## How it consumes frames (the generic interface)

The renderer is **schema-agnostic**. Everything is built on one dataclass
(`frames.py`):

```python
Frame(
  timestamp: str,                       # "1936.1.1" / "1940.9.10.13"
  owner:      {state_id: TAG | (r,g,b)},
  controller: {state_id: TAG} | None,   # None == controller equals owner
  divisions:  [DivisionMarker(province_id, owner, strength, label)] | None,
)
```

A frame = `(timestamp, {state→owner}, optional {state→controller}, optional
[division markers])` — exactly the generic interface the task specifies.

Adapters (the **only** code that knows a concrete on-disk shape):

| adapter | input |
|---|---|
| `frame_from_ownership_dict(ts, ownership, divisions)` | the `save_oracle` `state_ownership.json` shape `{state:{owner,controller}}` + the `divisions.json` `divisions` list |
| `frame_from_replay_snapshot(snapshot)` | **one** canonical `replay/SCHEMA.md` snapshot (placeholder) |
| `frames_from_replay_series(series)` | a time-series (list or `{date:snapshot}`) of snapshots |

### Adapting to `replay/SCHEMA.md` (one-file change)

`replay/SCHEMA.md` is owned by another agent and not present yet. Until it
lands, `frame_from_replay_snapshot` is a tolerant placeholder (accepts
`date`/`timestamp`/`save_date`; `ownership`/`states`/`owner`; division
`province` or `location_province`). **When the real schema lands, edit ONLY the
two adapter functions in `frames.py`** to match its key names/structure — the
renderer, time-lapse builder, and scrubber all work purely in terms of `Frame`
and never change. If the schema carries **province-level** control (not just
state-level), extend the adapter to emit per-province control plus add a
province-keyed lookup in `render._color_tables`; that is the one anticipated
larger change and it is localized.

## Files

| file | role |
|---|---|
| `frames.py` | the generic `Frame` / `DivisionMarker` interface + schema adapters |
| `geometry.py` | read-only loader for `map_pipeline/assets/` (PNG decode w/ stdlib fallback, province→state, province & state pixel **centroids**) |
| `render.py` | `FrameRenderer`: one `Frame` → RGB ndarray (owner fill, contested hatch, front lines, division discs) + stdlib PNG writer |
| `timelapse.py` | `build_timelapse`: frames → PNG sequence (+ optional GIF/MP4) + manifest |
| `scrubber.py` | `build_scrubber_html`: frames → self-contained HTML scrubber (JS mirrors the renderer) |
| `demo.py` | synthetic 3-frame demo + optional real `save_oracle` snapshot; CLI |
| `output/` | generated PNG/GIF/MP4/HTML — **gitignored**, never commit |

## Rendering model

Mirrors `map_view/app.js` so PNG frames and the HTML scrubber look identical:

```
pixel → province id (from id-raster)
      → state id     (province_to_state)
      → owner / controller TAG (the Frame)
      → country color (country_colors)
```

- **Owner fill**: each land province painted its owner's color; water / no-state
  → neutral. Sized lookup tables (`maxId+1`) rebuilt per frame; one numpy pixel
  pass (decode the raster **once**, reuse across all frames).
- **Contested hatch**: where controller ≠ owner, a diagonal hatch `((x+y)&7)<4`
  blends toward the controller color (owner view) / a darkened owner (control
  view) — purely coordinate-derived, so it stays snapshot-driven.
- **Front lines** (derived `D`): a pixel whose controller differs from its
  right/down neighbor's controller is drawn dark — a cheap control-adjacency
  edge, recomputed per frame.
- **Division markers** (optional): aggregated per province, drawn as a disc at
  the province **pixel centroid**, radius ∝ √strength, colored by majority
  owner. Centroids are computed once from the raster.

## Performance

Geometry (incl. all 13,413 province centroids) loads in ~0.3 s. A single real
frame — 1081 states, contested hatch, front lines, 1654 division markers at
half-res 2816×1024 — renders in **~0.09 s**, so a full-game daily replay
(~365 frames) renders in well under a minute after the one-time geometry load.
The geometry/`FrameRenderer` are built once and reused across every frame.

> GIF assembly buffers rendered frames in memory; for very long sequences
> prefer `make_gif=False` and assemble externally from the PNG sequence (or use
> the MP4 path with ffmpeg, which streams from disk).

## Boundaries

Creates/edits files only under `replay_render/`. Reads `map_pipeline/assets/`
and `save_oracle/output/` read-only; does not import or modify `map_pipeline`,
`map_view`, `replay`, `save_oracle`, `memory_reader`, `dashboard`, or `mod`.
Generated media under `output/` must stay out of git.

### gitignore

Add to the repo `.gitignore` (large generated media must never be committed):

```
replay_render/output/
```
