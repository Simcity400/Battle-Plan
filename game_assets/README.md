# game_assets — HOI4 UI asset-extraction pipeline

Foundation layer for the **game-authentic replay UI**. It reads a (read-only)
Hearts of Iron IV install plus the mod overlay and produces a sprite manifest,
parsed window layouts, and converted PNGs so the UI agent can recreate the real
HOI4 interface from replay data.

## What it produces

| Output | Committed? | Contents |
|---|---|---|
| `extract.py`, `paradox_parser.py` | yes | the pipeline + Clausewitz-script parser |
| `sprites.json` | yes | sprite-name -> texture/frame **metadata** (paths, dims, frame counts) — no art bytes |
| `gui/*.json` + `gui/_index.json` | yes | parsed window layouts (geometry / sprite refs / text) |
| `README.md`, `.gitignore` | yes | this file + ignore rules |
| `assets/**` | **NO (gitignored)** | the converted PNGs (Paradox art) |
| `extract_report.json` | **NO (gitignored)** | diagnostics (failures/anomalies; may hold relative texture paths) |

> **Copyright / privacy.** These are the owner's own Paradox installed content,
> extracted for a private offline tool. The converted art under `assets/` is
> **gitignored and never redistributed**. Only the script, the metadata manifest,
> and the parsed layouts are committed. No personal home path is baked into any
> committed file — install paths come from CLI args / a documented default.

## Regenerate from a local install

```bash
cd game_assets
# defaults: --vanilla = the documented Steam path, --mod = ../../test
python extract.py
# point at your own install / mod:
python extract.py --vanilla "D:/Steam/steamapps/common/Hearts of Iron IV" \
                  --mod "/path/to/mod/test"
```

Requires Python 3.11+ and Pillow (`pip install pillow`). A full run converts
~33k PNGs and takes ~8 min; the manifest/layouts alone are instant.

### Useful flags

| Flag | Effect |
|---|---|
| `--manifest-only` | parse `.gfx`/`.gui` only; skip texture conversion (fast) |
| `--only sprites` / `--only flags` | convert just one pass |
| `--merge` | with a partial `--only` pass, keep the other section of an existing `sprites.json` |
| `--categories goals,ideas` | convert bytes only for those sprite categories (others get metadata only) |
| `--limit N` | convert at most N items per pass (smoke test) |

## How the UI agent references a sprite

`sprites.json` shape:

```jsonc
{
  "sprites": {
    "GFX_fuel_strip": {
      "type": "spriteType",
      "no_of_frames": 3,
      "category": "icons",
      "texturefile": "gfx/interface/unitcontrol/fuel_strip.dds",
      "png": ["assets/icons/GFX_fuel_strip_f0.png",
              "assets/icons/GFX_fuel_strip_f1.png",
              "assets/icons/GFX_fuel_strip_f2.png"],
      "width": 72, "height": 24
    },
    "GFX_tiled_window_small_small": {        // 9-slice window frame
      "type": "corneredTileSpriteType",
      "nine_slice": true,
      "size": {"x": 48, "y": 48},
      "border_size": {"x": 16, "y": 16},
      "png": ["assets/window_chrome/GFX_tiled_window_small_small.png"]
    },
    "GFX_shield_medium": {                    // player-flag shield (composite)
      "type": "maskedShieldType", "masked_shield": true,
      "png":  ["assets/icons/GFX_shield_medium.png"],     // textureFile1 (overlay)
      "png2": "assets/icons/GFX_shield_medium__2.png"     // textureFile2 (mask)
    },
    "GFX_topbar_fuel_bar": {                  // progress bar (fuel/stab/WS/etc.)
      "type": "progressbartype", "progress_bar": true,
      "color": [0.0, 0.5, 0.0], "color_two": [1.0, 0.0, 0.0],  // RGB 0..1 gradient
      "size": {"x": 45, "y": 8},                          // authoritative bar size
      "png":  ["assets/icons/GFX_topbar_fuel_bar.png"],    // textureFile1 (fill)
      "png2": "assets/icons/GFX_topbar_fuel_bar__2.png"    // textureFile2 (background)
    }
  },
  "flags": {
    "GER_fascism": {                          // key = TAG[_ideology]
      "tag": "GER", "ideology": "fascism",
      "png": "assets/flags/GER_fascism.png",  // largest available
      "width": 82, "height": 52,
      "res": {                                // up to 3 resolutions
        "large":  {"png": "assets/flags/GER_fascism.png",         "width": 82, "height": 52},
        "medium": {"png": "assets/flags/GER_fascism__medium.png", "width": 41, "height": 26},
        "small":  {"png": "assets/flags/GER_fascism__small.png",  "width": 10, "height": 7}
      }
    }
  },
  "_meta": { ...counts... }
}
```

Rules for the UI agent:

1. **Look up by sprite name** (the `GFX_*` string a `.gui` element references via
   `spriteType=` / `quadTextureSprite=`). Use `png[0]` for single-frame, or the
   indexed frame for multi-frame strips (`_f0`, `_f1`, ...). All PNG paths are
   relative to `game_assets/`.
2. **Flags** are keyed `TAG` or `TAG_ideology` (ideology one of
   `fascism/communism/democratic/neutrality`; `null` for cosmetic-tag flags whose
   suffix is not an ideology). Pick a resolution from `res`, or use the top-level
   `png` for the largest.
3. **9-slice frames** (`nine_slice: true`) are window chrome: render with
   `border_size` corners fixed and the center stretched/tiled — do not scale the
   whole PNG uniformly.
3b. **Two-layer sprites** carry a `png2` second layer:
   - `masked_shield: true` (player-flag shields) — `png` is the flag/shield
     overlay, `png2` is the mask; composite mask over flag at render time.
   - `progress_bar: true` (fuel / stability / war-support / resource bars) —
     `png` is the fill, `png2` the background; tint the fill between `color`
     and `color_two` (RGB 0..1) by fill fraction, and treat `size` as the bar's
     authoritative pixel dimensions (the PNG may differ).
4. **Window layouts** live in `gui/<view>.json` as a recursive tree of elements
   (`position`, `size`, `orientation`, `sprite`, `text`, `children` — child order
   = draw order). Percentages like `100%%` are preserved verbatim as strings;
   resolve them against the parent at render time.

## Layouts parsed (`gui/`)

`topbar`, `research`, `production`, `national_focus`, `diplomacy`, `decisions`,
`province_state`, `construction`, `land_combat`, `naval_combat`, `combat_log`,
`menubar`. See `gui/_index.json` for element-type counts per view.

## Known limitations / what is NOT captured

- **Truly-missing textures** (~441): referenced by a `.gfx` but not shipped on
  disk (DLC-gated ace portraits, some tech icons). Logged in `extract_report.json`,
  not invented.
- **Odd-width frame strips** (~86): width not divisible by `noOfFrames` (mostly
  battle-plan tool buttons). Saved as the **whole strip** + flagged, never floored
  into corrupted frames. The UI can slice them manually if needed.
- **`pdx_online_assets/`** textures (6): online-account UI living outside `gfx/`;
  irrelevant to the replay, intentionally not converted.
- **DX10 BC-compressed** / **RXGB** (1 file): only *uncompressed* DX10 (DXGI
  28/29 RGBA8, 87/91 BGRA8) is decoded via the manual fallback; the lone RXGB file
  (`swe_rearnament`) is skipped. Pillow handles all DXT1/3/5 + uncompressed DDS +
  TGA directly.
- **Not extracted** (out of scope for layout reproduction): localisation `.yml`
  text, font glyph assets, runtime visibility conditions, listbox/scrollbar
  contents, scripted-GUI behavior. The pipeline reproduces the **static layout
  skeleton + sprites**, not live UI logic.
