# Map preprocessing pipeline

Offline pipeline that turns a HOI4 mod's raw map + history + country data into
browser-friendly assets for the multiplayer-stats dashboard. **Pure offline data
prep — no game, no network.** Run it once whenever the mod's map / states /
country colours change; the dashboard's canvas renderer then consumes the assets.

## Run

```
python build_map_assets.py [--mod <path-to-mod>] [--out <dir>] \
                           [--vanilla <hoi4-dir>] [--downscale N | --full-res]
```

- `--mod`   HOI4 mod directory. Default: `../../test` (the sibling mod), so no
            absolute personal path is hard-coded. Pass your own path to override.
- `--out`   Output root; `assets/` is created under it. Default: this script's dir.
- `--vanilla` Optional base-game dir; used only to find a country file that the
            mod doesn't ship. Skipped (with a note) if absent.
- `--downscale N` Integer nearest-neighbour downscale for the optional display
            raster (default `2` → 2816×1024). `--full-res` emits no display copy.

Dependencies: **Python stdlib + numpy only.** Pillow is *not* used — the PNG is
written via stdlib `zlib`+`struct`, and the BMP is parsed by hand, so the
pipeline runs even where Pillow is missing.

## Assets (under `assets/`)

| File | What it is |
|------|------------|
| `province_ids.png` | **Authoritative** full-res (5632×2048) lossless RGB PNG. Each pixel encodes the *province id* it belongs to (not a display colour). |
| `province_ids_half.png` | Optional half-res (2816×1024) display copy, same encoding. Verified to lose **0** provinces at the default ×2 scale. |
| `province_ids_meta.json` | Dimensions, the id-encoding scheme, JS decode snippet, downscale info + any lost-province ids. |
| `province_to_state.json` | `{province_id: state_id}`. A **missing** key means the province is unassigned (sea / lake / no state). |
| `state_meta.json` | `{state_id: {name_key, provinces, vp_total, victory_points, state_category, manpower}}`. |
| `country_colors.json` | `{TAG: [r,g,b]}` map colours, 0–255. |

### Province-id encoding

The id raster is *not* a picture of the map — each pixel's RGB **is** the id of
the province under that pixel:

```
province_id = R + G*256 + B*65536          (R = id & 0xFF, G = (id>>8)&0xFF, B = (id>>16)&0xFF)
```

Id `0` is the "no province" sentinel (sea/lake/unassigned colours and any pixel
whose colour isn't in `definition.csv` all encode as id 0).

This is the standard web map technique: a lossless image whose pixels carry data,
decoded per pixel. The original `provinces.bmp` colours are arbitrary per-province
RGB; we replace them with the *encoded id* so the renderer can map pixel→province
with one `getImageData` read and no colour-lookup table on the client.

## How the canvas renderer uses these

1. Load `province_ids.png` into an offscreen canvas (or the half-res copy for
   display/perf). **Disable smoothing** (`ctx.imageSmoothingEnabled = false`) and
   never scale it with interpolation — interpolation would blend ids into garbage.
2. For a pixel under the cursor (or for a full repaint, iterate the buffer):
   ```js
   const px = ctx.getImageData(x, y, 1, 1).data;   // [R,G,B,A]
   const provinceId = px[0] | (px[1] << 8) | (px[2] << 16);
   ```
3. `state_id = province_to_state[provinceId]` (absent ⇒ unassigned: sea/lake).
4. `owner = <state owner TAG>` — the **owner is dynamic per-game** and comes from
   the dashboard's live data source (memory reader / save), *not* from this
   pipeline. (`state_meta` gives static facts: name, VP, category, manpower,
   province list — it deliberately does **not** bake in an owner, because owners
   change during a game.)
5. `color = country_colors[owner]` → fill that province's pixels with the owner's
   map colour. For a political map you typically build, once, an
   `id → state → owner → [r,g,b]` table and recolour the whole id buffer in one
   pass.

So the static chain this pipeline provides is **pixel → province → state**; the
dashboard supplies the live **state → owner**, and `country_colors` closes it with
**owner → colour**.

## Source data & parsing notes

- `map/definition.csv` — `province_id;R;G;B;type;coastal;terrain;continent`, 13414
  rows (id 0 is the sentinel). Province colour = `(R,G,B)`. Read as latin1.
- `map/provinces.bmp` — 24bpp uncompressed BMP, bottom-up BGR. Parsed by hand:
  the pixel-data offset and row stride are read from the header (not assumed),
  rows are flipped to top-down, BGR→RGB. A 2²⁴-entry int32 lookup table maps each
  pixel's colour → province id in one vectorised numpy pass.
- `history/states/*.txt` (latin1) — `id`, `name`, `provinces={}`, `victory_points={}`,
  `state_category`, `manpower`. `#` comments are stripped first. The `provinces`
  block is a flat id list. `victory_points` blocks are flat `prov vp` pairs;
  any brace block containing `=` (e.g. a per-province building block like
  `3838 = { naval_base = 3 }`) is rejected so it can't be mistaken for VP data.
  This mod has **no** VPs nested inside dated `YYYY.M.D = {}` blocks, so every VP
  collected is base-setup; if a future mod adds dated VPs they would also be
  collected (acceptable for a base-setup stats view).
- Country colours — `common/country_tags/*.txt` give `TAG = "countries/File.txt"`.
  The per-country file's `color = { r g b }` / `color = rgb {…}` is the primary
  map colour; `common/countries/colors.txt` (`TAG = { color = … }`) overrides it
  where present. HSV entries are converted with `H∈[0,1]`, `S`/`V` clamped to
  `[0,1]`. A few `colors.txt` entries are **mislabelled RGB** written as
  `HSV { 221 178 83 }` (H > 1); those are detected by `H > 1` and read as 0–255 RGB.

## Validation (printed by the script, also asserted)

- Spot-checks: state 351 (Vologda, 70 provinces), 400 (Kirov, 65), 495 (Amazonas,
  44), 1 (Corsica, provinces `3838 9851 11804`) — province lists match the files,
  exist in `definition.csv`, and resolve to a colour present in the BMP.
- Counts: 13414 province rows (incl. id 0), 1081 states.
- Coverage: every real **land** province is assigned to a state (only id 0 is
  "unassigned land"). ~3141 provinces unassigned overall = sea + lake zones.
- Reports duplicate definition colours, pixel colours absent from `definition.csv`,
  province→state collisions, and provinces lost at the chosen downscale (0 at ×2).
