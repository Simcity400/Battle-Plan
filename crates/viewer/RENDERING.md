# Map rendering: making the replay map match the game

Owner instruction 2026-09-13: the viewer's map must look like the game's map at the same hour and camera (terrain, water, rivers, borders, names, icons, lighting, day and night, weather effects) closely enough that the two are indistinguishable. The game's own interface is out of scope; on-map unit counters (divisions and their movement) are in scope. This document is the shared design for that work. Status and evidence stay in `PLAN.md`; this file holds the design, the layer plan, the asset and data contracts and the evidence pointers.

## 1. Approach

The game composites its map with readable shader source under `gfx/FX/` (`pdxmap.shader`, `pdxwater.shader`, `river.shader`, `border.shader`, `mapname.shader`, `strait.shader`, `fow.fxh`, `standardfuncsgfx.fxh`, `constants.fxh`) driven by assets under `map/`, `map/terrain/`, `gfx/`, `interface/` and constants in `common/defines/00_graphics.lua`. We port that pipeline to wgpu/WGSL one pass at a time, sample the same textures with the same math, and reproduce the intermediates the engine generates at runtime (terrain id map, gradient border fields, border meshes, name curves, unit stacks) from the same inputs. Where the engine's own computation is not visible in assets (camera model, sun position, name placement, border field thickness), the decompiled a729 build in `<local-path>` is the evidence, and a fit against a game screenshot at a recorded hour is the check.

Rules that carry over from `PLAN.md`: data is exact and comes from the replay; visuals come from the game's own files with our compositing; nothing is interpolated between hours; unavailable tables are shown as such; the headless `--dump` path renders through the same pipeline as the window; no game data or assets are stored in the replay or the repository.

## 2. Reference pairs and scoring

A reference pair is a game screenshot at a paused, saved hour with a known camera, and the viewer's headless render of the same hour on the same camera. Pairs live in `reference/NN.md` with both images; captures and saves stay under `captures\`.

Existing material: `reference/01.md` (1936, format-1 replay, no longer readable). `captures\m21_takeover_20260913\array_loaded.png` is the game at 19:00 23 Aug 1943 (hour 60826314, the loaded save `GER_1943_08_23_19.hoi4`) at the load camera; the `array` replay's branch 2 starts at 60826315. `checkpoint2_save1_menu.png` shows 14:00 26 May 1944 (60832933, recorded on `checkpoint2` branch 1) behind the save dialog. Those are comparable framings until fresh pairs are captured with the game idle (the game lock `C:\Users\Example\AppData\Local\Temp\claude\hoi4_game.lock` belongs to the 2.1 performance track while it exists).

Scoring: the viewer gains a `--compare <game.png> <viewer.png>` mode that prints a per-region and overall similarity (mean absolute error in gamma space, structural similarity on a luminance pyramid) and writes a difference image. Masks exclude the game's UI bars, tooltips and dialogs. Targets are set per layer as the layers land; the acceptance in `PLAN.md` 2.4 is five pairs.

## 3. Pipeline

Coordinate systems: map pixel space is `provinces.bmp` (5632 x 2048, origin top-left, x right, y down). Game world space is x = pixel x, z = 2048 - pixel y (bottom-up, to be verified by research R1), y = height with `WATER_HEIGHT = 9.5` and heightmap value x 0.01 (`pdxmap.shader` vertex shader). `map/unitstacks.txt` and `map/buildings.txt` positions are world x, y, z. Shader `uv` is pixel/size; `uv2` flips y and scales by the power-of-two texture ratio (`MAP_POW2_X/Y`).

Camera: the game's perspective camera (height 50 to 3000, pitch as a function of height, field of view; research R1 derives the model from the decompile and fits it to a screenshot). `Camera` keeps `center`, `height` (or vertical span) and the derived pitch; `--camera x,y,span` stays accepted for headless runs and a game-style `--camera-height` is added.

Frame constants (bind group 0), mirroring the game's `ConstantBuffer(0,0)`: view-projection and inverse, camera position and look direction, gradient-border camera-distance overrides and outline cutoffs, sun and moon virtual positions (two each), `DayNight_Hour_SunDir`, `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed`, ambient cube, `HdrRange`, sun and moon diffuse intensity, `GB_TextureHeight`, global time. Values come from `00_graphics.lua` and research R1.

Pass order (each pass a module under `crates/viewer/src/map/`, registered in one list):

1. `terrain`: heightmap mesh (LOD quads as in the game or a single tessellated grid), `PixelShaderTerrain` port: terrain id map with the four-neighbour blend and the +100 "all same" trick, atlas diffuse and normal (`atlas0..2.dds`, `atlas_normal0..2.dds` selected by `texture_quality`), colormap overlay, snow and mud from the weather tables, gradient borders, secondary (occupation) mask, sun/moon Blinn-Phong, ambient cube, city lights at night, distance fog, day/night desaturation. Underwater terrain (`PixelShaderUnderwater`) below water.
2. `water`: `pdxwater.shader` port: flat plane at `WATER_HEIGHT`, LEAN normal maps (`lean1/2.dds`), specular map (`fow_rgb_waterspec_a.dds` alpha), refraction of the underwater pass, cube reflection (`reflection.dds`), ice, gradient borders for sea provinces, shore alpha from the heightmap, dominance effects only when recorded data exists.
3. `rivers` and `straits`: river meshes from `rivers.bmp` (source, flow, width classes) with `river.shader`; strait quads from `adjacencies.csv` with `strait.dds`.
4. `borders`: meshes along province boundaries classed country / state / province / sea / sea region / impassable, textured with `border_*_N.dds`, `BORDER_WIDTH`, camera-distance fades from the defines, `border.shader` port.
5. `names`: country names on curves (engine placement reproduced from research R2; `hoi_mapfont4`), victory-point and capital labels (`victory_point_mapicon`, `capital_mapicon` in `interface/mapicons.gui`, `GFX_onmap_victorypoints_strip` frame by value, font `hoi_20bs`), province names within `PROVINCE_NAME_DRAW_DISTANCE`.
6. `icons`: screen-space sprites anchored to world positions: unit counters at unit-stack positions (frame, flag stripe, count, organisation and strength bars, `unit_counter` layout at `interface/mapicons.gui`), naval counters for task forces, air base and naval base icons with counts, land combat icons, front lines and order arrows from `ArmyOrders.path`.
7. `post`: the game's post pass: color-correction LUT (`gfx/world/colorcorrection*.tga`, `gfx/posteffect_volumes.txt`), bloom on the city-light alpha, HDR range. Research R1 establishes which LUT and parameters apply.
8. `overlay`: egui only: timeline, warnings, table status; nothing of the map is drawn by egui.

Fog of war: the recorded fog inputs exist (D102) but v1 is omniscient (`PLAN.md` section 2); the FOW pass is built with the shader but disabled until per-country perspective is scheduled.

## 4. Asset contracts (crate `installed-content`)

All readers take `&ResolvedLayers` and return owned data; parsers take bytes and are unit-tested on synthetic input. Coverage: `map`, `common`, `history`, `localisation`, `interface`, `gfx/flags`, `gfx/interface` are hash-verified. Fonts (`gfx/fonts`), LUTs (`gfx/world`) and map items (`gfx/mapitems`) are outside the coverage policy; `ResolvedLayers::read_uncovered` reads them from the resolved layers in load order without hash verification and the viewer states that limitation once. Changing the coverage policy changes every layer hash and belongs to the recorder track, not here.

- `image::dds::decode(bytes) -> Texture { format: Bc1 | Bc2 | Bc3 | Rgba8, width, height, faces: 1 | 6, mips: Vec<Vec<MipLevel>> }`; block-compressed data is kept compressed for direct GPU upload; uncompressed 16/24/32-bit and 8-bit formats convert to RGBA8; cube maps keep six faces.
- `image::bmp::decode_indexed(bytes) -> IndexedImage { width, height, indices, palette: [[u8; 4]; 256] }` for `terrain.bmp`, `rivers.bmp`, `trees.bmp`, `cities.bmp`, `heightmap.bmp` (8-bit); `image::bmp::decode_rgb(bytes) -> Rgba8Image` for `world_normal.bmp`; rows always top-left first.
- `image::tga::decode(bytes) -> Rgba8Image` (uncompressed and RLE, 8/24/32-bit) for flags, fonts and LUTs.
- `fonts::parse_fnt(text) -> BitmapFont { line_height, base, scale, pages, glyphs: id -> Glyph { x, y, width, height, xoffset, yoffset, xadvance, page, channel }, kernings }` and `fonts::read_definitions(layers) -> name -> FontDefinition { path, color, border_color, overrides }` from the `bitmapfonts` blocks in `interface/*.gfx`.
- `localisation::read(layers, language) -> Localisation` with `get(key)`, precedence per the game's rules (research R2 confirms `replace/` and duplicate handling); color codes (`§Y...§!`) are kept in the string and stripped at layout.
- `states::read(layers) -> id -> StateDefinition { name_key, category, provinces, victory_points: Vec<(province, value)>, state buildings, province buildings, impassable }` from `history/states`.
- `regions::read(layers)` for `map/strategicregions` (id, name key, provinces).
- `positions::read_unit_stacks(layers) -> Vec<UnitStack { province, kind, x, y, z, rotation, scale }>` and `positions::read_buildings(layers) -> Vec<BuildingPosition { state, kind, x, y, z, rotation, adjacent_sea }>`.
- `terrain::read(layers) -> TerrainDefinitions { categories: name -> { color, is_water, naval }, palette: index -> { category, atlas_index } }` from `common/terrain/00_terrain.txt`, plus `map/default.map` keys.
- `adjacencies::read(layers) -> Vec<Adjacency { from, to, kind, through, start, stop, rule, comment }>`.
- `gfx::sprites(layers) -> name -> SpriteType { texture, frames, ... }` from `interface/*.gfx` `spriteTypes`, and `gui::read(layers, file)` returning the parsed `.gui` tree for layout constants.

## 5. Replay data used by the map

| Visual | Tables | Notes |
|---|---|---|
| Political fill, borders | Provinces (owner, controller, state), States, Countries, CountryLinks | occupation stripes where controller differs from owner |
| Country name, flag | Countries (tag, name), Politics (ruling_party, capital), PoliticalParties (token) | localisation key `<TAG>_<ideology>`; cosmetic-tag names are not recorded |
| Sea fills | Provinces (sea owner/controller if any), WarRelations for hostility tint | research R2 confirms what political mode draws at sea |
| Unit counters | Divisions (province, owners, hit_points, organisation, orders_group), OrdersGroups, LandCombats | no division template table exists yet, so the unit-type icon is a stated gap; count, flag, bars and combat state are available |
| Naval counters | TaskForces (province, mission), Ships, Fleets, NavalCombats | |
| Air bases | AirWings (air_base), ProvinceBuildings, StateBuildings | base level from buildings tables |
| Orders and fronts | ArmyOrders (path, target_states, order_type) | |
| Snow, mud, ice | WeatherProvinces (snow, water, mud), WeatherRegions, WeatherLimits | drives `SnowMudData` |
| Day and night | Clock (hour) | `DayNight_Hour_SunDir` from the game hour |
| Victory points, capitals | Provinces (victory_points), Politics (capital) | |

## 6. Evidence pointers

- Shaders: `gfx/FX/*.shader`, `*.fxh` (read directly).
- Defines: `common/defines/00_graphics.lua` (`LIGHT_*`, `BORDER_WIDTH`, `*_BORDER_FADE_*`, `DRAW_COUNTRY_NAMES_CUTOFF`, `PROVINCE_NAME_DRAW_DISTANCE`, `GRADIENT_BORDERS_*`, `CAMERA_*`, `SUN_*`, `DAY_NIGHT_FEATHER`, `WEATHER_*`).
- Decompile FTS hits (a729): `CAMERA_MIN_HEIGHT` at `0x14082e940`; `SUN_LATITUDE` at `0x140865e60`; `GRADIENT_BORDERS_THICKNESS_COUNTRY_LOW` at `0x1408499d0`; `unitstacks` at `0x14163d0a0`, `0x14163da30`; victory-point strings at `0x140e68e30`, `0x141e644d0` and others.
- Layouts: `interface/mapicons.gui`, `interface/mapicons.gfx`, `interface/core.gfx` (fonts).
