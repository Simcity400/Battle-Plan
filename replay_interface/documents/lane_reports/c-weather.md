# LANE REPORT — lane/c-weather (B13 + Phase-C weather half)

**Mission:** make recorded weather LOOK RIGHT — wire the recording's weather /
SnowMud state through the already-mined weather/snow render laws, and enumerate
what still blocks "weather looks normal and nice" end-to-end.

Worktree: `C:\Users\Example\hoi4_lanes\c-weather` (branch `lane/c-weather`).
Cargo root: `replay_interface\native\`.

---

## What got wired

### 1. Recording-fed `SnowMudData` synthesis (B13 product path) — NEW MODULE

`map_viewer/src/snow_mud_feed.rs` (new; registered + exported in `lib.rs`).

- `synthesize_snow_mud_from_levels(inputs, fade_phase)` rasterizes the recording
  feed — per-province `{snow_0_255, mud_0_255}` (the shape of `replay_core`'s
  `SnowMudView.by_province` / `SnowMudCell`) — into the exact `1408×512`
  `B8G8R8A8_UNORM` `SnowMudData` payload the terrain shader samples, **replacing
  the zero-fill** that caused the documented "zero SnowMudData winter confound".
- `synthesize_snow_mud_from_resolver(levels, fade_phase, resolver)` is the
  production entry point: mod-aware `provinces.bmp` + `definition.csv` load, then
  rasterize with `flip_source_y = true`.
- **Addressing/orientation reuses SNOW1 exactly** (same quarter-res block-center
  sample + south-origin Y-flip as `snow_law::synthesize_snow_floor_texture`); a
  unit test pins that the recording feed and the files-only floor light up the
  *identical* texels, so they can never disagree on orientation.
- **Lossless-w.r.t.-recording collapse (documented, not a fit):** the engine
  texture packs two temporal keyframes per channel and blends by `SnowMudFade`;
  the recording supplies ONE resolved current level per province, so the current
  level is written into BOTH fade endpoints (like SNOW1's
  `to_snow_mud_payload`) → displayed value is exact for any `SnowMudFade`.

**Fixture gate (`fx_snow_mud_accumulator`).** Expectations derived from the
fixture generator SOURCE (`replay_fixture_gen/src/main.rs`):
`snow=min(tick·21,180)`, `mud= tick<5 ? tick·9 : max(45−(tick−5)·5,0)`,
prov 1001 `snow=(snow+18).min(220)`, `mud/2`. At the fixture probe (tick 8) the
rasterized terrain-input snow field == ground truth: prov 1000 snow **168**, mud
**30**; prov 1001 snow **186**, mud **15** — and fade-invariant. A second test
sweeps all 12 ticks and asserts equality at every hour.

### 2. Weather phenomenon → overlay wiring

`map_viewer/src/weather_law.rs`: new `WeatherType::from_recording_kind(kind)`
maps the recorded `WeatherRegionRow.weather_kind` onto the WEATHER1 overlay type,
accepting both the authoritative `common/weather.txt` `weather_effects` KEYS and
the friendly aliases the fixtures emit (`clear→no_phenomenon`, `rain→rain_light`,
`storm→rain_heavy`; the alias map is LABELLED an inference, not a file key).

**Fixture gate (`fx_weather_sweep`).** For each of the 6 phenom names the fixture
emits (`clear/rain/storm/snow/blizzard/sandstorm`), the test asserts the mapped
`WeatherType` and that it correctly reaches (or declines) a cloud overlay through
the shipped-file binding chain (`no_phenomenon`/`sandstorm` → no `fading_entity`;
the rest → a cloud layer). Fail-closed on an unknown kind.

### On the "weather CB0 params" scope item

The mined **terrain**-shader law exposes exactly ONE weather-driven scalar:
`SnowMudFade` (`vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z`, CB0 offset 248,
already writable via `shader_meta::PoliticalSceneCb0.snow_mud_fade`). This lane
threads the recording's `fade_phase` to that scalar (carried on
`RecordedSnowMudTexture.fade`). The airborne weather overlay is a **particle**
system (WEATHER1), NOT a terrain-shader CB0 pass — so there is no separate
"weather CB0" to wire; the terrain path's entire weather surface is
snow/mud + `SnowMudFade`, which is now fed.

---

## Gate numbers

| Gate | Result |
|---|---|
| `cargo build -p map_viewer` | OK (only pre-existing warnings) |
| `cargo test -p map_viewer --lib` | **626 passed / 0 failed / 1 ignored** (was 618; **+8** new: 7 `snow_mud_feed` + 1 `weather_law`) |
| `cargo test -p map_viewer` (full, all bins/doctests) | all green, **0 failed** |
| `cargo fmt -p map_viewer --check` | clean |
| `cargo test -p replay_core` | green, **0 failed** — `roundtrip_gates` **17**, `world_timeline` **7**, `fixture_conformance` **3**, `make_demo_recording` **3** (unchanged; replay_core untouched) |

All parameters FILE/CAPTURE/FIT/UNKNOWN-labelled; no unlabelled guesses. No
forbidden files touched (no `extractor/`, `replay_core`, `replay_app` render
blocks, or fixtures modified).

---

## Honest remaining-gap table → "weather looks nice"

| Gap | State | What it needs / closure path |
|---|---|---|
| **Final replay_app bind of the SnowMud payload** | law + fixture gate DONE this lane; bind NOT wired (replay_app render is off-limits to this lane) | Hook points identified: substitute `RecordedSnowMudTexture.payload` as the `snow_mud_data.rgba` bundle resource (same pattern as `frame_bundle::substitute_synthesized_terrain_id_map`), and set `PoliticalSceneCb0.snow_mud_fade = Some(tex.fade)`. Both plumbing halves already exist (EXTRACTION_NEEDS A2 "READY"); the missing call is `SnowMudView → synthesize_snow_mud_from_resolver → substitute + fade`. Owner: render-integration lane. |
| **The SnowMud feed itself (extractor side)** | OPEN (EXTRACTION_NEEDS A2/A9) | Extractor must emit per-hour per-province `{snow_level_0_255, mud_level_0_255}` (+ ideally the two fade endpoints). Files-only graceful degrade already exists: SNOW1 `synthesize_snow_floor_from_resolver` (min_snow_level date floor — small Arctic/deep-winter band only). |
| **Distinct fade-0/fade-1 endpoints** | collapsed (both = current level) | Only meaningful if the extractor emits the two keyframe levels; documented in REPLAY_DATA_REQUIREMENTS §6. Until then one level/province is lossless w.r.t. the recording. |
| **Airborne weather particles (rain/snow/blizzard clouds)** | law DONE (WEATHER1); NOT implemented — **v1 descope candidate (plan C7)** | Per mission, particles NOT built. Decision needed: descope for v1 (draw nothing, or a static representative cloud billboard from the file-derived peak-curve frame via `render_weather_overlay`) vs a particle sim. If kept: a `weather_pass` capture (WEATHER_LAW UNTRACED U2–U5) recording the particle instance buffer, fog CB, and applied fade factor. FILE law is ready the moment a decision lands. |
| **Weather-layer camera/zoom fade shape** | constants FILE-cited; combine formula UNTRACED (U3) | `weather_camera_fade` is a labelled reconstruction. Closure: a multi-zoom capture recording the applied per-layer fade factor vs camera height. Only matters once particles are drawn. |
| **`ApplyDistanceFog` on particles (`FOG` define)** | UNTRACED (U4), identity for now | Needs the runtime fog CB from a `weather_pass` capture. Particle-only. |
| **Cloud shadows on terrain** | UNKNOWN (no shipped-shader evidence) | Whole-tree FX scan found no cloud/weather/shadow coupling; the game clouds are billboards above the map, so ground cloud-shadows are likely absent. Closure if pursued: capture over a clouded region, compare ground luminance under vs beside a cloud. Low priority. |
| **Storm / weather map-mode icons** | UNKNOWN, out of WEATHER1 scope | The strategic-region weather map icons are a UI/map-icon layer (`interface/*.gfx`), not the airborne particle overlay. Would need interface-gfx RE. Low priority for "map looks nice". |
| **Sandstorm ground dust** | not implemented (U6) | `sandstorm` has no cloud `fading_entity`; its foreground dust `.asset` is out of the "clouds-from-altitude" scope. Port separately if wanted (same `particle.shader` law). |

### Winter parity smoke gate (scope item 4) — NOT added, by the rules

The mined RenderDoc oracle set (`%LOCALAPPDATA%\hoi4_renderdoc\mined`) is a single
**1936-January political frame** (`hoi4_vanilla_frame9232`) — a live capture with
**no recording-side per-province SnowMud feed**. A faithful parity gate would
require reconstructing that frame's accumulated per-province snow/mud INPUT, which
is seeded sim state not recoverable from files (SNOW_LAW §4) and has no recording
attached. Per the no-fake-inputs rule I did **not** add a gate that would fabricate
the input to pass. Feeding the capture's own SnowMudData SRV back to itself would
be a tautology (and would not exercise the province rasterizer). The sanctioned
B13 gate is the `fx_snow_mud_accumulator` fixture (plan: "B13 rides Phase A
fixtures, not a capture"), which this lane implements.

---

## Requirements additions (this worktree)

- `REPLAY_DATA_REQUIREMENTS.md` §6: added a "Refined 2026-07-09" note — (a)
  **weather_kind vocabulary**: prefer the recording emit the `weather_effects`
  KEY, not a friendly alias; (b) **fade endpoints**: one level/province is
  lossless, the fade phase is only visually meaningful with distinct fade-0/fade-1
  endpoint levels.
- `EXTRACTION_NEEDS.md` §A row A9: replay-side wiring LANDED note + the
  weather_kind-key preference.

No genuinely new *fields* were added (weather/SnowMud are item 6 / A2 / A9
already); these are semantics clarifications the wiring surfaced.

---

## Files changed

- `map_viewer/src/snow_mud_feed.rs` — NEW (recording-fed SnowMudData synthesis + 7 tests).
- `map_viewer/src/lib.rs` — module registration + exports (2 lines).
- `map_viewer/src/weather_law.rs` — `WeatherType::from_recording_kind` + 1 fixture test.
- `documents/REPLAY_DATA_REQUIREMENTS.md`, `documents/EXTRACTION_NEEDS.md` — requirement clarifications.
- `documents/lane_reports/` unchanged; this report at `LANE_REPORT.md`.
