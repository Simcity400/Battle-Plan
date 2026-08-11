# Snow and mud production law (XMAP07)

Status: shipping through the shared XMAP07/XMAP22 route.

## Authority

The rendered snow/mud surface is the typed passive world-global
`_xmap07_snow_mud_runtime` carrier for the current replay hour. It binds:

- the exact CPU pre-upload `B8G8R8A8_UNORM` payload produced at authenticated
- dynamic width and height, decoded-payload SHA-256, and the
  content-addressed blob identity;
- producer epoch, frontier, reset/carry-forward, completeness, executable, and
  static-evidence identity; and

XMAP07 verifies the carrier against the current frame and active map extent,
uploads the four planes unchanged, and supplies the same retained texture and
fade authority to terrain, trees, borders, rivers, water, and XMAP10 grading.
The XMAP22 v4 receipt binds `xmap07_snow_mud_authority_sha256` independently of
the water execution receipt.

## Texture and shader law

The admitted effective `gfx/FX/pdxmap.shader` and
`gfx/FX/standardfuncsgfx.fxh` define the consumer:

- the texture is one quarter of the active map extent on each axis;
- world addressing is the FOW-style `(x + 0.5, z + 0.5)` map transform with the
  active engine map constants;
- `.r`/`.a` are the two mud fade endpoints and `.b`/`.g` are the two snow fade
  endpoints; and
- displayed mud and snow are the shader's linear interpolation by the carried
  `SnowMudFade` value.

Effective game/mod content is resolved through the manifest-restricted VFS.
The shipped shader and include closure are authority; Replay does not substitute
a hard-coded vanilla shader or reinterpret the two endpoint planes as calendar
months.

## Semantic weather separation

Per-province snow and mud rows describe gameplay/weather semantics. They cannot
reconstruct the dense texture's two spatial endpoints or its global fade and
therefore never authorize a map frame. The files-derived `min_snow_level` and
weather-period parser in `map_viewer/src/snow_law.rs` remains a diagnostic law,
not a rendering fallback.

## Fail-closed conditions

The XMAP07 capability is unavailable when the carrier is absent, stale,
incomplete, wrong-hour, wrong-build, wrong-extent, malformed, or disagrees with
its payload/blob hashes or fade receipt. There is no selected-Present texture,
capture file, environment override, semantic rasterization, prior-frame guess,
or files-only floor fallback in product rendering.

## Evidence and implementation

The exact-a729 producer/fade evidence is retained under
`re_evidence/rendering_laws/XMAP07/`, including broker job
`20260720T050936Z-w4-g3-snowmud-owner-fade-producer-7d640b15`.
The carrier contract is implemented in
`portable_replay/native/render_stream/src/xmap07_snow_mud.rs`; preparation and
execution are in `map_viewer/src/production_features/xmap07.rs` and
`map_viewer/src/xmap07_d3d_handoff.rs`.
