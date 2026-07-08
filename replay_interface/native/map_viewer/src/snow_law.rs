//! SNOW1 — snow / mud "winter law", files-first, capture-validated.
//!
//! This module encodes what the game engine's `SnowMudData` runtime texture
//! (the `SnowMudTexture` SRV, sampler slot 7 in `gfx/FX/pdxmap.shader`) means,
//! how it is addressed in world space, and which parts of it are a
//! *deterministic function of the replay date and the shipped installed content*
//! versus which parts are *weather-simulation state* that no shipped file can
//! reproduce byte-for-byte.
//!
//! # Where the law comes from (shipped files, source of truth)
//!
//! ## The texture and how it is sampled
//!
//! `gfx/FX/pdxmap.shader` declares `SnowMudData` at sampler `Index = 7`
//! (Linear filter, Wrap address) and samples it exactly once per terrain pixel
//! (pdxmap.shader:319):
//! ```text
//! float4 vMudSnow = GetMudSnowColor( Input.prepos, SnowMudData );
//! ```
//! `Input.prepos` is the world-space terrain position (`x`, height, `z`).
//!
//! `gfx/FX/standardfuncsgfx.fxh:231-234` defines the addressing law (identical
//! to the Fog-of-War texture's UV law two functions up at fxh:193):
//! ```text
//! float4 GetMudSnowColor( float3 vPos, in sampler2D MudSnowTexture ) {
//!   return tex2D( MudSnowTexture,
//!     float2( ((vPos.x + 0.5f) / MAP_SIZE_X) * FOW_POW2_X,
//!             ((vPos.z + 0.5f) / MAP_SIZE_Y) ) * FOW_POW2_Y );
//! }
//! ```
//! `MAP_SIZE_X`, `MAP_SIZE_Y`, `FOW_POW2_X`, `FOW_POW2_Y` are engine-injected
//! constants (they appear in no shipped text file). The captured runtime
//! constants (`live_PAN2/map_frame_bundle.json`) measure them as
//! `MAP_SIZE_X = 5632`, `MAP_SIZE_Y = 2048`, `FOW_POW2_X = FOW_POW2_Y = 1.0`.
//! `5632 x 2048` is exactly the vanilla `provinces.bmp` / `terrain.bmp` /
//! `heightmap.bmp` resolution, so the world spans `[0, 5632] x [0, 2048]` in
//! `(x, z)` and the SnowMud UV is simply `((x+0.5)/5632, (z+0.5)/2048)`.
//!
//! The `SnowMudData` texture itself is `1408 x 512` (measured in every
//! capture, `source_dxgi_format = 87` = `B8G8R8A8_UNORM`). `1408 = 5632 / 4`
//! and `512 = 2048 / 4`, so the texture is **quarter map resolution on each
//! axis** and the Linear sampler blends between quarter-res texels. See
//! [`SNOW_MUD_TEXTURE_WIDTH`] / [`SNOW_MUD_TEXTURE_HEIGHT`] and
//! [`world_to_snow_mud_texel`].
//!
//! ## What the four channels mean (packing / temporal-blend law)
//!
//! `standardfuncsgfx.fxh` decodes the sampled `float4 vMudSnowColor`:
//! * `GetMud` (fxh:243): `lerp( vMudSnowColor.r, vMudSnowColor.a, SnowMudFade )`
//!   -> mud is `.r` (the fade-0 endpoint) blended toward `.a` (fade-1).
//! * `GetSnow`/GetWinter (fxh:261): `lerp( vMudSnowColor.b, vMudSnowColor.g,
//!   SnowMudFade )` -> snow/"winter" is `.b` (fade-0) blended toward `.g`
//!   (fade-1).
//!
//! So the RGBA channels pack, per texel:
//! `r = mud_fade0`, `a = mud_fade1`, `b = snow_fade0`, `g = snow_fade1`
//! (see [`SnowMudChannels`]). What the shader PROVES is only which stored byte
//! is the fade-0 endpoint and which is the fade-1 endpoint; the common gloss of
//! these as "previous month" / "current month" keyframes is an *inference*
//! about the C++ writer, not shown by any shipped shader — this module names
//! them by fade endpoint to stay honest. `SnowMudFade` is
//! `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z` (CB0 offset 248; the float4
//! is at CB0 offset 240 — see [`CB0_OFFSET_SNOW_MUD_FADE`]) and is the engine's
//! temporal cross-fade in `[0,1]`: `0` shows the fade-0 endpoint, `1` the
//! fade-1 endpoint.
//!
//! ## Per-region weather source (the deterministic FLOOR)
//!
//! `map/strategicregions/*.txt` carry, per strategic region, a
//! `weather = { period = { ... } ... }` block with twelve monthly `period`
//! sub-blocks. Each period has:
//! * `between = { START END }` — a `DAY.MONTH0` window (month is 0-based; day
//!   is 0-based within a nominal 31-slot month). E.g. `{ 0.0 30.0 }` = Jan,
//!   `{ 0.10 29.10 }` = Nov, `{ 0.11 30.11 }` = Dec. See [`WeatherPeriod`] and
//!   [`RegionWeather::period_for_date`], which selects the active period for a
//!   replay date with year-wrap handling.
//! * `temperature = { MIN MAX }` — a stochastic per-day temperature range.
//! * `snow`, `blizzard`, `rain_light`, `rain_heavy`, `mud`, `sandstorm`,
//!   `arctic_water`, `no_phenomenon` — **probabilities** the engine rolls each
//!   simulated day; NOT deterministic state.
//! * `min_snow_level` — a deterministic floor in `[0, 1]` on the snow channel
//!   for that region during that period. Across all 304 vanilla regions its
//!   observed values are `{0.0, 0.01, 0.05, 0.1, 0.2, 0.3}` (max 0.3, in the
//!   deep-Siberian regions in January). This is the ONLY per-region snow
//!   number that is a deterministic function of the date. NOTE: in the two
//!   analysed captures the floor-like small snow values appear in the fade-1
//!   endpoint (`.g`) while the fade-0 endpoint (`.b`) is zero, and both were
//!   captured at `SnowMudFade = 0`; so `min_snow_level` was NOT a
//!   currently-displayed floor at those instants — it sits in the fade target.
//!
//! ## Snow is accumulated simulation state (`common/weather.txt`)
//!
//! `common/weather.txt` proves the snow channel is *accumulated* sim state,
//! not a direct function of the date. Its `snowing` block (weather.txt:90):
//! `snow_gain_on_snowing = 1.0`, `snow_gain_on_blizzard = 5.0`,
//! `snow_gain_max = 80.0`; `snow_visual_min = 128` ("Visual fix, so even tiny
//! amount of snow can be visible. (0-255)" — this is exactly why captured snow
//! bytes cluster at 128); `snow_melt_temperature = { 0.0 12.0 }` /
//! `snow_melt_multiplier = { 0.001 0.9 }` (melt by temperature); and
//! `duration = { snow = { 5 20 } }` (random-length snow spells). Game
//! bookmarks call `randomize_weather` with a seed (1936 uses `12345`, 1939
//! uses `22345`). So the map snow at any instant
//! is the running accumulate/melt integral of pseudo-random daily rolls — it
//! needs the runtime/save weather state and RNG history, which shipped files +
//! date alone do not provide.
//!
//! # The LAW: what this module claims is deterministic
//!
//! For a replay date, files-first and deterministically:
//! 1. **Addressing law** — world `(x, z)` -> SnowMud texel via
//!    [`world_to_snow_mud_texel`]; the texture is quarter-res and world spans
//!    `[0, MAP_SIZE_X] x [0, MAP_SIZE_Y]`.
//! 2. **Packing / temporal-blend law** — channels `(mud_fade0, snow_fade0,
//!    mud_fade1, snow_fade1)` and the `SnowMudFade` cross-fade (see
//!    [`SnowMudChannels`], [`decode_snow_mud_texel`]).
//! 3. **Per-region snow FLOOR** — [`RegionWeather::min_snow_floor_for_date`]
//!    gives each strategic region's `min_snow_level` for the active period.
//!    Rasterizing
//!    that floor into the texture via each region's provinces
//!    ([`synthesize_snow_floor_texture`]) is a deterministic lower bound on the
//!    snow (`.b`/`.g`) channels.
//!
//! # What is NOT derivable from shipped files + date alone (extractor item)
//!
//! The **actual per-texel snow and mud values above the `min_snow_level`
//! floor** are produced by the engine's C++ weather system per `weather.txt`:
//! each simulated day it rolls the `snow`/`blizzard`/`mud` probabilities and
//! samples the `temperature` range per province, ACCUMULATES snow
//! (`snow_gain_on_*`, capped at `snow_gain_max`) and MELTS it by temperature,
//! rasterizes two fade endpoints into `SnowMudData`, and cross-fades them with
//! `SnowMudFade`. This is a stateful, seeded pseudo-random simulation, so an
//! exact `SnowMudData` is not a closed function of (shipped files, date): it
//! needs the runtime/save weather state, RNG history, elapsed ticks, and fade
//! phase. Evidence it is simulation state rather than a files-only function of
//! the date:
//! * Three captures produce three distinct content hashes for `SnowMudData`
//!   (`restore_gate_1`=`cd2693a5..`, `live_PAN2`=`a94188fa..`, checked-in
//!   harness=`17b9454249922325`), and the harness capture is entirely zero —
//!   a legal game frame the deterministic floor alone cannot reproduce.
//! * `min_snow_level` maxes at `0.3`, but captured snow (`.g`) reaches `255`
//!   (`1.0`) only in the far-north rows (497-511 = Arctic, since world z=0 is
//!   SOUTH per `CalcGlobeNormal`); that saturation is accumulated sim state
//!   (with the `snow_visual_min = 128` visibility fix), not the
//!   `min_snow_level` floor.
//!
//! Therefore this module deliberately does NOT claim to synthesize a
//! byte-exact `SnowMudData`; it synthesizes the deterministic floor and
//! provides the addressing/packing law, and the validation bin characterizes
//! the residual honestly. The above-floor accumulated field is proposed as a
//! `REPLAY_DATA_REQUIREMENTS` extractor item (see documents/SNOW_LAW.md).

use crate::*;
use std::collections::BTreeMap;
use std::path::Path;

/// Directory of shipped strategic-region weather files.
pub const STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";
/// Shipped `provinces.bmp` (province-color -> province-id raster).
pub const SNOW_PROVINCES_BMP_SOURCE: &str = "map/provinces.bmp";
/// Shipped `definition.csv` (province-id -> color / type).
pub const SNOW_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";

/// Measured width of the `SnowMudData` runtime texture (all captures).
pub const SNOW_MUD_TEXTURE_WIDTH: usize = 1408;
/// Measured height of the `SnowMudData` runtime texture (all captures).
pub const SNOW_MUD_TEXTURE_HEIGHT: usize = 512;
/// `source_dxgi_format` measured in every `SnowMudData` capture
/// (`DXGI_FORMAT_B8G8R8A8_UNORM`).
pub const SNOW_MUD_SOURCE_DXGI_FORMAT: u32 = 87;
/// Byte length of a full `SnowMudData` payload (`W * H * 4`).
pub const SNOW_MUD_TEXTURE_BYTE_LEN: usize = SNOW_MUD_TEXTURE_WIDTH * SNOW_MUD_TEXTURE_HEIGHT * 4;

/// Vanilla world extent on X (`MAP_SIZE_X`), matching `provinces.bmp` width.
/// Used only as the documented default; the addressing law takes the extent as
/// an argument so a mod with a resized map still maps correctly.
pub const VANILLA_MAP_SIZE_X: f32 = 5632.0;
/// Vanilla world extent on Z (`MAP_SIZE_Y`), matching `provinces.bmp` height.
pub const VANILLA_MAP_SIZE_Y: f32 = 2048.0;

/// CB0 byte offset of `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed` (float4).
/// Derived from the `ConstantBuffer(0,0)` layout in `standardfuncsgfx.fxh`:
/// two `float4x4` (128) + `vGB..` float4 (16) + four virtual-light float4s
/// (64) + `vCamPos`+`HdrRange` (16) + `vCamLookAtDir`+`vGlobalTime` (16) = 240.
pub const CB0_OFFSET_SNOW_MUD_FADE_VECTOR: usize = 240;
/// CB0 byte offset of the scalar `SnowMudFade` (`.z` of the float4 above).
pub const CB0_OFFSET_SNOW_MUD_FADE: usize = CB0_OFFSET_SNOW_MUD_FADE_VECTOR + 8;

/// Number of `min_snow_level` distinct source levels observed in vanilla, for
/// documentation / sanity only.
pub const OBSERVED_MIN_SNOW_LEVELS: &[f32] = &[0.0, 0.01, 0.05, 0.1, 0.2, 0.3];

// ---------------------------------------------------------------------------
// Dates and weather periods
// ---------------------------------------------------------------------------

/// A calendar date used to key the snow law. The strategic-region `between`
/// windows are year-agnostic (they repeat every year), so only month/day
/// matter; `month` is 1..=12 and `day` is 1..=31 (human 1-based), matching the
/// convention already used by [`crate::SeasonDate`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SnowDate {
    /// 1-based month (1 = January).
    pub month: u8,
    /// 1-based day of month.
    pub day: u8,
}

impl SnowDate {
    pub fn new(month: u8, day: u8) -> Result<Self> {
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            Ok(Self { month, day })
        } else {
            Err(format!("invalid snow-law date {month:02}.{day:02}").into())
        }
    }

    /// Ordinal day-of-year in a nominal 31-day-month calendar (0..=371). The
    /// engine's `between` windows use a `DAY.MONTH` scheme on a 31-slot month,
    /// so period matching uses the SAME nominal calendar for both the window
    /// bounds and the queried date. This is intentional: we compare positions
    /// on the engine's own weather calendar, not on a real 365-day calendar.
    fn nominal_ordinal(self) -> u32 {
        (u32::from(self.month) - 1) * 31 + (u32::from(self.day) - 1)
    }
}

/// A parsed `period` sub-block from a strategic region's `weather` block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherPeriod {
    /// Inclusive start of the `between` window as a nominal `DAY.MONTH0` point.
    pub between_start: WeatherPoint,
    /// Inclusive end of the `between` window.
    pub between_end: WeatherPoint,
    /// Deterministic snow floor in `[0, 1]` for this period.
    pub min_snow_level: f32,
    /// Stochastic snow probability (NOT deterministic; kept for diagnostics).
    pub snow_probability: f32,
    /// Stochastic blizzard probability (diagnostics only).
    pub blizzard_probability: f32,
    /// Stochastic mud probability (diagnostics only).
    pub mud_probability: f32,
    /// `temperature = { MIN MAX }` (diagnostics only).
    pub temperature: [f32; 2],
}

/// A `between` window endpoint: a `DAY.MONTH0` point on the engine's nominal
/// 31-slot-per-month weather calendar (month is 0-based in the file).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct WeatherPoint {
    /// 0-based day within the month (0..=30 in vanilla).
    pub day: u8,
    /// 0-based month (0 = January .. 11 = December).
    pub month0: u8,
}

impl WeatherPoint {
    /// Parse a `DAY.MONTH0` token such as `"0.0"`, `"30.0"`, `"29.10"`.
    fn parse(token: &str) -> Result<Self> {
        let (day_s, month_s) = token
            .split_once('.')
            .ok_or_else(|| format!("weather `between` point {token:?} is not DAY.MONTH"))?;
        let day = day_s
            .parse::<u8>()
            .map_err(|e| format!("weather `between` day {day_s:?}: {e}"))?;
        let month0 = month_s
            .parse::<u8>()
            .map_err(|e| format!("weather `between` month {month_s:?}: {e}"))?;
        if month0 > 11 {
            return Err(format!("weather `between` month {month0} out of range 0..=11").into());
        }
        if day > 30 {
            return Err(format!("weather `between` day {day} out of range 0..=30").into());
        }
        Ok(Self { day, month0 })
    }

    /// Ordinal on the nominal 31-slot calendar (0..=371).
    fn nominal_ordinal(self) -> u32 {
        u32::from(self.month0) * 31 + u32::from(self.day)
    }
}

/// A strategic region's parsed weather law: id, display name, province list,
/// and the ordered monthly periods.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionWeather {
    pub region_id: u32,
    pub name: String,
    pub provinces: Vec<u32>,
    pub periods: Vec<WeatherPeriod>,
}

impl RegionWeather {
    /// The active period for `date`, if any window contains it (with year-wrap).
    pub fn period_for_date(&self, date: SnowDate) -> Option<&WeatherPeriod> {
        let ord = date.nominal_ordinal();
        self.periods.iter().find(|p| period_contains(p, ord))
    }

    /// The deterministic snow floor for `date`, or `None` if no period matches
    /// (fail-closed: a region with a gap in its weather calendar yields no
    /// floor rather than a silent zero).
    pub fn min_snow_floor_for_date(&self, date: SnowDate) -> Option<f32> {
        self.period_for_date(date).map(|p| p.min_snow_level)
    }
}

/// Inclusive containment of a nominal ordinal in a period's `between` window,
/// handling windows that wrap past the year end (start > end).
fn period_contains(period: &WeatherPeriod, ordinal: u32) -> bool {
    let start = period.between_start.nominal_ordinal();
    let end = period.between_end.nominal_ordinal();
    if start <= end {
        start <= ordinal && ordinal <= end
    } else {
        ordinal >= start || ordinal <= end
    }
}

// ---------------------------------------------------------------------------
// Channel packing / decode law
// ---------------------------------------------------------------------------

/// The four logical `SnowMudData` channels for one texel, decoded per the
/// `standardfuncsgfx.fxh` `GetMud`/`GetSnow` law. Values are the raw `[0,255]`
/// bytes (the shader treats them as `[0,1]` UNORM). Named by fade endpoint
/// (`fade0` = shown at `SnowMudFade=0`, `fade1` = shown at `SnowMudFade=1`)
/// because that is exactly what the shader proves; the "prev month / current
/// month" reading is an unproven inference about the C++ writer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnowMudChannels {
    /// `.r` — mud at the fade-0 endpoint.
    pub mud_fade0: u8,
    /// `.a` — mud at the fade-1 endpoint.
    pub mud_fade1: u8,
    /// `.b` — snow ("winter") at the fade-0 endpoint.
    pub snow_fade0: u8,
    /// `.g` — snow at the fade-1 endpoint.
    pub snow_fade1: u8,
}

impl SnowMudChannels {
    /// The snow value the shader displays for a given `SnowMudFade` in `[0,1]`,
    /// i.e. `lerp(.b, .g, fade)` (GetSnow, fxh:261), as `[0,1]`.
    pub fn displayed_snow(self, fade: f32) -> f32 {
        let fade = fade.clamp(0.0, 1.0);
        let a = f32::from(self.snow_fade0) / 255.0;
        let b = f32::from(self.snow_fade1) / 255.0;
        a + (b - a) * fade
    }

    /// The mud value the shader displays for a given `SnowMudFade` in `[0,1]`,
    /// i.e. `lerp(.r, .a, fade)` (GetMud, fxh:243), as `[0,1]`.
    pub fn displayed_mud(self, fade: f32) -> f32 {
        let fade = fade.clamp(0.0, 1.0);
        let a = f32::from(self.mud_fade0) / 255.0;
        let b = f32::from(self.mud_fade1) / 255.0;
        a + (b - a) * fade
    }
}

/// Decode one texel from a `B8G8R8A8_UNORM` payload at byte offset `i*4`.
///
/// Memory byte order for format 87 is `[B, G, R, A]`; the HLSL swizzles then
/// read `.r`=byte2, `.g`=byte1, `.b`=byte0, `.a`=byte3, so `snow` uses `.b`/`.g`
/// = bytes 0/1 and `mud` uses `.r`/`.a` = bytes 2/3.
pub fn decode_snow_mud_texel(payload: &[u8], texel_index: usize) -> Option<SnowMudChannels> {
    let base = texel_index.checked_mul(4)?;
    let end = base.checked_add(4)?;
    let px = payload.get(base..end)?;
    Some(SnowMudChannels {
        // px = [B, G, R, A]
        snow_fade0: px[0], // .b
        snow_fade1: px[1], // .g
        mud_fade0: px[2],  // .r
        mud_fade1: px[3],  // .a
    })
}

/// Encode one texel into `B8G8R8A8_UNORM` byte order `[B, G, R, A]`.
pub fn encode_snow_mud_texel(ch: SnowMudChannels) -> [u8; 4] {
    [ch.snow_fade0, ch.snow_fade1, ch.mud_fade0, ch.mud_fade1]
}

/// Read the scalar `SnowMudFade` from a captured CB0 buffer, or `None` if the
/// buffer is too short (fail-closed).
pub fn snow_mud_fade_from_cb0(cb0: &[u8]) -> Option<f32> {
    let end = CB0_OFFSET_SNOW_MUD_FADE.checked_add(4)?;
    let s = cb0.get(CB0_OFFSET_SNOW_MUD_FADE..end)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// ---------------------------------------------------------------------------
// Addressing law: world (x, z) -> SnowMud texel
// ---------------------------------------------------------------------------

/// The world extent used by the `GetMudSnowColor` UV law (`MAP_SIZE_X`,
/// `MAP_SIZE_Y`) and the FoW power-of-two correction (`FOW_POW2_X/Y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnowMudAddressing {
    pub map_size_x: f32,
    pub map_size_y: f32,
    pub fow_pow2_x: f32,
    pub fow_pow2_y: f32,
    pub texture_width: usize,
    pub texture_height: usize,
}

impl Default for SnowMudAddressing {
    fn default() -> Self {
        // Vanilla, matching the captured runtime constants.
        Self {
            map_size_x: VANILLA_MAP_SIZE_X,
            map_size_y: VANILLA_MAP_SIZE_Y,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: SNOW_MUD_TEXTURE_WIDTH,
            texture_height: SNOW_MUD_TEXTURE_HEIGHT,
        }
    }
}

impl SnowMudAddressing {
    /// The `GetMudSnowColor` UV for world `(x, z)` (fxh:233). Not clamped —
    /// the shader sampler is `Wrap`, so callers that need a texel index wrap.
    pub fn world_to_uv(self, x: f32, z: f32) -> (f32, f32) {
        let u = ((x + 0.5) / self.map_size_x) * self.fow_pow2_x;
        let v = ((z + 0.5) / self.map_size_y) * self.fow_pow2_y;
        (u, v)
    }

    /// The nearest SnowMud texel `(tx, ty)` for world `(x, z)`, wrapping like
    /// the `Wrap` sampler. `None` if the texture dimensions are zero.
    pub fn world_to_texel(self, x: f32, z: f32) -> Option<(usize, usize)> {
        if self.texture_width == 0 || self.texture_height == 0 {
            return None;
        }
        let (u, v) = self.world_to_uv(x, z);
        let w = self.texture_width as f32;
        let h = self.texture_height as f32;
        let tx = wrap_index(u * w, self.texture_width);
        let ty = wrap_index(v * h, self.texture_height);
        Some((tx, ty))
    }
}

/// Convenience wrapper over [`SnowMudAddressing::world_to_texel`] with the
/// vanilla addressing.
pub fn world_to_snow_mud_texel(x: f32, z: f32) -> Option<(usize, usize)> {
    SnowMudAddressing::default().world_to_texel(x, z)
}

/// Floor-and-wrap a floating texel coordinate into `0..dim` like a `Wrap`
/// sampler (handles negative and >= dim).
fn wrap_index(coord: f32, dim: usize) -> usize {
    if dim == 0 {
        return 0;
    }
    let d = dim as i64;
    let mut i = coord.floor() as i64 % d;
    if i < 0 {
        i += d;
    }
    i as usize
}

// ---------------------------------------------------------------------------
// Parsing strategic-region weather from files
// ---------------------------------------------------------------------------

/// Parse every strategic region's weather law from the resolver-routed
/// `map/strategicregions/*.txt`. Mod-aware (later mods win per relative path).
pub fn region_weather_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<u32, RegionWeather>> {
    let files = resolver.resolve_dir_files(STRATEGIC_REGIONS_DIR, Some("txt"))?;
    let mut out = BTreeMap::new();
    for (_rel, path) in &files {
        let text = read_latin1(path)?;
        if let Some(region) = parse_region_weather(&text).map_err(|e| {
            format!(
                "failed to parse strategic region weather in {}: {e}",
                path.display()
            )
        })? {
            out.entry(region.region_id).or_insert(region);
        }
    }
    if out.is_empty() {
        return Err(
            format!("no strategic-region weather parsed under {STRATEGIC_REGIONS_DIR}").into(),
        );
    }
    Ok(out)
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn region_weather_from_game_dir(game_dir: &Path) -> Result<BTreeMap<u32, RegionWeather>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    region_weather_from_resolver(&resolver)
}

/// Parse a single region file's text into a [`RegionWeather`], or `None` if the
/// file has no `id` (e.g. a stray/empty file). Fails closed on malformed
/// numbers inside a present `weather` block.
pub fn parse_region_weather(source: &str) -> Result<Option<RegionWeather>> {
    let text = strip_comments(source);
    let Some(region_id) = parse_u32_value(&text, "id") else {
        return Ok(None);
    };
    let name =
        parse_quoted_value(&text, "name").unwrap_or_else(|| format!("STRATEGICREGION_{region_id}"));
    let provinces = extract_block_after(&text, "provinces", 0)
        .map(|(inner, _)| parse_u32s(&inner))
        .unwrap_or_default();

    let periods = match extract_block_after(&text, "weather", 0) {
        Some((weather_inner, _)) => parse_weather_periods(&weather_inner)?,
        None => Vec::new(),
    };

    Ok(Some(RegionWeather {
        region_id,
        name,
        provinces,
        periods,
    }))
}

fn parse_weather_periods(weather_inner: &str) -> Result<Vec<WeatherPeriod>> {
    let mut periods = Vec::new();
    let mut cursor = 0usize;
    while let Some((period_inner, next)) = extract_block_after(weather_inner, "period", cursor) {
        periods.push(parse_weather_period(&period_inner)?);
        cursor = next;
    }
    Ok(periods)
}

fn parse_weather_period(period_inner: &str) -> Result<WeatherPeriod> {
    let (start_tok, end_tok) = parse_between(period_inner)?;
    let between_start = WeatherPoint::parse(&start_tok)?;
    let between_end = WeatherPoint::parse(&end_tok)?;
    let min_snow_level = parse_f32_value(period_inner, "min_snow_level").unwrap_or(0.0);
    let snow_probability = parse_f32_value(period_inner, "snow").unwrap_or(0.0);
    let blizzard_probability = parse_f32_value(period_inner, "blizzard").unwrap_or(0.0);
    let mud_probability = parse_f32_value(period_inner, "mud").unwrap_or(0.0);
    let temperature = parse_temperature(period_inner).unwrap_or([0.0, 0.0]);
    if !(0.0..=1.0).contains(&min_snow_level) {
        return Err(format!("min_snow_level {min_snow_level} out of [0,1]").into());
    }
    Ok(WeatherPeriod {
        between_start,
        between_end,
        min_snow_level,
        snow_probability,
        blizzard_probability,
        mud_probability,
        temperature,
    })
}

fn parse_between(period_inner: &str) -> Result<(String, String)> {
    let (inner, _) = extract_block_after(period_inner, "between", 0)
        .ok_or("weather period missing `between` block")?;
    let toks: Vec<&str> = inner.split_whitespace().collect();
    if toks.len() != 2 {
        return Err(format!("weather `between` expects two DAY.MONTH tokens, got {toks:?}").into());
    }
    Ok((toks[0].to_string(), toks[1].to_string()))
}

fn parse_temperature(period_inner: &str) -> Option<[f32; 2]> {
    let (inner, _) = extract_block_after(period_inner, "temperature", 0)?;
    let nums: Vec<f32> = inner
        .split_whitespace()
        .filter_map(|t| t.parse::<f32>().ok())
        .collect();
    if nums.len() == 2 {
        Some([nums[0], nums[1]])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Deterministic snow-floor synthesis
// ---------------------------------------------------------------------------

/// A rasterized deterministic snow-floor texture: for a replay date, each
/// SnowMud texel carries the `min_snow_level` of the strategic region covering
/// the province at that texel (0 where no region/province covers the texel).
///
/// This is a *lower bound* on the snow channels, not the full `SnowMudData`
/// (see the module docs on weather-sim state).
#[derive(Clone, Debug, PartialEq)]
pub struct SnowFloorTexture {
    pub width: usize,
    pub height: usize,
    /// `min_snow_level` in `[0,1]` per texel, row-major.
    pub floor: Vec<f32>,
    /// How many texels received a floor from a matched region period.
    pub covered_texels: usize,
    /// How many regions had no period for this date (fail-closed skips).
    pub regions_without_period: usize,
}

impl SnowFloorTexture {
    /// The floor at texel `(tx, ty)`, or 0.0 out of bounds.
    pub fn at(&self, tx: usize, ty: usize) -> f32 {
        if tx >= self.width || ty >= self.height {
            return 0.0;
        }
        self.floor[ty * self.width + tx]
    }

    /// Pack the floor into a `B8G8R8A8_UNORM` payload, writing the floor into
    /// BOTH the `snow_prev` (`.b`) and `snow_curr` (`.g`) channels so the
    /// displayed snow equals the floor for ANY `SnowMudFade`. Mud channels are
    /// left zero (mud is entirely weather-sim state — no deterministic floor).
    pub fn to_snow_mud_payload(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.width * self.height * 4];
        for (i, &f) in self.floor.iter().enumerate() {
            let byte = (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            let base = i * 4;
            out[base] = byte; // .b snow_prev
            out[base + 1] = byte; // .g snow_curr
                                  // out[base+2] .r mud_prev = 0
                                  // out[base+3] .a mud_curr = 0
        }
        out
    }
}

/// Inputs to snow-floor synthesis: the province raster (province-id per map
/// pixel) plus its dimensions, and the province->region map.
pub struct SnowFloorInputs<'a> {
    /// Province id per map pixel, row-major, `map_width * map_height`.
    /// `province_ids[y*w+x]` with `y = 0` at the TOP of the source
    /// `provinces.bmp` (the convention `load_provinces_bmp` produces).
    pub province_ids: &'a [u32],
    pub map_width: usize,
    pub map_height: usize,
    /// province id -> strategic region id.
    pub province_to_region: &'a BTreeMap<u32, u32>,
    /// strategic region id -> parsed weather.
    pub region_weather: &'a BTreeMap<u32, RegionWeather>,
    /// Addressing law (defaults to vanilla).
    pub addressing: SnowMudAddressing,
    /// Whether texture row 0 maps to the BOTTOM of the top-down province BMP
    /// (`true`), matching the vertical flip the shipped map textures in the
    /// same UV family use (`political_color_synthesis`'s
    /// `political_source_pixel_for_texel`, and terrain-id map, both flip Y
    /// before packing). Production callers pass `true`; unit tests that build
    /// a synthetic already-oriented raster pass `false`.
    pub flip_source_y: bool,
}

/// Load the province raster + region weather from a resolver and synthesize
/// the deterministic snow floor for `date`. This is the files-first entry
/// point production/validation callers use.
///
/// The province->region map is built from each region's own province list
/// (`load_strategic_regions`), and the province raster from
/// `provinces.bmp` + `definition.csv`. All reads are mod-aware.
pub fn synthesize_snow_floor_from_resolver(
    date: SnowDate,
    resolver: &GameFileResolver,
) -> Result<SnowFloorTexture> {
    let region_weather = region_weather_from_resolver(resolver)?;
    let mut province_to_region: BTreeMap<u32, u32> = BTreeMap::new();
    for (id, rw) in &region_weather {
        for p in &rw.provinces {
            province_to_region.entry(*p).or_insert(*id);
        }
    }

    let definition_csv = resolver.resolve_required(SNOW_DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(SNOW_PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    let inputs = SnowFloorInputs {
        province_ids: &bmp.ids,
        map_width: bmp.width,
        map_height: bmp.height,
        province_to_region: &province_to_region,
        region_weather: &region_weather,
        addressing: SnowMudAddressing::default(),
        // Y-FLIP required. `GetMudSnowColor` UV is `v = (z+0.5)/MAP_SIZE_Y`, and
        // per `CalcGlobeNormal` (standardfuncsgfx.fxh:347-355) world `z=0` is the
        // SOUTH pole side (`SOUTH_POLE_OFFSET`) and `z=MAP_SIZE_Y` is NORTH, so
        // SnowMud texture row 0 (v=0) = world z=0 = SOUTH and row 511 = NORTH.
        // `load_provinces_bmp` yields `ids` with row 0 = image top = NORTH, so
        // the top-down raster must be flipped into world-z order for the floor
        // to land in the right rows. Confirmed empirically: with the flip,
        // NH-winter (January) Siberia floors land in the high (north) texture
        // rows ~497-511, matching the ARCTIC snow seen in the captures.
        flip_source_y: true,
    };
    synthesize_snow_floor_texture(date, &inputs)
}

/// Rasterize the deterministic `min_snow_level` floor for `date` into a
/// quarter-resolution [`SnowFloorTexture`].
///
/// Each SnowMud texel corresponds to a `4x4` block of map pixels (quarter-res).
/// We sample the province at each texel's center map pixel, look up its region,
/// select the region's active period for `date`, and take `min_snow_level`.
/// Fails closed: a region with no matching period contributes no floor.
pub fn synthesize_snow_floor_texture(
    date: SnowDate,
    inputs: &SnowFloorInputs<'_>,
) -> Result<SnowFloorTexture> {
    let width = inputs.addressing.texture_width;
    let height = inputs.addressing.texture_height;
    if width == 0 || height == 0 {
        return Err("snow floor synthesis: zero texture dimensions".into());
    }
    if inputs.province_ids.len() != inputs.map_width * inputs.map_height {
        return Err(format!(
            "snow floor synthesis: province raster {} != {}x{}",
            inputs.province_ids.len(),
            inputs.map_width,
            inputs.map_height
        )
        .into());
    }

    // Precompute each region's floor for this date once (fail-closed).
    let mut region_floor: BTreeMap<u32, f32> = BTreeMap::new();
    let mut regions_without_period = 0usize;
    for (id, rw) in inputs.region_weather {
        // Regions with no periods at all (e.g. sea regions lacking weather)
        // simply have no floor and are not counted as "without period".
        if rw.periods.is_empty() {
            continue;
        }
        match rw.min_snow_floor_for_date(date) {
            Some(f) => {
                region_floor.insert(*id, f);
            }
            None => regions_without_period += 1,
        }
    }

    let mut floor = vec![0.0f32; width * height];
    let mut covered_texels = 0usize;
    // Quarter-res block size on each axis (rounded; vanilla = 4).
    let bx = (inputs.map_width as f32 / width as f32).max(1.0);
    let by = (inputs.map_height as f32 / height as f32).max(1.0);
    for ty in 0..height {
        for tx in 0..width {
            // Center map pixel of this texel's block.
            let mx = ((tx as f32 + 0.5) * bx) as usize;
            let unflipped_my = ((ty as f32 + 0.5) * by) as usize;
            if mx >= inputs.map_width || unflipped_my >= inputs.map_height {
                continue;
            }
            // Match the shipped UV-family vertical flip: texture row 0 -> BMP
            // bottom row (south). See SnowFloorInputs::flip_source_y.
            let my = if inputs.flip_source_y {
                inputs.map_height - 1 - unflipped_my.min(inputs.map_height - 1)
            } else {
                unflipped_my
            };
            let pid = inputs.province_ids[my * inputs.map_width + mx];
            let Some(region_id) = inputs.province_to_region.get(&pid) else {
                continue;
            };
            if let Some(&f) = region_floor.get(region_id) {
                if f > 0.0 {
                    floor[ty * width + tx] = f;
                    covered_texels += 1;
                }
            }
        }
    }

    Ok(SnowFloorTexture {
        width,
        height,
        floor,
        covered_texels,
        regions_without_period,
    })
}

// ---------------------------------------------------------------------------
// Validation against captured SnowMudData payloads
// ---------------------------------------------------------------------------

/// Per-payload decoded statistics for a captured `SnowMudData` texture.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedSnowMudStats {
    pub width: usize,
    pub height: usize,
    /// Nonzero-texel counts per logical channel (by fade endpoint).
    pub mud_fade0_nonzero: usize,
    pub mud_fade1_nonzero: usize,
    pub snow_fade0_nonzero: usize,
    pub snow_fade1_nonzero: usize,
    /// Max byte per logical channel.
    pub mud_fade0_max: u8,
    pub mud_fade1_max: u8,
    pub snow_fade0_max: u8,
    pub snow_fade1_max: u8,
    pub total_texels: usize,
}

/// Decode channel statistics from a captured `SnowMudData` payload.
pub fn captured_snow_mud_stats(
    payload: &[u8],
    width: usize,
    height: usize,
) -> Result<CapturedSnowMudStats> {
    let total = width
        .checked_mul(height)
        .ok_or("snow-mud stats: dimensions overflow")?;
    if payload.len() < total * 4 {
        return Err(format!(
            "snow-mud stats: payload {} shorter than {}x{}x4 = {}",
            payload.len(),
            width,
            height,
            total * 4
        )
        .into());
    }
    let mut s = CapturedSnowMudStats {
        width,
        height,
        mud_fade0_nonzero: 0,
        mud_fade1_nonzero: 0,
        snow_fade0_nonzero: 0,
        snow_fade1_nonzero: 0,
        mud_fade0_max: 0,
        mud_fade1_max: 0,
        snow_fade0_max: 0,
        snow_fade1_max: 0,
        total_texels: total,
    };
    for i in 0..total {
        let ch = decode_snow_mud_texel(payload, i).ok_or("snow-mud stats: short texel")?;
        if ch.mud_fade0 > 0 {
            s.mud_fade0_nonzero += 1;
        }
        if ch.mud_fade1 > 0 {
            s.mud_fade1_nonzero += 1;
        }
        if ch.snow_fade0 > 0 {
            s.snow_fade0_nonzero += 1;
        }
        if ch.snow_fade1 > 0 {
            s.snow_fade1_nonzero += 1;
        }
        s.mud_fade0_max = s.mud_fade0_max.max(ch.mud_fade0);
        s.mud_fade1_max = s.mud_fade1_max.max(ch.mud_fade1);
        s.snow_fade0_max = s.snow_fade0_max.max(ch.snow_fade0);
        s.snow_fade1_max = s.snow_fade1_max.max(ch.snow_fade1);
    }
    Ok(s)
}

/// Residual of the deterministic floor against a captured `SnowMudData`.
///
/// The floor is a LOWER BOUND that the engine may carry in EITHER fade
/// endpoint (in the analysed captures the small floor-like snow sits in the
/// fade-1 endpoint `.g` while fade-0 `.b` is zero). So the physically
/// meaningful check is: at every texel where the deterministic floor is `f`,
/// does the capture carry `>= f` snow in `max(.b, .g)` (within tolerance)?
/// `floor_violations` counts texels where BOTH endpoints are below the floor
/// (which would refute the floor law). We also report exact byte matches
/// against the currently-displayed snow (at the capture's `SnowMudFade`) for
/// completeness, knowing they will be low because the above-floor field is
/// weather-sim state.
#[derive(Clone, Debug, PartialEq)]
pub struct SnowFloorResidual {
    pub total_texels: usize,
    /// Texels where the deterministic floor is > 0.
    pub floored_texels: usize,
    /// Of the floored texels, how many carry >= floor in max(.b,.g).
    pub floor_respected: usize,
    /// Of the floored texels, how many are BELOW the floor in BOTH endpoints.
    pub floor_violations: usize,
    /// Worst (floor - captured max-endpoint) shortfall in `[0,1]`.
    pub max_floor_shortfall: f32,
    /// Exact `snow` byte matches (floor byte == captured displayed byte at the
    /// capture's fade).
    pub exact_snow_byte_matches: usize,
    /// Mean absolute difference between floor and captured displayed snow over
    /// ALL texels, in `[0,1]`.
    pub mean_abs_snow_diff: f32,
    /// The `SnowMudFade` used to compute captured displayed snow.
    pub fade: f32,
}

impl SnowFloorResidual {
    pub fn exact_match_percent(&self) -> f32 {
        if self.total_texels == 0 {
            0.0
        } else {
            100.0 * self.exact_snow_byte_matches as f32 / self.total_texels as f32
        }
    }
    pub fn floor_respected_percent(&self) -> f32 {
        if self.floored_texels == 0 {
            100.0
        } else {
            100.0 * self.floor_respected as f32 / self.floored_texels as f32
        }
    }
}

/// Compare a synthesized [`SnowFloorTexture`] against a captured `SnowMudData`
/// payload at a given `SnowMudFade`.
pub fn compare_floor_to_capture(
    floor: &SnowFloorTexture,
    payload: &[u8],
    fade: f32,
) -> Result<SnowFloorResidual> {
    let total = floor.width * floor.height;
    if payload.len() < total * 4 {
        return Err(format!(
            "compare: payload {} shorter than {}x{}x4",
            payload.len(),
            floor.width,
            floor.height
        )
        .into());
    }
    let mut r = SnowFloorResidual {
        total_texels: total,
        floored_texels: 0,
        floor_respected: 0,
        floor_violations: 0,
        max_floor_shortfall: 0.0,
        exact_snow_byte_matches: 0,
        mean_abs_snow_diff: 0.0,
        fade,
    };
    let tol = 1.0 / 255.0; // one UNORM step
    let mut sum_abs = 0.0f64;
    for i in 0..total {
        let ch = decode_snow_mud_texel(payload, i).ok_or("compare: short texel")?;
        let captured_snow = ch.displayed_snow(fade);
        // The floor may be carried in either endpoint; the lower-bound check
        // uses the larger of the two so a floor stashed in the fade target is
        // still counted as respected.
        let endpoint_max = f32::from(ch.snow_fade0.max(ch.snow_fade1)) / 255.0;
        let floor_val = floor.floor[i];
        let floor_byte = (floor_val.clamp(0.0, 1.0) * 255.0).round() as u8;
        // Displayed byte at this fade for the capture.
        let captured_byte = (captured_snow.clamp(0.0, 1.0) * 255.0).round() as u8;
        if floor_byte == captured_byte {
            r.exact_snow_byte_matches += 1;
        }
        sum_abs += (captured_snow - floor_val).abs() as f64;
        if floor_val > 0.0 {
            r.floored_texels += 1;
            if endpoint_max + tol >= floor_val {
                r.floor_respected += 1;
            } else {
                r.floor_violations += 1;
                r.max_floor_shortfall = r.max_floor_shortfall.max(floor_val - endpoint_max);
            }
        }
    }
    r.mean_abs_snow_diff = if total == 0 {
        0.0
    } else {
        (sum_abs / total as f64) as f32
    };
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION_SRC: &str = r#"
strategic_region={
	id=42
	name="STRATEGICREGION_42"
	provinces={
		100 101 102
	}
	weather={
		period={
			between={ 0.0 30.0 }
			temperature={ -40.0 -12.0 }
			snow=0.350
			blizzard=0.300
			mud=0.500
			min_snow_level=0.300
		}
		period={
			between={ 0.1 27.1 }
			temperature={ -7.0 12.0 }
			snow=0.200
			blizzard=0.050
			mud=0.300
			min_snow_level=0.100
		}
		period={
			between={ 0.10 29.10 }
			temperature={ -1.0 15.0 }
			snow=0.210
			blizzard=0.000
			mud=0.300
			min_snow_level=0.050
		}
		period={
			between={ 0.11 30.11 }
			temperature={ -5.0 13.0 }
			snow=0.200
			blizzard=0.000
			mud=0.300
			min_snow_level=0.200
		}
	}
}
"#;

    fn parsed_region() -> RegionWeather {
        parse_region_weather(REGION_SRC)
            .expect("parse ok")
            .expect("has id")
    }

    #[test]
    fn parses_id_name_provinces_and_periods() {
        let r = parsed_region();
        assert_eq!(r.region_id, 42);
        assert_eq!(r.name, "STRATEGICREGION_42");
        assert_eq!(r.provinces, vec![100, 101, 102]);
        assert_eq!(r.periods.len(), 4);
        assert_eq!(r.periods[0].min_snow_level, 0.3);
        assert_eq!(r.periods[0].temperature, [-40.0, -12.0]);
        assert_eq!(r.periods[0].snow_probability, 0.35);
    }

    #[test]
    fn between_day_month_zero_based_parse() {
        // "0.0" = day0 month0 (Jan 1 nominal); "30.11" = day30 month11 (Dec 31).
        let p0 = WeatherPoint::parse("0.0").unwrap();
        assert_eq!(p0, WeatherPoint { day: 0, month0: 0 });
        let p1 = WeatherPoint::parse("29.10").unwrap();
        assert_eq!(
            p1,
            WeatherPoint {
                day: 29,
                month0: 10
            }
        );
        let p2 = WeatherPoint::parse("30.11").unwrap();
        assert_eq!(
            p2,
            WeatherPoint {
                day: 30,
                month0: 11
            }
        );
    }

    #[test]
    fn period_matching_selects_month_by_date() {
        let r = parsed_region();
        // Jan 5 -> period 0 (min_snow 0.3).
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 5).unwrap()),
            Some(0.3)
        );
        // Feb 10 -> period 1 (min_snow 0.1).
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(2, 10).unwrap()),
            Some(0.1)
        );
        // Nov 15 -> period between {0.10 29.10} (min_snow 0.05).
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(11, 15).unwrap()),
            Some(0.05)
        );
        // Dec 20 -> period {0.11 30.11} (min_snow 0.2).
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(12, 20).unwrap()),
            Some(0.2)
        );
        // March has no period here -> fail-closed None.
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(3, 15).unwrap()),
            None
        );
    }

    #[test]
    fn period_matching_handles_year_wrap_window() {
        // Build a region whose only period wraps the year: Dec 15 .. Jan 15.
        let src = r#"
strategic_region={
	id=7
	provinces={ 1 }
	weather={
		period={
			between={ 14.11 14.0 }
			min_snow_level=0.5
		}
	}
}
"#;
        let r = parse_region_weather(src).unwrap().unwrap();
        // Dec 20 inside wrap.
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(12, 20).unwrap()),
            Some(0.5)
        );
        // Jan 10 inside wrap.
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 10).unwrap()),
            Some(0.5)
        );
        // June outside.
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(6, 1).unwrap()),
            None
        );
    }

    #[test]
    fn no_weather_block_yields_empty_periods_not_error() {
        let src = "strategic_region={ id=9 provinces={ 5 } }";
        let r = parse_region_weather(src).unwrap().unwrap();
        assert!(r.periods.is_empty());
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 1).unwrap()),
            None
        );
    }

    #[test]
    fn malformed_between_fails_closed() {
        let src = r#"
strategic_region={ id=3 provinces={ 1 }
	weather={ period={ between={ 0.0 } min_snow_level=0.1 } } }
"#;
        let err = parse_region_weather(src).expect_err("one-token between fails");
        assert!(err.to_string().contains("between"), "{err}");
    }

    #[test]
    fn min_snow_out_of_range_fails_closed() {
        let src = r#"
strategic_region={ id=3 provinces={ 1 }
	weather={ period={ between={ 0.0 30.0 } min_snow_level=1.5 } } }
"#;
        let err = parse_region_weather(src).expect_err("min_snow 1.5 fails");
        assert!(err.to_string().contains("out of [0,1]"), "{err}");
    }

    #[test]
    fn channel_decode_uses_bgra_byte_order() {
        // px = [B=snow_fade0, G=snow_fade1, R=mud_fade0, A=mud_fade1]
        let payload = [10u8, 20, 30, 40];
        let ch = decode_snow_mud_texel(&payload, 0).unwrap();
        assert_eq!(ch.snow_fade0, 10);
        assert_eq!(ch.snow_fade1, 20);
        assert_eq!(ch.mud_fade0, 30);
        assert_eq!(ch.mud_fade1, 40);
        // round trip
        assert_eq!(encode_snow_mud_texel(ch), payload);
    }

    #[test]
    fn displayed_snow_lerps_fade0_to_fade1_by_fade() {
        let ch = SnowMudChannels {
            snow_fade0: 0,
            snow_fade1: 255,
            mud_fade0: 0,
            mud_fade1: 0,
        };
        assert_eq!(ch.displayed_snow(0.0), 0.0);
        assert_eq!(ch.displayed_snow(1.0), 1.0);
        assert!((ch.displayed_snow(0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn decode_and_fade_fail_closed_on_short_buffers() {
        assert!(decode_snow_mud_texel(&[1, 2, 3], 0).is_none());
        assert!(snow_mud_fade_from_cb0(&[0u8; CB0_OFFSET_SNOW_MUD_FADE]).is_none());
        assert!(snow_mud_fade_from_cb0(&[0u8; CB0_OFFSET_SNOW_MUD_FADE + 4]).is_some());
    }

    #[test]
    fn cb0_snow_mud_fade_reads_correct_offset() {
        let mut cb0 = vec![0u8; 416];
        cb0[CB0_OFFSET_SNOW_MUD_FADE..CB0_OFFSET_SNOW_MUD_FADE + 4]
            .copy_from_slice(&0.75f32.to_le_bytes());
        assert_eq!(snow_mud_fade_from_cb0(&cb0), Some(0.75));
    }

    #[test]
    fn addressing_maps_world_to_quarter_res_texel() {
        let a = SnowMudAddressing::default();
        // World origin maps near texel (0,0).
        assert_eq!(a.world_to_texel(0.0, 0.0), Some((0, 0)));
        // Center of the map maps near the texture center.
        let (tx, ty) = a
            .world_to_texel(VANILLA_MAP_SIZE_X / 2.0, VANILLA_MAP_SIZE_Y / 2.0)
            .unwrap();
        assert!(
            (tx as i32 - (SNOW_MUD_TEXTURE_WIDTH as i32 / 2)).abs() <= 1,
            "tx={tx}"
        );
        assert!(
            (ty as i32 - (SNOW_MUD_TEXTURE_HEIGHT as i32 / 2)).abs() <= 1,
            "ty={ty}"
        );
        // Far corner maps to the last texel.
        let (tx, ty) = a
            .world_to_texel(VANILLA_MAP_SIZE_X - 1.0, VANILLA_MAP_SIZE_Y - 1.0)
            .unwrap();
        assert_eq!(
            (tx, ty),
            (SNOW_MUD_TEXTURE_WIDTH - 1, SNOW_MUD_TEXTURE_HEIGHT - 1)
        );
    }

    #[test]
    fn addressing_wraps_out_of_range_like_wrap_sampler() {
        let a = SnowMudAddressing::default();
        // x = MAP_SIZE_X wraps to 0.
        assert_eq!(
            a.world_to_texel(VANILLA_MAP_SIZE_X, 0.0).map(|t| t.0),
            Some(0)
        );
        // Negative wraps to the far side.
        let neg = a.world_to_texel(-1.0, 0.0).unwrap().0;
        assert_eq!(neg, SNOW_MUD_TEXTURE_WIDTH - 1);
    }

    /// Build a tiny synthetic map: 8x8 pixels, one region covering the left
    /// half (provinces 100/101/102) with a Jan floor of 0.3, right half
    /// uncovered.
    fn tiny_floor_inputs() -> (BTreeMap<u32, RegionWeather>, BTreeMap<u32, u32>, Vec<u32>) {
        let mut rw = BTreeMap::new();
        rw.insert(42, parsed_region());
        let mut p2r = BTreeMap::new();
        for pid in [100u32, 101, 102] {
            p2r.insert(pid, 42);
        }
        // 8x8: left 4 columns = province 100, right 4 = province 999 (no region).
        let mut ids = vec![999u32; 64];
        for y in 0..8 {
            for x in 0..4 {
                ids[y * 8 + x] = 100;
            }
        }
        (rw, p2r, ids)
    }

    #[test]
    fn floor_synthesis_covers_region_texels_only() {
        let (rw, p2r, ids) = tiny_floor_inputs();
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        let inputs = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        // Jan -> floor 0.3 on the covered left half.
        let tex = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &inputs).unwrap();
        assert_eq!(tex.width, 4);
        assert_eq!(tex.height, 4);
        // Left two texel columns covered (map pixels 0..4 -> texels 0..2).
        assert_eq!(tex.at(0, 0), 0.3);
        assert_eq!(tex.at(1, 0), 0.3);
        // Right two texel columns uncovered.
        assert_eq!(tex.at(2, 0), 0.0);
        assert_eq!(tex.at(3, 0), 0.0);
        assert_eq!(tex.covered_texels, 4 * 2); // 4 rows x 2 covered cols
    }

    #[test]
    fn floor_synthesis_fails_closed_when_no_period_matches_date() {
        let (rw, p2r, ids) = tiny_floor_inputs();
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        let inputs = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        // March has no period in the parsed region -> region skipped, zero floor.
        let tex = synthesize_snow_floor_texture(SnowDate::new(3, 15).unwrap(), &inputs).unwrap();
        assert_eq!(tex.covered_texels, 0);
        assert_eq!(tex.regions_without_period, 1);
        assert!(tex.floor.iter().all(|&f| f == 0.0));
    }

    #[test]
    fn floor_synthesis_flip_source_y_toggle_controls_orientation() {
        // Mechanism test for the flip toggle. Production uses
        // `flip_source_y = true`: per `CalcGlobeNormal` world z=0 is SOUTH, so
        // SnowMud texture row 0 = south while `load_provinces_bmp` row 0 =
        // north, requiring the flip. This test proves the toggle does what it
        // says in both directions.
        // 8x8 raster: only the TOP 4 rows (source north) carry province 100.
        let mut rw = BTreeMap::new();
        rw.insert(42, parsed_region());
        let mut p2r = BTreeMap::new();
        p2r.insert(100u32, 42);
        let mut ids = vec![999u32; 64];
        for y in 0..4 {
            for x in 0..8 {
                ids[y * 8 + x] = 100;
            }
        }
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        // With the flip, texture row 0 = BMP bottom (south) = uncovered, and the
        // covered top BMP rows land in the BOTTOM texture rows.
        let flipped = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: true,
        };
        let tex = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &flipped).unwrap();
        assert_eq!(
            tex.at(0, 0),
            0.0,
            "flipped: top texture row is uncovered south"
        );
        assert_eq!(
            tex.at(0, 3),
            0.3,
            "flipped: bottom texture row carries the north floor"
        );

        // Without the flip, the covered top rows stay in the top texture rows.
        let unflipped = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        let tex2 = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &unflipped).unwrap();
        assert_eq!(
            tex2.at(0, 0),
            0.3,
            "unflipped: top texture row carries the north floor"
        );
        assert_eq!(
            tex2.at(0, 3),
            0.0,
            "unflipped: bottom texture row is uncovered south"
        );
    }

    #[test]
    fn floor_payload_writes_snow_into_b_and_g_only() {
        let tex = SnowFloorTexture {
            width: 2,
            height: 1,
            floor: vec![0.2, 0.0],
            covered_texels: 1,
            regions_without_period: 0,
        };
        let payload = tex.to_snow_mud_payload();
        // texel 0: floor 0.2 -> byte 51 in b(.b=byte0) and g(.g=byte1); r/a zero.
        let byte = (0.2f32 * 255.0).round() as u8;
        assert_eq!(payload[0], byte); // .b snow_prev
        assert_eq!(payload[1], byte); // .g snow_curr
        assert_eq!(payload[2], 0); // .r mud_prev
        assert_eq!(payload[3], 0); // .a mud_curr
                                   // texel 1: zero floor -> all zero.
        assert_eq!(&payload[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn floor_respected_against_a_capture_that_meets_the_floor() {
        // floor 0.3 in one texel; capture displays 0.5 there (fade 0 => .b).
        let floor = SnowFloorTexture {
            width: 1,
            height: 1,
            floor: vec![0.3],
            covered_texels: 1,
            regions_without_period: 0,
        };
        // capture .b = 128 (~0.502), fade 0 => displayed = .b.
        let payload = [128u8, 0, 0, 0];
        let r = compare_floor_to_capture(&floor, &payload, 0.0).unwrap();
        assert_eq!(r.floored_texels, 1);
        assert_eq!(r.floor_respected, 1);
        assert_eq!(r.floor_violations, 0);
        assert_eq!(r.floor_respected_percent(), 100.0);
    }

    #[test]
    fn floor_violation_detected_when_capture_below_floor() {
        let floor = SnowFloorTexture {
            width: 1,
            height: 1,
            floor: vec![0.3],
            covered_texels: 1,
            regions_without_period: 0,
        };
        // capture .b = 0 (no snow), fade 0 => displayed 0 < floor 0.3.
        let payload = [0u8, 0, 0, 0];
        let r = compare_floor_to_capture(&floor, &payload, 0.0).unwrap();
        assert_eq!(r.floor_violations, 1);
        assert!((r.max_floor_shortfall - 0.3).abs() < 1e-3);
    }

    #[test]
    fn captured_stats_decode_channel_maxes() {
        // 2 texels: [b,g,r,a]
        let payload = [10u8, 20, 30, 40, 0, 0, 0, 0];
        let s = captured_snow_mud_stats(&payload, 2, 1).unwrap();
        assert_eq!(s.total_texels, 2);
        assert_eq!(s.snow_fade0_nonzero, 1);
        assert_eq!(s.snow_fade1_max, 20);
        assert_eq!(s.mud_fade1_max, 40);
    }
}
