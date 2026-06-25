# Map Viewer (owner-colored)

Offline, localhost-only owner/controller map for the HOI4 MP-stats dashboard.
Single HTML page + vendored JS. No CDN, no network, stdlib-only server.

## Run

From the repo root (`battle-plan/`), or from anywhere:

```
python map_view/serve.py
```

Then open **http://127.0.0.1:8777/map_view/** in a browser.

`serve.py` serves the **repo root** so the page can fetch sibling assets with
root-relative paths (`/map_pipeline/...`, `/save_oracle/...`) — this avoids
`..` path-escaping and keeps **no absolute personal path** in tracked source.
The bind is hard-locked to `127.0.0.1` (loopback only).

Options: `--port N`, `--root PATH`.

> Opening `index.html` via `file://` will **not** work — browsers block
> `fetch()` of local files. Use the server.

## Data flow

```
province_ids_half.png ──decode once──▶ Uint32Array idRaster   (id = R + G*256 + B*65536)
province_to_state.json ─────────────▶ Int32Array stateByProvince[maxId+1]  (-1 = none, id 0 = water)
snapshot {state:{owner,controller}} ┐
country_colors.json {TAG:[r,g,b]}   ┴▶ packed per-province color tables  (primary / alt-stripe / contested)
                                       │
                                       ▼
                                    recolor(): single pixel loop  idRaster[i] → packedColor → display ImageData
```

- **Decode once.** The id-PNG is decoded to a typed array a single time. Recolor
  iterates provinces/pixels and writes a packed-`Uint32` color per pixel — it
  never re-reads the PNG. Mode/snapshot changes only rebuild the small
  per-province color tables (sized `maxId+1`), then run the loop once,
  coalesced to one pass per animation frame.
- **Owner view** (default): each land province painted its state owner's country
  color. Sea/lake/unassigned (province id 0 or no state) → neutral water color.
- **Control view**: painted by **controller**. Contested states
  (controller ≠ owner) get a diagonal hatch — controller color striped with a
  darkened owner color (`((x+y)&7)<4`), computed purely from pixel coords so it
  stays snapshot-driven.

### Resolution choice

Display **and** hit-testing use the **half-res** raster (`2816×1024`, 2.9M px) by
default — `province_ids_meta.json` reports `provinces_lost_at_this_scale: 0`, so
no province is dropped, and it keeps recolor/memory light (full-res 5632×2048 is
~11.5M px and ~150MB peak during decode). If half-res ever fails to decode, the
viewer falls back to full-res automatically (`chooseRaster()`).

## UI

- **Owner / Control** mode switcher (top bar).
- **Legend**: top owners (or controllers) by state count, color swatch + count.
- **Hover** → floating tooltip (state name, owner, controller, contested mark).
- **Click** → side panel with state name + id, province id, owner, controller,
  VP total, province count, category, manpower.
- **Validation** panel prints province/owner counts + spot-checks (also logged
  to the browser console).

All data-derived text (tags, names) is injected via `textContent` / DOM nodes —
never `innerHTML` — so it is HTML-safe.

## TODO: timeline scrubber

The recolor pipeline is **snapshot-driven**: `recolor()` / `buildColorTables()`
read `M.snapshot` (a `{state_id:{owner,controller}}` object). Today there is
exactly one snapshot (the current melted save), so the slider is disabled.

To wire a real timeline later (see `app.js`, `wireUI()` → `TODO: SCRUBBER`):

1. Load an array of dated snapshots
   `SNAPS = [{date, ownership:{state:{owner,controller}}}, …]`.
2. Set `slider.max = SNAPS.length-1`, enable it, label with `SNAPS[i].date`.
3. On `input`: `M.snapshot = SNAPS[i].ownership; applyViewChange();`

`applyViewChange()` repaints the map, refreshes the legend, and re-renders the
sticky info panel for the selected province — so owner/controller text never
goes stale across snapshots. Nothing else changes; the flow was built around a
single snapshot input.

## Files

- `index.html` — markup + styles.
- `app.js` — load, decode, recolor, UI, validation.
- `serve.py` — stdlib loopback static server.
- `README.md` — this file.

Inputs are read from the sibling `map_pipeline/assets/` and
`save_oracle/output/` directories (read-only; not modified).
