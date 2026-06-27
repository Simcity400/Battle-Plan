//! `dynamic` — the TIME-VARYING map machinery, parameterised so the scrubber drives the
//! look hour-by-hour exactly like the real game. This module is the CONTRACT between the
//! render side (which ports the game's dynamic shader branches) and the data side (which a
//! capture agent fills from memory/save). It owns NO data acquisition — only the
//! interface + the faithful game math that turns those inputs into the dynamic visuals.
//!
//! ## The dynamic branches the game's map shaders contain (all ported here / in map.wgsl)
//! Enumerated + Codex-verified from `pdxmap.shader`, `pdxwater.shader`, `standardfuncsgfx.fxh`,
//! `seasons.txt`, `common/weather.txt`:
//!
//!   * **Snow / winter** — `ApplySnow` [standardfuncsgfx.fxh:264-301]. Driven by per-province
//!     winter level (`SnowMudData.g`, crossfaded from `.b` summer by the global `SnowMudFade`
//!     scalar), gated by world height (`SNOW_START_HEIGHT`) + upward normal (`SNOW_NORMAL_START`)
//!     + camera opacity ramp. Blends `SNOW_COLOR` over terrain.
//!   * **Mud** — `GetMudColor` [standardfuncsgfx.fxh:237-257]. Per-province wet level
//!     (`SnowMudData.a`, crossfaded from `.r` dry by `SnowMudFade`). Darkens/tints terrain.
//!   * **Ice / frozen sea** — `ApplyIce` [pdxwater.shader:231-256]. Sea freezes where winter
//!     level high AND far-north (map-Y > 0.74); lerps toward `ICE_COLOR`.
//!   * **Seasonal colormap** — `seasons.txt` `colorbalance`/`hsv` per latitude band, blended by
//!     DATE between the 4 season blocks. A per-channel multiply + HSV shift over the colormap.
//!   * **Day / night environment** — `DayNightWithBlend` [pdxmap.shader:395]. The sun-direction
//!     vector + hour phase (`DayNight_Hour_SunDir`) rotate the day/night terminator; the whole
//!     map tone shifts via `NightifyColor` × `NIGHT_OPACITY`.
//!   * **City lights** — night-driven additive [pdxmap.shader:384-385], gated by `nightFactor`.
//!   * **War-state** — control recolour (border channels), occupation stripes (secondary mask),
//!     contested/dominance pulse. Driven by per-province control TAG + occupation flag.
//!   * **Weather particles** — a SEPARATE entity/particle system (`particle.shader`,
//!     `weatherpositions.txt`), NOT a terrain-shader branch; in THIS mod
//!     `WEATHER_DISTANCE_CUTOFF=50` so particles draw only at extreme close zoom. The weather's
//!     EFFECT on the map (snow/mud accumulation) flows through `SnowMudData` = the winter/mud
//!     inputs above, which the data side fills from the weather state. We expose the per-region
//!     weather TYPE so a future particle layer can use it; the map TONE effect is the snow/mud.
//!   * **Built objects** (forts/rail/buildings/ports/hubs) — a separate MESH layer
//!     (`pdxmesh.shader`), and in this mod `DRAW_MAP_OBJECTS_CUTOFF=1.0` culls them at map view.
//!     Exposed as a `built_objects` set for a future mesh layer; not a terrain-shader branch.
//!
//! ## Mod-confirmed render flags (msb_defines.lua)
//! `DRAW_FOW_CUTOFF=0`, `DRAW_SHADOWS_CUTOFF=0`, `DRAW_REFRACTIONS_CUTOFF=0` → fog-of-war,
//! terrain shadows and water refraction are NOT drawn at the political/strategic map view in
//! this mod. Omitting them is therefore FAITHFUL to this mod, not a gap.

//! ## THE INPUT-INTERFACE SPEC (the data-side contract — required deliverable)
//!
//! Every time-varying input the map needs, and where it lives in this module. A capture agent
//! fills these from memory/save each scrubber tick; the renderer consumes them. Inputs marked
//! (LAYER) belong to a separate render layer (rivers/arrows/meshes) the renderer adds later —
//! the contract names them so the data side captures them now.
//!
//! PER-PROVINCE (→ [`ProvinceDynamic`], packed into the per-province dynamic LUT):
//!   * `winter_level` 0..1 — snow cover (ApplySnow). The data side resolves it from
//!     SnowMudData.g × the height/normal/season gates (we apply the camera-opacity ramp in
//!     shader). Drives snow tint.
//!   * `mud_level` 0..1 — wet/mud (GetMudColor). From SnowMudData.a.
//!   * `frozen` — sea/lake ice (ApplyIce). Set where winter high AND far-north sea.
//!   * `weather` — the phenomenon ([`WeatherType`]) for a future particle layer + as the
//!     SOURCE the data side accumulates winter/mud from (via common/weather.txt).
//!   * control TAG / occupation flag — ALREADY carried by the political LUT + the
//!     secondary-colour LUT ([`crate::map::build_secondary_lut`]); drive border recolour +
//!     occupation stripes. (These change with war state every tick.)
//!
//! GLOBAL (→ [`DynamicInputs`] → [`DynamicUniforms`]):
//!   * `date` (y,m,d,h) — drives the season balance + the day/night hour phase + sun.
//!   * `snow_mud_fade` — the SnowMud old↔current interp factor (default 1.0; the data side
//!     may set <1 to cross-fade two captured ticks for smooth scrubbing).
//!   * `global_time` — the `vGlobalTime` animation clock (border/dominance pulse, water/river
//!     flow, arrow blink, trade-route scroll). Frozen for a paused scrubber; advances live.
//!   * `naval_dominance` (per sea-region control/contested) — the `dominance_fx_apply` pulse
//!     inputs (LAYER: a future water-dominance pass; captured now in the contract).
//!
//! SEPARATE LAYERS the contract names (renderer adds these passes later; data captured now):
//!   * gradient-border control state per province → the animated country/state border ramp
//!     (we approximate with an owner-edge scan today; the gradient distance-field is a future
//!     pass built from provinces.bmp + control).
//!   * `built_objects` set (forts/rail/buildings/ports/hubs) → a mesh/decal layer
//!     ([`BuiltObject`]); mod culls them at map view (`DRAW_MAP_OBJECTS_CUTOFF=1.0`).
//!   * map labels / battle-plan arrows / trade routes → separate text/arrow layers.
//!
//! MOD-DISABLED (faithfully omitted for THIS mod — proven, not assumed): fog-of-war, terrain
//! shadows, water refraction (`DRAW_*_CUTOFF=0`); weather particles except at extreme zoom
//! (`WEATHER_DISTANCE_CUTOFF=50`).

use crate::terrain::gc;

/// A weather phenomenon a strategic region can have (from `strategicregions/*.txt` +
/// `common/weather.txt`). Used by the data side to set per-province winter/mud and (later) to
/// drive a particle layer. The map-TONE effect is the winter/mud levels these accumulate into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherType {
    Clear,
    RainLight,
    RainHeavy,
    Snow,
    Blizzard,
    Sandstorm,
    ArcticWater,
}

impl Default for WeatherType {
    fn default() -> Self {
        WeatherType::Clear
    }
}

/// A built map object that appears as construction completes (data-side fills the set per
/// tick). The mesh/decal LAYER is future work (mod culls objects at map view); this is the
/// interface so the data side can capture them now.
#[derive(Debug, Clone)]
pub struct BuiltObject {
    pub province_id: u32,
    pub kind: String, // "fort" | "railway" | "building:<type>" | "port" | "supply_hub"
    pub level: u32,
    /// world position (map pixel space), if known.
    pub pos: [f32; 2],
}

/// PER-PROVINCE time-varying inputs — one entry per province id (index = province id). The
/// data side fills these from captured state each scrubber tick; the render side turns them
/// into the dynamic visuals. All are 0..=255 / flags so they pack into per-province LUT
/// textures (like the owner index), uploaded cheaply per tick.
#[derive(Debug, Clone, Default)]
pub struct ProvinceDynamic {
    /// CURRENT winter level 0..1 (→ snow) = the game's `SnowMudData.g`. The shader crossfades
    /// `lerp(winter_level_old, winter_level, snow_mud_fade)`. [standardfuncsgfx.fxh:259-262]
    pub winter_level: f32,
    /// OLD (previous-tick / summer) winter level = `SnowMudData.b`. Defaults equal to
    /// `winter_level` (no cross-fade) unless the data side sets a previous value for smooth
    /// scrubbing between captured ticks. (Codex #3: faithful fade is over old↔current, not 0.)
    pub winter_level_old: f32,
    /// CURRENT mud wet level 0..1 (→ mud tint) = `SnowMudData.a`.
    pub mud_level: f32,
    /// OLD mud level = `SnowMudData.r`. Defaults equal to `mud_level`.
    pub mud_level_old: f32,
    /// frozen sea/lake flag (→ ice tint). Set where winter high AND a far-north sea.
    pub frozen: bool,
    /// NAVAL DOMINANCE / contested control for a SEA province: −1 (enemy-dominated) .. 0
    /// (uncontested) .. +1 (friendly-dominated). Drives the `dominance_fx_apply` pulsing
    /// overlay on contested seas. The data side resolves it from naval/war state per sea
    /// province (from the per-region `naval_dominance` map). 0 = no dominance overlay.
    pub dominance: f32,
    /// per-province weather (for a future particle layer + as the source of winter/mud).
    pub weather: WeatherType,
}

impl ProvinceDynamic {
    /// A STEADY province (no cross-fade): old == current, so `lerp(old, current, fade)` is the
    /// same value at any fade. This is the right constructor when a single tick is captured
    /// (no previous-tick value to interpolate from). [`Default`] is `steady(0,0,false)`.
    pub fn steady(winter: f32, mud: f32, frozen: bool, weather: WeatherType) -> ProvinceDynamic {
        ProvinceDynamic {
            winter_level: winter,
            winter_level_old: winter,
            mud_level: mud,
            mud_level_old: mud,
            frozen,
            dominance: 0.0,
            weather,
        }
    }
}

/// The full set of time-varying inputs for ONE scrubber tick. This is the REQUIRED
/// data-side CONTRACT: a capture agent fills it from memory/save, the renderer consumes it.
#[derive(Debug, Clone)]
pub struct DynamicInputs {
    /// in-game date components (year, month 1..12, day 1..31, hour 0..23).
    pub date: (i32, u32, u32, u32),
    /// per-province dynamic state, index = province id (len = max_id+1). Missing provinces
    /// default (no snow/mud/ice).
    pub provinces: Vec<ProvinceDynamic>,
    /// the engine's `SnowMudFade` (`vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z`). Codex
    /// CORRECTION: this is the TEXTURE-INTERP factor between the SnowMudData OLD and CURRENT
    /// channels (snow = lerp(`.b` old/summer, `.g` current/winter, fade); mud = lerp(`.r`,
    /// `.a`, fade)) — NOT a calendar winter fraction. Because we upload per-province
    /// winter_level/mud_level as the ALREADY-RESOLVED CURRENT values each scrubber tick, the
    /// faithful default is `1.0` (= "use the current value fully"). The data side may still
    /// set it < 1 to cross-fade between two captured ticks (smooth scrubbing).
    pub snow_mud_fade: f32,
    /// per-sea-region NAVAL DOMINANCE / contested state (the `dominance_fx_apply` inputs):
    /// region id → (control −1..1, contested −1..1). Drives the contested-sea pulse. The
    /// data side fills it from naval/war state; the renderer (future water layer) pulses it.
    pub naval_dominance: std::collections::HashMap<u32, (f32, f32)>,
    /// continuous-animation PHASE inputs (the `vGlobalTime`-driven cosmetics): for a paused
    /// scrubber these freeze at `global_time`; for live playback they advance. Border/dominance
    /// pulse, water/river flow, arrow blink, trade-route scroll all read `global_time`.
    pub animate: bool,
    /// continuous animation clock (seconds) — `vGlobalTime` (border/dominance pulse, water
    /// flow, arrow blink). The scrubber advances it; visuals that pulse read it.
    pub global_time: f32,
    /// built objects constructed by this tick (future mesh layer).
    pub built_objects: Vec<BuiltObject>,
}

impl DynamicInputs {
    /// A neutral tick: summer, noon, no snow/mud/ice — the static-map equivalent. Useful as a
    /// base the data side overrides field-by-field, and as the default for a t=0 render.
    pub fn neutral(num_provinces: usize, date: (i32, u32, u32, u32)) -> DynamicInputs {
        DynamicInputs {
            date,
            provinces: vec![ProvinceDynamic::default(); num_provinces],
            // 1.0 = "use the per-province CURRENT winter/mud fully" (Codex: fade is the
            // old↔current texture-interp factor, not a calendar fraction).
            snow_mud_fade: 1.0,
            global_time: 0.0,
            built_objects: Vec::new(),
            naval_dominance: std::collections::HashMap::new(),
            animate: false,
        }
    }

    /// A date-driven WINTER-INTENSITY estimate (0 deep-summer .. 1 deep-winter) the data side
    /// MAY use to seed per-province `winter_level` when it has no direct SnowMudData measurement
    /// (a fallback only). NOTE (Codex): this is NOT `snow_mud_fade` — that field is the engine's
    /// old↔current texture-interp factor (default 1.0). This helper estimates how snowy the
    /// SEASON is, which the data side then modulates per province by latitude/height.
    pub fn season_winter_estimate(&self) -> f32 {
        season_winter_fraction(self.date.1, self.date.2)
    }

    /// Build the per-tick dynamic inputs from the snapshot WEATHER domain (the now-recorded
    /// `weather.provinces.*` / `weather.regions.*` reader fold), starting from the neutral
    /// base for `num_provinces` at `date`. This is the PER-PROVINCE-DIRECT path (snow + mud);
    /// the per-region FROZEN-SEAS join lives in [`DynamicInputs::from_weather_snapshot_full`],
    /// which calls this then overlays `frozen` using a province→region map.
    ///
    /// NO-FABRICATION (Codex review): the per-province weather data this consumes are the
    /// directly-MEASURED values:
    ///   * `weather.provinces.<pid>.mud` (a JSON bool) → `mud_level` (true→1.0, false→0.0), and
    ///   * `weather.provinces.<pid>.snow` (a JSON number, the reader's save snow amount, i32/1e5)
    ///     → `winter_level` = `snow as f32` clamped to the shader's 0..1 range. The value is fed
    ///     DIRECTLY (no second divide, no season/latitude derivation) — a measured snow amount
    ///     of `0.0` is a real "no snow here" reading and still counts as measured.
    /// It does NOT derive `winter_level`/`frozen`/`WeatherType` from the measured per-province
    /// `temp_c` or the per-region `phenomenon` (temperature is not snow coverage; the region
    /// phenomenon needs the province→region join handled separately). A province with NO
    /// measured `mud` AND no measured `snow` stays NEUTRAL (no snow/mud/ice) — never a
    /// fabricated `false`/`0`-as-data; it is simply "no weather datum".
    ///
    /// Province ids are 1-based and used as the LUT index directly (no compaction); an id ≥
    /// `num_provinces` or an unparseable key is SKIPPED (never coerced to province 0).
    ///
    /// Returns `(inputs, measured_any)`: `measured_any` is true iff at least one province
    /// carried a measured `mud` OR `snow` value, so the caller can GATE the dynamic upload —
    /// when no weather is captured it should keep the neutral/static path (preserving the
    /// byte-identical no-weather map), rather than upload an all-neutral dynamic LUT that
    /// differs from "off". (Codex (a): clamping a measured snow value outside 0..1 is a render
    /// SATURATION, not new data; such out-of-range values are surfaced via the returned count
    /// so they are not silently turned into plausible visuals.)
    pub fn from_weather_snapshot(
        num_provinces: usize,
        date: (i32, u32, u32, u32),
        weather: Option<&serde_json::Value>,
    ) -> (DynamicInputs, bool) {
        let (inp, measured_any, _) =
            Self::from_weather_snapshot_counted(num_provinces, date, weather);
        (inp, measured_any)
    }

    /// As [`from_weather_snapshot`] but also returns the count of MEASURED snow values whose
    /// magnitude fell OUTSIDE the shader's 0..1 range (so they were clamped). This is a
    /// data-honesty signal (Codex (a)): clamping turns an out-of-band value into a plausible
    /// visual, so the caller can log/assert it rather than have it pass silently.
    pub fn from_weather_snapshot_counted(
        num_provinces: usize,
        date: (i32, u32, u32, u32),
        weather: Option<&serde_json::Value>,
    ) -> (DynamicInputs, bool, usize) {
        let mut inp = DynamicInputs::neutral(num_provinces, date);
        let mut measured_any = false;
        let mut snow_out_of_range = 0usize;

        // the per-province weather map: `weather.provinces` → { "<pid>": { "mud": bool,
        // "snow": number, ... } }.
        let provinces = weather
            .and_then(|w| w.get("provinces"))
            .and_then(serde_json::Value::as_object);
        if let Some(provs) = provinces {
            for (pid_key, pv) in provs {
                // The province id keys the per-province LUT cell DIRECTLY (the LUT is the full
                // contiguous `0..=max_id` range). An unparseable key, or an id ≥ num_provinces,
                // is SKIPPED (never coerced/clamped to another cell).
                //
                // Index 0: this preserves the module's ESTABLISHED, tested policy (see
                // `from_weather_snapshot_skips_bad_province_ids`): id 0 maps to LUT index 0 (no
                // off-by-one), the same for snow/mud/frozen as it already was for mud. Province 0
                // is the game's sentinel slot; the live reader only ever emits a `"0"` entry as part
                // of the FULL per-province array (alongside the real provinces 1..N that drive
                // gating), so this id-0 cell never independently fabricates a dynamic upload, and
                // changing the policy here would diverge snow/frozen from the mud contract.
                let pid: usize = match pid_key.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if pid >= num_provinces {
                    continue;
                }
                // MUD: a present bool (true OR false) is a real measurement → mud_level 1.0 /
                // 0.0 (steady old==current so the crossfade is identity). Absent/null/non-bool
                // → unknown, stays neutral (no fabricated false).
                if let Some(serde_json::Value::Bool(is_mud)) = pv.get("mud") {
                    measured_any = true;
                    let lvl = if *is_mud { 1.0 } else { 0.0 };
                    inp.provinces[pid].mud_level = lvl;
                    inp.provinces[pid].mud_level_old = lvl;
                }
                // SNOW: a present NUMBER is a real measurement (the reader's already-/1e5 save
                // snow amount) → winter_level = value, clamped to 0..1 for the shader. A
                // measured 0.0 counts as measured (real "no snow"). Absent/null/non-number →
                // unknown, winter stays neutral 0 (no fabricated 0-as-data).
                if let Some(snow_v) = pv.get("snow").and_then(serde_json::Value::as_f64) {
                    measured_any = true;
                    if snow_v < 0.0 || snow_v > 1.0 {
                        snow_out_of_range += 1;
                    }
                    let lvl = (snow_v as f32).clamp(0.0, 1.0);
                    inp.provinces[pid].winter_level = lvl;
                    inp.provinces[pid].winter_level_old = lvl;
                }
            }
        }
        (inp, measured_any, snow_out_of_range)
    }

    /// The FULL per-tick dynamic feed: the per-province-direct snow/mud
    /// ([`from_weather_snapshot`]) PLUS the per-REGION FROZEN-SEAS overlay. `frozen_seas` is
    /// the set of SEA province ids resolved by the strategic-region join (every sea province
    /// of a region whose `weather.regions.<rid>.phenomenon` contained `arctic_water`); see
    /// [`crate::strategic_regions::StrategicRegions::frozen_sea_provinces`]. Setting `frozen`
    /// from a MEASURED arctic_water region is a real datum → it also flips `measured_any` so a
    /// snapshot whose only weather signal is a frozen sea still drives the dynamic layer.
    ///
    /// NO-FABRICATION: `frozen_seas` is built sea-only from MEASURED region phenomenon; a
    /// province not in that set keeps its (snow/mud) state and is never fabricated frozen.
    pub fn from_weather_snapshot_full(
        num_provinces: usize,
        date: (i32, u32, u32, u32),
        weather: Option<&serde_json::Value>,
        frozen_seas: &std::collections::HashSet<u32>,
    ) -> (DynamicInputs, bool, usize) {
        let (mut inp, mut measured_any, snow_oor) =
            Self::from_weather_snapshot_counted(num_provinces, date, weather);
        // `frozen_seas` is built SEA-ONLY from a MEASURED `arctic_water` region (the join filters
        // to `definition.csv` type==sea), so province 0 (the `land` sentinel) is never present
        // and a non-empty set always reflects a real arctic-water measurement. An id ≥
        // num_provinces is skipped (never writes past the LUT). Each frozen sea is a real datum →
        // it flips `measured_any`, so a frozen-only tick still drives the dynamic layer.
        for &pid in frozen_seas {
            let idx = pid as usize;
            if idx < num_provinces {
                inp.provinces[idx].frozen = true;
                measured_any = true;
            }
        }
        (inp, measured_any, snow_oor)
    }
}

/// Day-of-year (0..365) for a (month, day), using a fixed non-leap calendar (the game uses a
/// 365-day year). month is 1..12, day 1..31.
pub fn day_of_year(month: u32, day: u32) -> u32 {
    const CUM: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let m = (month.clamp(1, 12) - 1) as usize;
    CUM[m] + (day.clamp(1, 31) - 1)
}

/// The winter fraction 0..1 for a date — peaks at 1.0 around mid-January, troughs at 0.0
/// around mid-July, a smooth cosine through the year. This is the season crossfade the engine
/// applies between the `seasons.txt` blocks (whose shipped colorbalances are near-neutral, so
/// the dominant visible season effect IS this snow/mud winter level).
pub fn season_winter_fraction(month: u32, day: u32) -> f32 {
    let doy = day_of_year(month, day) as f32;
    // phase: 0 at ~Jan 15 (peak winter), pi at ~Jul 15 (peak summer).
    let peak_winter = day_of_year(1, 15) as f32; // ~14
    let year = 365.0;
    let phase = ((doy - peak_winter) / year) * std::f32::consts::TAU;
    // cos: 1 at phase 0 (winter), -1 at phase pi (summer) → remap to 0..1.
    (phase.cos() * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// A latitude-banded seasonal colour balance (the `seasons.txt` north/center/south
/// `colorbalance` RGB multiply + `hsv` shift), date-interpolated. The shipped vanilla values
/// are near-neutral; this faithfully reproduces the blend so a modded `seasons.txt` with
/// stronger values (or the data side's own season) tints the map correctly.
#[derive(Debug, Clone, Copy)]
pub struct SeasonBalance {
    /// per-channel RGB multiply over the (linear) colormap, by latitude band.
    pub colorbalance_north: [f32; 3],
    pub colorbalance_center: [f32; 3],
    pub colorbalance_south: [f32; 3],
    /// the season `hsv_*` shift by band (H shift, S multiply, V multiply). Applied as a
    /// postprocess (restorescene.shader) HSV adjust — vanilla ships near-neutral values
    /// (north S=0.1 slightly desaturates). [seasons.txt:63-123] (Codex point 4).
    pub hsv_north: [f32; 3],
    pub hsv_center: [f32; 3],
    pub hsv_south: [f32; 3],
}

impl SeasonBalance {
    /// The shipped vanilla `seasons.txt` balance (near-neutral: north slightly cooler). All
    /// four active season blocks ship the SAME values, so this is date-independent in vanilla;
    /// kept as a function of date so a modded seasons.txt (different per block) still blends.
    /// [map/seasons.txt:63-123]
    pub fn vanilla(_month: u32, _day: u32) -> SeasonBalance {
        SeasonBalance {
            colorbalance_north: [0.9, 0.9, 1.0],
            colorbalance_center: [1.0, 1.0, 1.0],
            colorbalance_south: [1.0, 1.0, 1.0],
            // hsv_* (H, S, V) — vanilla: north {0 0.1 1} (slight desat), center/south {0 1 1}.
            hsv_north: [0.0, 0.1, 1.0],
            hsv_center: [0.0, 1.0, 1.0],
            hsv_south: [0.0, 1.0, 1.0],
        }
    }

    /// The colour-balance multiply for a normalised map-Y (0 = north/top, 1 = south/bottom):
    /// blend north→center over the top half, center→south over the bottom half.
    pub fn balance_at(&self, map_v: f32) -> [f32; 3] {
        let v = map_v.clamp(0.0, 1.0);
        let mut out = [0.0f32; 3];
        if v < 0.5 {
            let t = v / 0.5;
            for i in 0..3 {
                out[i] = self.colorbalance_north[i]
                    + (self.colorbalance_center[i] - self.colorbalance_north[i]) * t;
            }
        } else {
            let t = (v - 0.5) / 0.5;
            for i in 0..3 {
                out[i] = self.colorbalance_center[i]
                    + (self.colorbalance_south[i] - self.colorbalance_center[i]) * t;
            }
        }
        out
    }
}

/// The day/night ENVIRONMENT state derived from the in-game hour. The game drives the
/// day/night terminator from `DayNight_Hour_SunDir` (`.x` = hour longitude phase, `.yzw` =
/// sun direction). For the political map we expose a global `night_factor` 0..1 (0 = full day,
/// 1 = deep night) plus the sun direction so the renderer can tint the map tone + add city
/// lights exactly as `DayNightWithBlend` + the city-lights additive do.
#[derive(Debug, Clone, Copy)]
pub struct SunState {
    /// global night factor 0..1 (the average of `DayNightFactor` over the map — drives the
    /// whole-map tone shift + city-lights intensity).
    pub night_factor: f32,
    /// the sun direction unit vector (toward the sun), for relief lighting at this hour.
    pub sun_dir: [f32; 3],
    /// the game's `DayNight_Hour_SunDir`: `.x` = hour longitude phase (0..1 around the globe),
    /// `.yzw` = the sun direction the per-pixel `DayNightFactor` dots the globe normal against.
    /// This drives the day/night TERMINATOR sweeping across the map by longitude — the faithful
    /// per-pixel day/night the shader does, not just a whole-map factor. [standardfuncsgfx.fxh:349-360]
    pub hour_sun: [f32; 4],
}

impl SunState {
    /// Derive the sun state from the in-game hour 0..23. The sun crosses the sky over the day;
    /// `night_factor` peaks (1.0) at hour 0 (midnight) and troughs (0.0) at hour 12 (noon),
    /// with the `NIGHT_OPACITY` scaling applied by the renderer. The sun direction sweeps
    /// east→west; at noon it is overhead-ish (matching the game's noon sun). This is a faithful
    /// global reduction of the per-pixel `DayNightFactor`/`CalculateSunDirection` (which depend
    /// on the globe normal); for a top-down political map the global tone shift is what reads.
    pub fn from_hour(hour: u32) -> SunState {
        let h = (hour % 24) as f32;
        // night factor: 1 at midnight, 0 at noon (cosine over the day). This is the GLOBAL
        // average tone shift; the per-pixel terminator (below) gives the spatial day/night.
        let phase = (h / 24.0) * std::f32::consts::TAU; // 0 at midnight
        let night = (phase.cos() * 0.5 + 0.5).clamp(0.0, 1.0); // 1 at h=0, 0 at h=12
                                                               // relief sun for lighting: sweeps east(-x)→west(+x), elevation peaks noon.
        let day_angle = ((h - 6.0) / 12.0) * std::f32::consts::PI; // 0 at 6h, pi at 18h
        let elev = day_angle.sin().max(0.05);
        let az = -day_angle.cos();
        let mut d = [az, elev, 0.4];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-4);
        for c in &mut d {
            *c /= l;
        }
        // --- per-pixel day/night terminator inputs (DayNight_Hour_SunDir) -------------
        // hour phase .x: the longitude (0..1 around the globe) added inside CalcGlobeNormal so
        // the terminator sweeps with the hour. At noon (h=12) the sub-solar longitude is the
        // GMT meridian (phase contributes 0).
        let hour_phase = ((h - 12.0) / 24.0).rem_euclid(1.0);
        // sun dir .yzw: DayNightFactor = dot(globe_normal, sun) with factor==1 meaning NIGHT
        // (city lights use it). So the SUN-FACING side must give a NEGATIVE dot (day, factor 0)
        // and the far side a positive dot (night). The sub-solar globe normal at the equator is
        // +Z; to make THAT face DAY, the night vector is the OPPOSITE: (0,0,-1). (Codex A.1:
        // the earlier (0,0,1) was sign-inverted — it made the GMT/noon face read as night.)
        let sun_yzw = [0.0f32, 0.0, -1.0];
        SunState {
            night_factor: night,
            sun_dir: d,
            hour_sun: [hour_phase, sun_yzw[0], sun_yzw[1], sun_yzw[2]],
        }
    }

    /// The whole-map night TONE multiplier the renderer applies (the average effect of
    /// `DayNightWithBlend`): at full night the map darkens toward `NIGHT_DARKNESS` scaled by
    /// `NIGHT_OPACITY`. Returns a 0..1 brightness multiplier for the day colour.
    pub fn night_tone_mul(&self) -> f32 {
        // lerp(1, NIGHT_DARKNESS, night_factor * NIGHT_OPACITY). [standardfuncsgfx.fxh:393,380]
        let t = self.night_factor * NIGHT_OPACITY;
        1.0 + (NIGHT_DARKNESS - 1.0) * t
    }
}

// --- exact night constants [standardfuncsgfx.fxh:328-329] ---
/// Night opacity (how much of the night look blends in). [standardfuncsgfx.fxh:328]
pub const NIGHT_OPACITY: f32 = 0.85;
/// Night darkness (the multiplier night colour is scaled by). [standardfuncsgfx.fxh:329]
pub const NIGHT_DARKNESS: f32 = 0.5;

/// Pack the per-province dynamic state into a per-province LUT texture row (RGBA8, width =
/// num_provinces): R = winter·255, G = mud·255, B = frozen·255, A = 255 (valid), where
/// winter/mud are the FAITHFUL SnowMud crossfade `lerp(old, current, snow_mud_fade)` resolved
/// here on the CPU (the engine's SnowMudData IS this resolved texture; the shader then applies
/// the resolved value at full strength). `snow_mud_fade` defaults to 1.0 (= current). Read in
/// the shader at `(province_id, 0)`. (Codex #3: fade is over old↔current channels, not 0.)
pub fn pack_province_dynamic_lut(provinces: &[ProvinceDynamic], snow_mud_fade: f32) -> Vec<u8> {
    let f = snow_mud_fade.clamp(0.0, 1.0);
    let mut lut = vec![0u8; provinces.len() * 4];
    for (i, p) in provinces.iter().enumerate() {
        let o = i * 4;
        // Faithful old↔current crossfade (Codex A.3): lerp(old, current, fade), NO sentinel —
        // the data side initializes old=current when there's no previous tick (see
        // [`ProvinceDynamic::steady`] / [`ProvinceDynamic::resolved`]). A legitimate
        // old=0→current>0 transition therefore fades correctly.
        let winter = p.winter_level_old + (p.winter_level - p.winter_level_old) * f;
        let mud = p.mud_level_old + (p.mud_level - p.mud_level_old) * f;
        lut[o] = (winter.clamp(0.0, 1.0) * 255.0).round() as u8;
        lut[o + 1] = (mud.clamp(0.0, 1.0) * 255.0).round() as u8;
        lut[o + 2] = if p.frozen { 255 } else { 0 };
        // A = naval dominance encoded −1..1 → 0..255 (128 = none). The shader reads it for the
        // dominance_fx_apply pulse on contested seas; pid<dyn_w already gates validity.
        lut[o + 3] = ((p.dominance.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8;
    }
    lut
}

/// The GPU dynamic UNIFORM the renderer binds each tick — the globals the dynamic shader
/// branches read. Padded to 16-byte alignment for wgpu.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DynamicUniforms {
    /// .x = night_factor 0..1, .y = SnowMudFade 0..1, .z = global_time, .w = night_tone_mul.
    pub state: [f32; 4],
    /// sun direction (xyz) + pad — for relief lighting at this hour.
    pub sun_dir: [f32; 4],
    /// `DayNight_Hour_SunDir`: .x = hour longitude phase, .yzw = the per-pixel day/night sun
    /// vector (the terminator the shader sweeps across the map by longitude).
    pub hour_sun: [f32; 4],
    /// season colour-balance for the north band (xyz) + pad.
    pub cb_north: [f32; 4],
    /// season colour-balance for the center band (xyz) + pad.
    pub cb_center: [f32; 4],
    /// season colour-balance for the south band (xyz) + pad.
    pub cb_south: [f32; 4],
    /// naval-dominance ENEMY colour (xyz, from naval_dominance_fx.dds px0) + pad.
    pub dom_enemy: [f32; 4],
    /// naval-dominance FRIENDLY colour (xyz, from naval_dominance_fx.dds px1) + pad.
    pub dom_friendly: [f32; 4],
}

impl DynamicUniforms {
    /// Assemble the GPU uniform from the high-level dynamic inputs (resolving the season
    /// balance + sun state). `snow_mud_fade` is taken from the inputs if set (>0) else derived
    /// from the date — faithful to the engine's date-driven crossfade.
    pub fn from_inputs(inp: &DynamicInputs) -> DynamicUniforms {
        let (_, month, day, hour) = inp.date;
        let sun = SunState::from_hour(hour);
        let season = SeasonBalance::vanilla(month, day);
        // fade = the old↔current SnowMud interp factor (data side sets it; default 1.0 = full
        // current). It scales how much of the per-province winter/mud the shader applies.
        let fade = inp.snow_mud_fade;
        // pack the colour-balance RGB in xyz and the season HSV saturation-multiply in w.
        let cb = |c: [f32; 3], hsv: [f32; 3]| [c[0], c[1], c[2], hsv[1]];
        DynamicUniforms {
            state: [
                sun.night_factor,
                fade.clamp(0.0, 1.0),
                inp.global_time,
                sun.night_tone_mul(),
            ],
            sun_dir: [sun.sun_dir[0], sun.sun_dir[1], sun.sun_dir[2], 0.0],
            hour_sun: sun.hour_sun,
            cb_north: cb(season.colorbalance_north, season.hsv_north),
            cb_center: cb(season.colorbalance_center, season.hsv_center),
            cb_south: cb(season.colorbalance_south, season.hsv_south),
            // dominance colours default to the game's red/cyan; overwritten by the renderer
            // from naval_dominance_fx.dds when materials are uploaded.
            dom_enemy: [1.0, 0.04, 0.1, 0.0],
            dom_friendly: [0.0, 0.55, 1.0, 0.0],
        }
    }
}

/// SNOW constants used by the dynamic snow branch (mirrored from the game, [constants.fxh]).
pub mod snow {
    /// Snow appears above this world height. [constants.fxh:75]
    pub const START_HEIGHT: f32 = 3.0;
    /// Upward-normal start for snow (snow only on near-flat-up faces). [constants.fxh:77]
    pub const NORMAL_START: f32 = 0.7;
    /// Cliff factor (how sharply the normal gate ramps). [constants.fxh:80]
    pub const CLIFFS: f32 = 5.0;
    /// Snow colour (linear). [constants.fxh:78]
    pub const COLOR: [f32; 3] = super::gc::SNOW_COLOR;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_year_known_dates() {
        assert_eq!(day_of_year(1, 1), 0);
        assert_eq!(day_of_year(2, 1), 31);
        assert_eq!(day_of_year(12, 31), 364);
        assert_eq!(day_of_year(7, 15), 195);
    }

    /// Winter fraction: ~1 in deep winter (Jan), ~0 in deep summer (Jul), monotone-ish between.
    #[test]
    fn winter_fraction_summer_to_winter() {
        let jan = season_winter_fraction(1, 15);
        let jul = season_winter_fraction(7, 15);
        let apr = season_winter_fraction(4, 15);
        let oct = season_winter_fraction(10, 15);
        assert!(jan > 0.95, "deep winter {jan}");
        assert!(jul < 0.05, "deep summer {jul}");
        // spring/autumn are intermediate.
        assert!((0.2..0.8).contains(&apr), "spring {apr}");
        assert!((0.2..0.8).contains(&oct), "autumn {oct}");
    }

    /// Sun: night at midnight, day at noon; the tone multiplier darkens at night.
    #[test]
    fn sun_day_to_night() {
        let midnight = SunState::from_hour(0);
        let noon = SunState::from_hour(12);
        assert!(
            midnight.night_factor > 0.95,
            "midnight night {}",
            midnight.night_factor
        );
        assert!(noon.night_factor < 0.05, "noon night {}", noon.night_factor);
        // night tone darker than day.
        assert!(midnight.night_tone_mul() < noon.night_tone_mul());
        // exact: day tone = 1.0 (no darkening), midnight tone = lerp(1,0.5,0.85)=0.575.
        assert!(
            (noon.night_tone_mul() - 1.0).abs() < 0.05,
            "day {}",
            noon.night_tone_mul()
        );
        assert!(
            (midnight.night_tone_mul() - 0.575).abs() < 0.02,
            "night {}",
            midnight.night_tone_mul()
        );
        // sun points up at noon (positive y, dominant), and is unit length.
        let d = noon.sun_dir;
        assert!(d[1] > 0.5, "noon sun up {d:?}");
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-3, "unit sun {len}");
    }

    /// Season balance: north band cooler (less red), center neutral; banded interpolation.
    #[test]
    fn season_balance_latitude_band() {
        let sb = SeasonBalance::vanilla(1, 1);
        let north = sb.balance_at(0.0);
        let center = sb.balance_at(0.5);
        let south = sb.balance_at(1.0);
        assert_eq!(north, [0.9, 0.9, 1.0]);
        assert_eq!(center, [1.0, 1.0, 1.0]);
        assert_eq!(south, [1.0, 1.0, 1.0]);
        // a point at v=0.25 is halfway north..center.
        let mid = sb.balance_at(0.25);
        assert!((mid[0] - 0.95).abs() < 1e-5, "banded blend {mid:?}");
    }

    /// The per-province dynamic LUT packs winter/mud/frozen into RGB, alpha = naval dominance
    /// (−1..1 → 0..255, 128 = none). Default dominance 0 → alpha 128.
    #[test]
    fn pack_dynamic_lut_channels() {
        let provs = vec![
            ProvinceDynamic {
                winter_level: 1.0,
                weather: WeatherType::Snow,
                ..Default::default()
            },
            ProvinceDynamic {
                mud_level: 1.0,
                weather: WeatherType::RainHeavy,
                ..Default::default()
            },
            ProvinceDynamic {
                winter_level: 1.0,
                frozen: true,
                weather: WeatherType::ArcticWater,
                ..Default::default()
            },
            // a contested sea: friendly dominance +1 → alpha 255.
            ProvinceDynamic {
                dominance: 1.0,
                ..Default::default()
            },
            // enemy dominance −1 → alpha 0.
            ProvinceDynamic {
                dominance: -1.0,
                ..Default::default()
            },
        ];
        let lut = pack_province_dynamic_lut(&provs, 1.0);
        // province 0: full winter; alpha = dominance 0 → 128.
        assert_eq!(&lut[0..4], &[255, 0, 0, 128]);
        // province 1: full mud.
        assert_eq!(&lut[4..8], &[0, 255, 0, 128]);
        // province 2: full winter + frozen.
        assert_eq!(&lut[8..12], &[255, 0, 255, 128]);
        // province 3: friendly dominance → alpha 255.
        assert_eq!(lut[3 * 4 + 3], 255);
        // province 4: enemy dominance → alpha 0.
        assert_eq!(lut[4 * 4 + 3], 0);
    }

    /// The SnowMud crossfade `lerp(old, current, fade)` resolves CPU-side (Codex #3): fade 0
    /// = OLD value, fade 1 = CURRENT, fade 0.5 = halfway. Faithful old↔current, never toward 0.
    #[test]
    fn pack_dynamic_lut_old_current_crossfade() {
        let provs = vec![ProvinceDynamic {
            winter_level: 1.0,     // current = full winter
            winter_level_old: 0.2, // old/summer = light
            ..Default::default()
        }];
        // fade 0 → old (0.2 → 51).
        assert_eq!(pack_province_dynamic_lut(&provs, 0.0)[0], 51);
        // fade 1 → current (1.0 → 255).
        assert_eq!(pack_province_dynamic_lut(&provs, 1.0)[0], 255);
        // fade 0.5 → halfway (0.6 → 153).
        let mid = pack_province_dynamic_lut(&provs, 0.5)[0];
        assert!((mid as i32 - 153).abs() <= 1, "mid {mid}");
    }

    /// The GPU uniform assembles from inputs: a summer-NOON tick vs a winter-MIDNIGHT tick
    /// differ in night factor and tone (day↔night), proving the time parameterisation drives
    /// the look. (Per Codex: SnowMudFade is the old↔current interp factor — default 1.0 for
    /// both — and the WINTER look comes from per-province winter_level, tested separately.)
    #[test]
    fn dynamic_uniforms_summer_noon_vs_winter_midnight() {
        let summer_noon = DynamicInputs::neutral(3, (1940, 7, 15, 12));
        let winter_midnight = DynamicInputs::neutral(3, (1941, 1, 15, 0));
        let su = DynamicUniforms::from_inputs(&summer_noon);
        let wu = DynamicUniforms::from_inputs(&winter_midnight);
        // night factor (global): ~0 noon, ~1 midnight.
        assert!(
            su.state[0] < 0.05 && wu.state[0] > 0.95,
            "night {} {}",
            su.state[0],
            wu.state[0]
        );
        // snow-mud fade defaults to 1.0 (full current) for BOTH — it's an interp factor.
        assert!(
            (su.state[1] - 1.0).abs() < 1e-6 && (wu.state[1] - 1.0).abs() < 1e-6,
            "fade"
        );
        // tone: noon = 1 (no darkening), midnight darker.
        assert!(
            su.state[3] > 0.95 && wu.state[3] < 0.65,
            "tone {} {}",
            su.state[3],
            wu.state[3]
        );
    }

    /// The snapshot WEATHER domain feeds the per-province dynamic input: a measured
    /// `weather.provinces.<pid>.mud=true` sets `mud_level=1.0` at THAT province id (1-based,
    /// no compaction); `mud=false` sets 0.0; an unmeasured province stays neutral. The
    /// `measured_any` gate is true iff at least one province carried a measured mud value.
    #[test]
    fn from_weather_snapshot_maps_mud_per_province() {
        let weather = serde_json::json!({
            "regions": { "5": { "temperature": -20.0, "phenomenon": ["snow"] } },
            "provinces": {
                "2": { "mud": true,  "temp_c": -20.0 },  // measured mud → 1.0
                "3": { "mud": false, "temp_c": -30.0 },  // measured no-mud → 0.0
                "4": { "temp_c": 5.0 }                   // NO mud field → neutral (no-fab)
            }
        });
        let (inp, measured_any) =
            DynamicInputs::from_weather_snapshot(6, (1941, 1, 15, 12), Some(&weather));
        assert!(
            measured_any,
            "at least one province carried a measured mud value"
        );
        // province 2: mud → 1.0 (and steady old==current).
        assert_eq!(inp.provinces[2].mud_level, 1.0);
        assert_eq!(inp.provinces[2].mud_level_old, 1.0);
        // province 3: measured no-mud → 0.0 (a real measurement, distinct from unmeasured).
        assert_eq!(inp.provinces[3].mud_level, 0.0);
        // NO-FAB: temp_c is NOT turned into winter_level/frozen/weather; despite -20C @ p2,
        // winter stays 0 and weather stays Clear (snow coverage is not measured per-province).
        assert_eq!(
            inp.provinces[2].winter_level, 0.0,
            "temp_c must NOT fabricate winter"
        );
        assert!(!inp.provinces[2].frozen, "temp_c must NOT fabricate frozen");
        assert_eq!(
            inp.provinces[2].weather,
            WeatherType::Clear,
            "region phenomenon must NOT fabricate type"
        );
        // province 4 (no mud field) + province 5 (not listed) stay neutral.
        assert_eq!(inp.provinces[4].mud_level, 0.0);
        assert_eq!(inp.provinces[5].mud_level, 0.0);
    }

    /// No weather domain (or an empty one) → `measured_any=false` so the caller keeps the
    /// neutral/static path; the inputs are exactly `neutral` (every province no-snow/mud/ice).
    #[test]
    fn from_weather_snapshot_absent_is_neutral_and_ungated() {
        let (inp_none, any_none) = DynamicInputs::from_weather_snapshot(4, (1940, 7, 15, 12), None);
        assert!(
            !any_none,
            "no weather domain → not gated on (keep static path)"
        );
        assert!(inp_none
            .provinces
            .iter()
            .all(|p| p.mud_level == 0.0 && p.winter_level == 0.0));
        // a weather object with no measured province mud is also un-gated (nothing to show).
        let empty =
            serde_json::json!({ "regions": { "5": { "temperature": 1.0 } }, "provinces": {} });
        let (_, any_empty) =
            DynamicInputs::from_weather_snapshot(4, (1940, 7, 15, 12), Some(&empty));
        assert!(
            !any_empty,
            "weather present but no measured province mud → ungated"
        );
    }

    /// Robustness (Codex traps): an out-of-range or unparseable province id is SKIPPED, never
    /// coerced to province 0 — so a stray key cannot corrupt the LUT.
    #[test]
    fn from_weather_snapshot_skips_bad_province_ids() {
        let weather = serde_json::json!({ "provinces": {
            "0": { "mud": true },        // id 0 is valid index but a real province key;
            "999": { "mud": true },      // out of range (num=4) → skipped, not clamped
            "abc": { "mud": true }       // unparseable → skipped, NOT province 0
        }});
        let (inp, any) = DynamicInputs::from_weather_snapshot(4, (1940, 7, 15, 12), Some(&weather));
        assert!(any, "the valid in-range id 0 was measured");
        assert_eq!(
            inp.provinces[0].mud_level, 1.0,
            "id 0 maps to index 0 (no off-by-one)"
        );
        // the out-of-range / unparseable keys did not write anywhere in range.
        assert_eq!(inp.provinces[1].mud_level, 0.0);
        assert_eq!(inp.provinces[2].mud_level, 0.0);
        assert_eq!(inp.provinces[3].mud_level, 0.0);
    }

    /// SNOW feed: a measured `weather.provinces.<pid>.snow` number → `winter_level` at that
    /// province (1-based, direct), clamped to 0..1; a measured 0.0 is a real reading; an absent
    /// snow leaves winter neutral. snow gates `measured_any` independent of mud (Deliverable 1).
    #[test]
    fn from_weather_snapshot_maps_snow_per_province() {
        let weather = serde_json::json!({
            "provinces": {
                "2": { "snow": 0.8 },              // measured snow → winter 0.8
                "3": { "snow": 0.0, "mud": false },// measured 0 snow → winter 0.0 (still measured)
                "4": { "mud": true },              // mud only, no snow → winter stays 0
                "5": { "temp_c": -10.0 }           // neither → neutral
            }
        });
        let (inp, measured) =
            DynamicInputs::from_weather_snapshot(7, (1941, 1, 15, 12), Some(&weather));
        assert!(measured, "a measured snow value gates the dynamic feed on");
        assert!(
            (inp.provinces[2].winter_level - 0.8).abs() < 1e-6,
            "snow → winter 0.8"
        );
        assert_eq!(
            inp.provinces[2].winter_level_old, inp.provinces[2].winter_level,
            "steady"
        );
        assert_eq!(
            inp.provinces[3].winter_level, 0.0,
            "measured 0 snow → winter 0"
        );
        // mud-only province has NO snow datum → winter stays neutral (not fabricated).
        assert_eq!(
            inp.provinces[4].winter_level, 0.0,
            "mud must not fabricate winter"
        );
        // province 5 (neither) + province 6 (absent) stay neutral.
        assert_eq!(inp.provinces[5].winter_level, 0.0);
        assert_eq!(inp.provinces[6].winter_level, 0.0);
    }

    /// A snapshot whose ONLY weather signal is snow (no mud) still gates on; a snapshot with
    /// neither snow nor mud (nor frozen) is ungated → static map preserved.
    #[test]
    fn snow_only_gates_on_and_absent_stays_static() {
        let snow_only = serde_json::json!({ "provinces": { "1": { "snow": 0.5 } } });
        let (_, gated) =
            DynamicInputs::from_weather_snapshot(3, (1941, 1, 1, 12), Some(&snow_only));
        assert!(gated, "snow-only weather must gate on");
        // a weather object with only temperatures (no snow/mud) → ungated.
        let no_visual = serde_json::json!({ "provinces": { "1": { "temp_c": -20.0 } } });
        let (_, g2) = DynamicInputs::from_weather_snapshot(3, (1941, 1, 1, 12), Some(&no_visual));
        assert!(
            !g2,
            "temp-only weather must NOT gate on (no snow/mud/frozen)"
        );
    }

    /// Codex (a): a measured snow value OUTSIDE 0..1 is clamped to the shader range AND counted
    /// (surfaced), never silently turned into a plausible visual.
    #[test]
    fn snow_out_of_range_is_clamped_and_counted() {
        let weather = serde_json::json!({
            "provinces": { "1": { "snow": 1.5 }, "2": { "snow": -0.2 }, "3": { "snow": 0.4 } }
        });
        let (inp, measured, oor) =
            DynamicInputs::from_weather_snapshot_counted(4, (1941, 1, 1, 12), Some(&weather));
        assert!(measured);
        assert_eq!(inp.provinces[1].winter_level, 1.0, "1.5 clamped to 1.0");
        assert_eq!(inp.provinces[2].winter_level, 0.0, "-0.2 clamped to 0.0");
        assert!(
            (inp.provinces[3].winter_level - 0.4).abs() < 1e-6,
            "in-range kept"
        );
        assert_eq!(oor, 2, "two out-of-range snow values counted");
    }

    /// FROZEN-SEAS overlay (Deliverable 3): `from_weather_snapshot_full` sets `frozen` on the
    /// supplied sea-province set, gates measured_any on, and never touches non-listed provinces.
    /// A frozen-only snapshot (no snow/mud) still gates the dynamic feed on.
    #[test]
    fn frozen_overlay_sets_sea_provinces_and_gates() {
        use std::collections::HashSet;
        let frozen: HashSet<u32> = [10u32, 11].into_iter().collect();
        // no per-province snow/mud at all → without frozen this would be ungated.
        let (inp, measured, _) =
            DynamicInputs::from_weather_snapshot_full(13, (1941, 1, 15, 12), None, &frozen);
        assert!(measured, "a frozen sea from measured arctic_water gates on");
        assert!(
            inp.provinces[10].frozen && inp.provinces[11].frozen,
            "listed seas frozen"
        );
        assert!(
            !inp.provinces[9].frozen && !inp.provinces[12].frozen,
            "others untouched"
        );
        // an out-of-range frozen id is ignored (never writes past the LUT).
        let oob: HashSet<u32> = [9999u32].into_iter().collect();
        let (inp2, m2, _) =
            DynamicInputs::from_weather_snapshot_full(4, (1941, 1, 1, 12), None, &oob);
        assert!(
            !m2,
            "an out-of-range frozen id does not gate on (no province written)"
        );
        assert!(inp2.provinces.iter().all(|p| !p.frozen));
    }

    /// A blizzard/snow weather province carries winter_level (data side sets it); the renderer
    /// reads it via the LUT. This proves clear→blizzard drives the look through the contract.
    #[test]
    fn weather_drives_winter_level_through_contract() {
        let mut inp = DynamicInputs::neutral(2, (1940, 7, 15, 12)); // summer noon, no global winter
                                                                    // a blizzard hits province 1 even in summer: the data side sets its winter_level high.
        inp.provinces[1].weather = WeatherType::Blizzard;
        inp.provinces[1].winter_level = 1.0;
        inp.provinces[1].frozen = true;
        let lut = pack_province_dynamic_lut(&inp.provinces, inp.snow_mud_fade);
        assert_eq!(&lut[0..4], &[0, 0, 0, 128]); // province 0 clear (alpha 128 = no dominance)
        assert_eq!(&lut[4..8], &[255, 0, 255, 128]); // province 1 blizzard: winter+frozen
    }
}
