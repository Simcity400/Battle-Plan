# Lane report — B3 building `.mesh` importer + map-object placement (files-first)

Branch: `lane/b3-buildings`. Plan item **B3** (REPLAY_PRODUCTION_PLAN.md §Phase B).
Scope executed: the **FILES-FIRST half** — a clean-room PdxMesh decoder, building
placement derivation, entity→mesh binding, and (new this lane) authoritative
**material-texture resolution**. Final byte-parity vs a captured object pass is
capture-gated (CAP9) and deferred by directive; the gates below are all
file-decode gates. No `extractor/` files touched. No new runtime-state
dependency introduced (building *levels* remain the pre-existing recording
dependency, REPLAY_DATA_REQUIREMENTS item 10).

All game-file access is routed through `GameFileResolver` (MOD_COMPATIBILITY.md
"one rule"); no bare vanilla path join. Vanilla root used for the gates:
`<local-path> of Iron IV`.

## 1. State on entry vs. work done this lane

The decoder (`pdx_mesh.rs`), placement parser (`map_static_data.rs`), and
binding/coverage layer (`building_files.rs`) already existed on the base branch
and decoded/gated cleanly. This lane's net-new work:

- **Material-texture resolution law + gate** (was the one uncovered mission gate:
  "material texture paths resolve"). Added `resolve_building_material_textures`
  + `BuildingMaterialTextureReport`/`BuildingMaterialTextureRef`, wired unresolved
  textures into the layer's fail-closed `gaps` and new stats fields
  (`material_texture_count/resolved_count/unresolved`).
- **`.gfx` `meshsettings` texture-override parser** — `MeshSettingTexture` +
  `parse_mesh_settings`, added to `BuildingPdxmeshBinding.mesh_settings`. This is
  the authoritative texture source (see §4); without it the facility meshes'
  baked placeholder texture names are dangling.
- **buildings.txt position-stats gate** — x/y/z bounds + finiteness added to the
  vanilla static-data gate.

Files changed: `native/map_viewer/src/building_files.rs`,
`native/map_viewer/src/map_static_data.rs`, `native/map_viewer/src/lib.rs`.

## 2. PdxMesh (`pdxasset`) binary format — as derived from the shipped bytes

Magic `@@b@` (binary; 7 vanilla `.anim` under `anim_blend_test/` are the text
twin `@@t@`). The stream after the 4-byte magic is a flat token stream; there
are **no fixed struct offsets** — every field is length-prefixed. Token layout
(byte-exact, little-endian), verified against every shipped file:

| Token | Bytes | Field | Type / offset (relative to token start) |
|---|---|---|---|
| magic | 4 | `"@@b@"` | +0 |
| property | 1 | tag `!` (0x21) | +0 |
| | 1 | `name_len` u8 | +1 |
| | `name_len` | `name` (UTF-8) | +2 |
| | 1 | `type_char` (`i`/`f`/`s`) | +2+name_len |
| property `i` | 4 | `count` u32 | after type_char |
| | 4·count | `i32` values | |
| property `f` | 4 | `count` u32 | after type_char |
| | 4·count | `f32` values | |
| property `s` | 4 | `count` u32 (string array len) | after type_char |
| | per string: 4 | `byte_len` u32 (**includes 1 trailing NUL**) | |
| | | `byte_len` bytes (last byte 0x00) | |
| object | 1·d | run of `[` (0x5B), `d` = depth (1-based) | +0 |
| | until 0x00 | `name` bytes, NUL-terminated | +d |

Object nesting: an object at depth `d` becomes a child of the most recent object
at depth `d−1` (root = 0); properties attach to the most-recently-opened object.
Fails closed on unknown magic, unknown `type_char`, malformed framing,
non-NUL-terminated / interior-NUL strings, and any `pdxasset` version ≠ `[1,0]`.

### `.mesh` object tree (exhaustive inventory over the shipped set)

`root(pdxasset i[1,0])` → `[object]` (opt `loddist f`) → shape nodes (opt
`lod i1`) → `[[[mesh]]]` (repeatable) with `p f(3·v)`, opt `n f(3·v)` /
`ta f(4·v)` / `u0`,`u1 f(2·v)`, `tri i(3·t)`; child `[[[[aabb]]]]`
(`min f3`,`max f3`, always present), `[[[[material]]]]` (`shader s1` always;
opt `diff`/`n`/`spec s1`), opt `[[[[skin]]]]` (`bones i1`∈1..4, `ix i(4·v)`,
`w f(4·v)`); opt `[[[skeleton]]]` joint nodes (`ix i1`, opt `pa i1`, `tx f12`
= transposed inverse bind, rare `rcf i1`). `[locator]` → nodes (`p f3`, `q f4`,
opt `pa s1`, opt `tx f16`). Anything outside this inventory is collected by name
and reported, never skipped.

## 3. Placement / binding derivation (all FILE-derived)

- **Positions**: `map/buildings.txt`, rows `state;building;x;y;z;rotation;adj_sea`
  (`;`-separated, 7 fields). Rotation is radians.
- **Which types draw + variant count**: `common/buildings/00_buildings.txt`
  `show_on_map` / `show_on_map_meshes`.
- **type→entity→mesh**: `gfx/entities/buildings.gfx` (pdxmesh `name`/`file`/
  `scale` + `meshsettings`) and `gfx/entities/buildings.asset` (entity `name`/
  `pdxmesh`/`scale`, with `clone` inheritance). Candidate entity names:
  `building_<type>` or `building_<type>_<1..N>` when `show_on_map_meshes = N`.
- **Total scale** = `entity.scale × pdxmesh.scale` (both FILE).

## 4. Material-texture resolution law (new)

Textures for a building shape are, in priority order:
1. **`.gfx` `meshsettings { texture_diffuse/normal/specular }`** for that pdxmesh,
   when present — authoritative, OVERRIDES the mesh's baked material. (Evidence:
   `facility_nuclear.mesh`'s baked material names a nonexistent
   `building_facility_nuclear_diffuse.dds`; the real textures
   `facility_diffuse.dds` / `nuclearreactor_diffuse.dds` live only in the `.gfx`.)
2. Else the mesh's **baked `material` `diff`/`n`/`spec`** names.

Resolution of a texture *filename* is **filename-keyed across `gfx/models`**
(the engine's texture manager is not strictly mesh-dir-relative): tried mesh-dir
join → root-relative → basename index over `gfx/models/**.dds`, all resolver-
routed so mod overrides win. Evidence forcing the filename index: `nonormal.dds`
resolves at `gfx/models/` (parent of the mesh dir); `v2_rocket_launchsite_*.dds`
(referenced by `rocket_site.mesh` under `buildings/`) live under
`gfx/models/units/planes/`.

## 5. Gate numbers (all green; numeric, no eyeballing)

| Gate | Result |
|---|---|
| `pdx_mesh::gate_a` — every vanilla mesh/anim parses | **436 mesh + 535 anim** (964 binary + 7 text) parse; **785 validation rows clean**; 40 property patterns; 0 mesh/anim/validation failures |
| `pdx_mesh::gate_b` — diverse semantic checks | bunker building mesh **1033 v / 974 t**, PdxMeshSnow, aabb_dev 0.000, uv[0.001,1.993]; unit/ship carry skeleton+skin |
| index buffers in-range | every `tri` index `< vert_count` (per-mesh, gate_a validation_failures empty) |
| `building_files::gate_vanilla_building_binding_coverage` | 29 `show_on_map` types; **28 fully resolved; unresolved tail = 1: `fuel_silo`** (no `building_fuel_silo` entity ships) |
| `building_files::gate_vanilla_building_material_textures_resolve` (new) | 24 building models, **93 texture refs, 93 resolved, 0 unresolved** |
| `map_static_data::gated_vanilla_...` — buildings.txt + position stats (new bounds) | **66,664 rows**; x[11.0, 5616.0] y[9.10, 23.80] z[2.7, 2047.0] (within 5632×2048 map), rotation[−6.040, 6.280] rad |
| `cargo build -p map_viewer` | green (only pre-existing unrelated dead-code warnings) |
| `cargo test -p map_viewer` | **621 passed / 0 failed / 1 ignored** (lib) + integration bins green |
| clippy on touched files | 0 errors/warnings in `building_files.rs` / `map_static_data.rs` (pre-existing errors elsewhere untouched) |

## 6. FILE vs UNKNOWN classification

| Parameter | Class | Note |
|---|---|---|
| Mesh geometry (p/n/ta/u0/u1/tri/aabb/boundingsphere) | **FILE** | byte-exact from `.mesh`; index+aabb self-consistency gated |
| Material shader name + baked texture names | **FILE** | from `.mesh` material node |
| `.gfx` `meshsettings` texture overrides + shader | **FILE** | authoritative; parsed this lane |
| Texture file resolution (filename-keyed across gfx/models) | **FILE** | 93/93 resolve; resolver-routed |
| type→entity→mesh binding + scales | **FILE** | `.gfx`/`.asset`; clone inheritance resolved |
| Placement pos / rotation / state | **FILE** | `map/buildings.txt` (66,664 rows) |
| `show_on_map` / `show_on_map_meshes` | **FILE** | `00_buildings.txt` |
| `loddist` values | **FILE** | present on the meshes that ship it |
| **Which slots are actually built (building levels)** | **UNKNOWN — recording** | REPLAY_DATA_REQUIREMENTS item 10; layer emits 0 instances fail-closed until fed |
| model→world→map transform (terrain snap, nudge, y-from-heightmap) | **UNKNOWN — CAP9** | mesh verts are model-space; engine placement transform not in files |
| LOD switch rule (which LOD at which camera height) | **UNKNOWN — CAP9** | `loddist` values are FILE; the switch/cull logic is engine-C++ |
| Runtime shape-index / mesh selection at draw | **UNKNOWN — CAP9** | multi-`meshsettings` index→drawn-mesh mapping |
| Snow/season material variant selection (PdxMeshSnow vs Advanced) | **UNKNOWN — engine state** | variant is FILE; the runtime switch is not |
| Damage/burning/`_destroyed` model selection | **UNKNOWN — recording** | building damage state (already requirements-listed) |
| Byte-parity of the rendered object pass | **UNKNOWN — CAP9** | the definition-of-done for the render half |

No parameter is left as an unlabelled FIT.

## 7. What CAP9 must capture to finish B3 (render half)

CAP9 = tree instance buffer + `cities_pass`/`map_objects_pass` (per the plan's
capture table). To close the UNKNOWN rows above and reach byte parity, the
capture must record, for a close-zoom frame containing buildings:

1. The **per-object world transform** (model→world matrix) actually used —
   including terrain-height snap, spawn-point nudge, and the y taken from the
   heightmap — so the file-space verts can be placed byte-exactly.
2. **Which LOD / mesh index** is bound at the captured camera height (to confirm
   the `loddist` switch rule and multi-`meshsettings` selection).
3. The **bound texture SRVs** for each draw (to confirm the meshsettings +
   filename-index resolution law resolves to the same files the engine binds).
4. The **material variant** chosen at runtime (snow vs non-snow) and the
   shader/effect bound (to confirm PdxMeshStandard/Advanced/Snow selection).
5. Draw order + OM/depth state for the object pass, and pre/post render-target
   payloads, per MAP_3D_OBJECTS_PLAN.md's `map_objects_pass` record spec.

Until CAP9, visible building rendering stays product-blocked (as it is today:
`replay_app` blocks `--buildings-from-files`); this lane keeps the fail-closed
model and delivers the full file-decode substrate the render half will consume.
