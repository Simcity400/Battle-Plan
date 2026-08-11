//! WEATHER1 — weather overlay rendering law, files-first, capture-validated.
//!
//! the game draws weather (clouds, rain, snow, blizzard, sandstorm, lightning) as a
//! **particle system**, not a map-composite shader pass. There is no
//! `cloud`/`weather` string anywhere in `gfx/FX/*.shader`/`*.fxh`; the visible
//! overlay the user sees from map altitude (e.g. clouds over the Alps) is the
//! `fading_entity` half of each weather effect — a cloud billboard particle
//! emitter drawn with `gfx/FX/particle.shader`.
//!
//! This module encodes the RENDER law: given a weather *type* per strategic
//! region (exact D24 supplies the type; this module never invents it),
//! how does the engine draw the overlay — 100% from shipped files, with every
//! parameter cited or explicitly assigned to its production owner.
//!
//! # The binding chain (shipped files, source of truth)
//!
//! Strategic-region weather uses these type keys (`map/strategicregions/*.txt`
//! `weather = { period = { ... } }`): `no_phenomenon`, `rain_light`,
//! `rain_heavy`, `snow`, `blizzard`, `sandstorm` (`arctic_water` / `mud` are
//! ground/water post-effects with no airborne particle, handled by SNOW1's
//! `snow_law` and the mud system, not here).
//!
//! `common/weather.txt` `weather_effects = { ... }` maps each type to one or
//! two `weather_effect` entries, each naming a foreground `entity` and (for the
//! precipitating types) a `fading_entity` — the cloud layer that is what
//! remains visible when zoomed out. The second entry per type carries
//! `small = yes` and is the *small* variant. Verbatim (`common/weather.txt`,
//! `weather_effects` block):
//! ```text
//! rain_light  -> entity=rain_entity            fading_entity=rain_clouds_entity
//!             -> entity=rain_small_entity       fading_entity=rain_small_clouds_entity        small=yes
//! rain_heavy  -> entity=lightning_storm_entity  fading_entity=lightning_storm_clouds_entity
//!             -> entity=lightning_storm_small_entity fading_entity=lightning_storm_small_clouds_entity small=yes
//! snow        -> entity=snow_entity             fading_entity=snow_clouds_entity
//!             -> entity=snow_small_entity        fading_entity=snow_small_clouds_entity          small=yes
//! blizzard    -> entity=snow_storm_entity        fading_entity=snow_storm_clouds_entity
//!             -> entity=snow_storm_small_entity   fading_entity=snow_storm_small_clouds_entity     small=yes
//! sandstorm   -> entity=sandstorm_entity   (no fading_entity — sandstorm has no cloud layer)
//! ```
//!
//! `<name>_clouds_entity` resolves in `gfx/entities/weather_entities.asset` to
//! an `entity = { ... state = { event = { particle = "<name>_clouds_particle"
//! ... } } }`. `<name>_clouds_particle` resolves in
//! `gfx/entities/particles.gfx` `pdxparticle = { name=... type="<name>_clouds_file" }`
//! to the particle definition file `type`, which is the `name=` of the
//! `particle = { ... }` block in `gfx/particles/environment/<name>_clouds.asset`.
//!
//! # The cloud subsystem (per `gfx/particles/environment/*_clouds.asset`)
//!
//! Each cloud `.asset` has a `subsystem = { ... }` (the lightning variants have
//! a second `flash` additive subsystem). The render-relevant fields (example
//! `rain_clouds.asset`):
//! - `texture = { file="gfx/particles/cloud_6.dds" shader="ParticleAdditive" }`
//!   — the billboard sprite and its `gfx/FX/particle.shader` Effect.
//! - `color = { x=110 y=120 z=150 alpha={ 180,smoke_fade 20 } }` — the vertex
//!   COLOR, components in 0..255, alpha modulated by an animation curve.
//! - `size = { 60.5 20 }` — billboard world size (mean, variance).
//! - `billboard = yes` — camera-facing quad (particle.shader's default, not the
//!   `NO_BILLBOARD` path).
//! - `life = { 2.6 0.7 }`, `emission`, `max_amount`, `velocity`, `force` —
//!   simulation params consumed by XMAP20's generic particle runtime.
//!
//! # The pixel-shader color law (`gfx/FX/particle.shader`)
//!
//! `VertexSimpleParticle` sets `Out.vColor = ToLinear( v.vColor )`
//! (`particle.shader:151`). `ToLinear(float4)` is
//! `float4( pow(rgb, 2.2), a )` (`standardfuncsgfx.fxh:53-56`) — gamma 2.2 on
//! RGB, alpha passed through. The vertex COLOR `v.vColor` is the asset `color`
//! normalized to 0..1 (the D3D `COLOR` semantic is a UNORM8 that the engine
//! fills from the 0..255 asset components — `SnowMudData`/`color_lut` in this
//! crate use the same `/255` convention, `color_lut.rs:128-130`).
//! `PixelSimpleParticle` returns `tex2D(DiffuseMap, uv) * vColor`
//! (`particle.shader:168`) — the sprite texel (linear, from the DXT5/BC3 DDS)
//! times the linear vertex color. The `FOG` define (present on
//! `ParticleAlphaBlend`, absent on `ParticleAdditive`) applies the exact
//! compile-time `constants.fxh`/`standardfuncsgfx.fxh` distance-fog law to RGB
//! before return (`particle.shader:189-191`).
//!
//! Blend (`particle.shader` BlendState blocks, `WriteMask="RED|GREEN|BLUE"` —
//! particles never write alpha to the target):
//! - `ParticleAlphaBlend`  : Src=SRC_ALPHA Dst=INV_SRC_ALPHA (`:200-206`)
//! - `ParticleAdditive`    : Src=SRC_ALPHA Dst=ONE           (`:208-214`)
//! - `ParticlePreAlphaBlend`: Src=ONE      Dst=INV_SRC_ALPHA (`:216-222`)
//! So over a destination `Dst.rgb` a particle fragment `src = texel*vColor`
//! contributes:
//! - AlphaBlend : `src.rgb*src.a + Dst.rgb*(1-src.a)`
//! - Additive   : `src.rgb*src.a + Dst.rgb`
//!
//! # Camera distance / zoom fade (`common/defines/00_graphics.lua:1008-1015`)
//!
//! Exact-a729 `CAirInterface::GetWeatherEffectForAirRegion`
//! (`hoi4.exe+0x0166ec50`) applies a piecewise whole-layer alpha and particle
//! playback-rate law to camera distance. The constants remain effective-VFS
//! inputs. Constants:
//! `WEATHER_DISTANCE_CUTOFF=1500` (hidden beyond),
//! `WEATHER_DISTANCE_FADE_LENGTH=400`, `WEATHER_ZOOM_IN_CUTOFF=358`,
//! `WEATHER_ZOOM_IN_FADE_LENGTH=220`, `WEATHER_ZOOM_IN_FADE_FACTOR=0.0`,
//! `WEATHER_PLAYBACK_RATE=0.15`, `WEATHER_PLAYBACK_RATE_CUTOFF=500`, and
//! `WEATHER_PLAYBACK_RATE_LENGTH=200`.
//!
//! # Spawn positions (`map/weatherpositions.txt`)
//!
//! One entry per line, `;`-separated: `region_id;x;y;z;size` with `size` in
//! `{big,small}`. `(x,y,z)` is the world position of a weather emitter (same
//! `[0,5632] x [0,2048]` map plane in x/z that SNOW1 documents; `y` is height
//! ~9..15). `big` uses the primary `weather_effect`, `small` uses the
//! `small=yes` variant. Multiple entries per region place multiple emitters.
//!
//! # What is a pure function of files (this module) vs. runtime state
//!
//! Pure function of mod-resolved files + a supplied weather *type*: the sprite
//! texture, the linear vertex color, the blend mode, the billboard size, the
//! spawn positions, and fade constants. Runtime state comes from exact D24,
//! the presentation-independent visual epoch, viewer camera, and the owning
//! lighting/composition packets. Live particle age and turbulence are renderer
//! temporal state, never selected-Present authority.

use crate::image_load::load_dds_rgba;
use crate::mod_resolver::GameFileResolver;
use crate::types::RgbaImage;
use crate::Result;
use std::{collections::BTreeSet, path::Path};

/// `common/weather.txt` source path (mod-resolved).
pub const WEATHER_EFFECTS_SOURCE: &str = "common/weather.txt";
/// `gfx/entities/weather_entities.asset` source path (mod-resolved).
pub const WEATHER_ENTITIES_SOURCE: &str = "gfx/entities/weather_entities.asset";
/// `gfx/entities/particles.gfx` source path (mod-resolved).
pub const PARTICLES_GFX_SOURCE: &str = "gfx/entities/particles.gfx";
/// `map/weatherpositions.txt` source path (mod-resolved).
pub const WEATHER_POSITIONS_SOURCE: &str = "map/weatherpositions.txt";
/// `common/defines/00_graphics.lua` source path (mod-resolved).
pub const GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
/// Shipped particle program whose named effects bind the decoded cloud law.
pub const PARTICLE_SHADER_SOURCE: &str = "gfx/FX/particle.shader";
/// Compile-time distance-fog constants included by the particle shader.
pub const FOG_CONSTANTS_SOURCE: &str = "gfx/FX/constants.fxh";

/// `standardfuncsgfx.fxh:48-56` `ToLinear` gamma exponent.
pub const TO_LINEAR_GAMMA: f32 = 2.2;

/// The weather *type* keys used by `map/strategicregions/*.txt` weather periods
/// and keyed by `common/weather.txt` `weather_effects`. Only the types with a
/// particle overlay are represented here (`no_phenomenon` draws nothing;
/// `arctic_water`/`mud` are ground post-effects, not airborne particles).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum WeatherType {
    /// No weather particle overlay.
    NoPhenomenon,
    /// `rain_light` — rain + `rain_clouds` overlay.
    RainLight,
    /// `rain_heavy` — lightning storm + `lightning_storm_clouds` overlay.
    RainHeavy,
    /// `snow` — snowfall + `snow_clouds` overlay.
    Snow,
    /// `blizzard` — snowstorm + `snow_storm_clouds` overlay.
    Blizzard,
    /// `sandstorm` — sandstorm (no cloud fading layer).
    Sandstorm,
}

impl WeatherType {
    /// Parse the strategic-region / `weather_effects` key.
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            "no_phenomenon" => Some(Self::NoPhenomenon),
            "rain_light" => Some(Self::RainLight),
            "rain_heavy" => Some(Self::RainHeavy),
            "snow" => Some(Self::Snow),
            "blizzard" => Some(Self::Blizzard),
            "sandstorm" => Some(Self::Sandstorm),
            _ => None,
        }
    }

    /// The `common/weather.txt` `weather_effects` key for this type.
    pub fn key(self) -> &'static str {
        match self {
            Self::NoPhenomenon => "no_phenomenon",
            Self::RainLight => "rain_light",
            Self::RainHeavy => "rain_heavy",
            Self::Snow => "snow",
            Self::Blizzard => "blizzard",
            Self::Sandstorm => "sandstorm",
        }
    }

    /// Decode only the exact `common/weather.txt` key recorded by D24.
    /// Friendly aliases are intentionally rejected: production cannot infer
    /// an engine phenomenon from display-oriented naming.
    pub fn from_recording_kind(kind: &str) -> Option<Self> {
        Self::from_key(kind)
    }
}

/// The `big`/`small` variant of a weather emitter position
/// (`map/weatherpositions.txt` column 5). `big` selects the primary
/// `weather_effect`, `small` selects the `small = yes` entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum WeatherPositionSize {
    /// `big` — primary (non-small) `weather_effect`.
    Big,
    /// `small` — the `small = yes` `weather_effect`.
    Small,
}

impl WeatherPositionSize {
    fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim() {
            "big" => Some(Self::Big),
            "small" => Some(Self::Small),
            _ => None,
        }
    }
}

/// One `weather_effect` binding row from `common/weather.txt`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WeatherEffectBinding {
    /// The foreground `entity = ...` (precipitation particle emitter).
    pub entity: String,
    /// The `fading_entity = ...` (cloud layer), if present. `sandstorm` has none.
    pub fading_entity: Option<String>,
    /// True iff this row carried `small = yes`.
    pub small: bool,
}

/// The `common/weather.txt` `weather_effects` table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WeatherEffectsTable {
    /// Rows in file order, keyed by weather type.
    entries: Vec<(WeatherType, WeatherEffectBinding)>,
}

impl WeatherEffectsTable {
    /// Every `(type, binding)` row, in file order.
    pub fn rows(&self) -> &[(WeatherType, WeatherEffectBinding)] {
        &self.entries
    }

    /// The binding for a `(type, size)` pair — `Big` picks the non-small row,
    /// `Small` picks the `small = yes` row (falling back to the sole row when a
    /// type has no small variant, e.g. `sandstorm`).
    pub fn binding(
        &self,
        weather: WeatherType,
        size: WeatherPositionSize,
    ) -> Option<&WeatherEffectBinding> {
        let want_small = matches!(size, WeatherPositionSize::Small);
        let of_type: Vec<&WeatherEffectBinding> = self
            .entries
            .iter()
            .filter(|(ty, _)| *ty == weather)
            .map(|(_, b)| b)
            .collect();
        of_type
            .iter()
            .copied()
            .find(|b| b.small == want_small)
            .or_else(|| of_type.first().copied())
    }

    /// All authored candidates for a weather type/size in file order.
    ///
    /// Exact-a729 selects one candidate for every non-empty emitter vector and
    /// advances the epoch PRNG once. A `big` position uses the big vector when
    /// non-empty and otherwise falls back to the small vector; `small` uses
    /// only the small vector.
    pub fn bindings(
        &self,
        weather: WeatherType,
        size: WeatherPositionSize,
    ) -> Vec<&WeatherEffectBinding> {
        let want_small = matches!(size, WeatherPositionSize::Small);
        let selected = self
            .entries
            .iter()
            .filter(|(ty, binding)| *ty == weather && binding.small == want_small)
            .map(|(_, binding)| binding)
            .collect::<Vec<_>>();
        if selected.is_empty() && matches!(size, WeatherPositionSize::Big) {
            self.entries
                .iter()
                .filter(|(ty, binding)| *ty == weather && binding.small)
                .map(|(_, binding)| binding)
                .collect()
        } else {
            selected
        }
    }
}

/// The fully-resolved cloud (`fading_entity`) render descriptor for one weather
/// type/size — the airborne overlay the user sees from map altitude.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudRenderLaw {
    /// The particle `.asset` `name` (== `pdxparticle` `type`), e.g.
    /// `rain_clouds_file`.
    pub particle_file: String,
    /// The relative sprite texture path, e.g. `gfx/particles/cloud_6.dds`.
    pub texture: String,
    /// The `gfx/FX/particle.shader` Effect name, e.g. `ParticleAdditive`.
    pub shader_effect: String,
    /// The `ParticleBlend` decoded from `shader_effect`.
    pub blend: ParticleBlend,
    /// The mean vertex COLOR in 0..255 (RGB) — the value the asset declares.
    pub color_rgb_255: [f32; 3],
    /// The mean base alpha in 0..255 (before the `smoke_fade`/`cloud_fade`
    /// animation multiplies it).
    pub base_alpha_255: f32,
    /// The animation-curve name modulating alpha (`smoke_fade`/`cloud_fade`), if
    /// any.
    pub alpha_anim: Option<String>,
    /// The peak (maximum) value of `alpha_anim`'s curve — the multiplier at the
    /// particle's brightest life phase ([`animation_curve_peak`]); 1.0 when no
    /// curve is declared. The static overlay samples the curve at this peak.
    pub alpha_anim_peak: f32,
    /// The mean billboard world size (asset `size` mean).
    pub size_mean: f32,
    /// The mean particle life in seconds (asset `life` mean).
    pub life_mean: f32,
    /// The subsystem's `sort` mode if declared (e.g. `"depth"` on the lightning
    /// cloud layers) — the engine depth-sorts these particles back-to-front. The
    /// non-lightning cloud subsystems declare no `sort`.
    pub sort: Option<String>,
}

impl CloudRenderLaw {
    /// The linear vertex color `ToLinear(color/255)` the pixel shader multiplies
    /// the sprite by — `pow(rgb/255, 2.2)` with alpha (a UNORM) passed through
    /// un-gamma'd. `anim_alpha_mul` in 0..1 is the evaluated `smoke_fade` value
    /// for the particle's current life fraction (1.0 = un-aged / peak).
    pub fn linear_vertex_color(&self, anim_alpha_mul: f32) -> [f32; 4] {
        let lin = |c: f32| (c / 255.0).powf(TO_LINEAR_GAMMA);
        let a = (self.base_alpha_255 / 255.0) * anim_alpha_mul.clamp(0.0, 1.0);
        [
            lin(self.color_rgb_255[0]),
            lin(self.color_rgb_255[1]),
            lin(self.color_rgb_255[2]),
            a.clamp(0.0, 1.0),
        ]
    }
}

/// The `gfx/FX/particle.shader` blend law for a cloud sprite.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParticleBlend {
    /// `ParticleAlphaBlend`: Src=SRC_ALPHA Dst=INV_SRC_ALPHA (`particle.shader:200-206`).
    AlphaBlend,
    /// `ParticleAdditive`: Src=SRC_ALPHA Dst=ONE (`particle.shader:208-214`).
    Additive,
    /// `ParticlePreAlphaBlend`: Src=ONE Dst=INV_SRC_ALPHA (`particle.shader:216-222`).
    PreAlphaBlend,
}

impl ParticleBlend {
    /// Decode the `texture.shader` string from an asset into a blend law.
    pub fn from_effect(effect: &str) -> Option<Self> {
        match effect.trim() {
            "ParticleAlphaBlend" | "ParticleAlphaBlendNoBillboard" => Some(Self::AlphaBlend),
            "ParticleAdditive" | "ParticleAdditiveNoBillboard" => Some(Self::Additive),
            "ParticlePreAlphaBlend" | "ParticlePreAlphaBlendNoBillboard" => {
                Some(Self::PreAlphaBlend)
            }
            _ => None,
        }
    }

    /// Blend one premultiplied-by-nothing source fragment `src` (linear rgba,
    /// alpha in [0,1]) over a linear destination `dst.rgb`, per the D3D
    /// `SourceBlend`/`DestBlend` this effect declares. `WriteMask` is RGB only
    /// (`particle.shader:205`), so destination alpha is never written.
    pub fn blend_over(self, src: [f32; 4], dst: [f32; 3]) -> [f32; 3] {
        let (sr, sg, sb, sa) = (src[0], src[1], src[2], src[3]);
        match self {
            // Src*SRC_ALPHA + Dst*(1-SRC_ALPHA)
            Self::AlphaBlend => [
                sr * sa + dst[0] * (1.0 - sa),
                sg * sa + dst[1] * (1.0 - sa),
                sb * sa + dst[2] * (1.0 - sa),
            ],
            // Src*SRC_ALPHA + Dst*ONE
            Self::Additive => [sr * sa + dst[0], sg * sa + dst[1], sb * sa + dst[2]],
            // Src*ONE + Dst*(1-SRC_ALPHA)
            Self::PreAlphaBlend => [
                sr + dst[0] * (1.0 - sa),
                sg + dst[1] * (1.0 - sa),
                sb + dst[2] * (1.0 - sa),
            ],
        }
    }
}

/// A `map/weatherpositions.txt` emitter position with a decoded size tag. This
/// wraps the shared [`crate::map_static_data::WeatherPosition`] row (which keeps
/// `size` as a raw string) with the `big`/`small` variant this render law needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherEmitter {
    /// Strategic-region id (column 1).
    pub region_id: u32,
    /// World `(x, y, z)` — x/z on the `[0,5632] x [0,2048]` map plane, y=height.
    pub position: [f32; 3],
    /// `big`/`small` variant selector.
    pub size: WeatherPositionSize,
}

/// The parsed `map/weatherpositions.txt` table (decoded emitters).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherEmitterTable {
    /// Every emitter position, in file order.
    pub emitters: Vec<WeatherEmitter>,
}

impl WeatherEmitterTable {
    /// All emitters for one strategic region.
    pub fn for_region(&self, region_id: u32) -> impl Iterator<Item = &WeatherEmitter> {
        self.emitters
            .iter()
            .filter(move |p| p.region_id == region_id)
    }
}

/// The `WEATHER_*` camera-fade and playback constants
/// (`00_graphics.lua:1008-1015`), consumed by the exact-a729 law.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherFadeDefines {
    /// `WEATHER_DISTANCE_CUTOFF` — hidden beyond this camera distance.
    pub distance_cutoff: f32,
    /// `WEATHER_DISTANCE_FADE_LENGTH` — length of the distant fade band.
    pub distance_fade_length: f32,
    /// `WEATHER_ZOOM_IN_CUTOFF` — most faded when zoomed in below this.
    pub zoom_in_cutoff: f32,
    /// `WEATHER_ZOOM_IN_FADE_LENGTH` — length of the zoom-in fade band.
    pub zoom_in_fade_length: f32,
    /// `WEATHER_ZOOM_IN_FADE_FACTOR` — residual factor at max zoom-in.
    pub zoom_in_fade_factor: f32,
    /// Minimum particle playback rate beyond the playback cutoff.
    pub playback_rate: f32,
    /// Camera distance at/after which the minimum playback rate applies.
    pub playback_rate_cutoff: f32,
    /// Length of the full-to-minimum playback transition.
    pub playback_rate_length: f32,
}

/// The default vanilla fade constants (`00_graphics.lua:1008-1012`). Present so
/// callers/tests have the shipped values without a resolver; the resolver path
/// [`weather_fade_defines_from_resolver`] is authoritative for mods.
pub const VANILLA_WEATHER_FADE_DEFINES: WeatherFadeDefines = WeatherFadeDefines {
    distance_cutoff: 1500.0,
    distance_fade_length: 400.0,
    zoom_in_cutoff: 358.0,
    zoom_in_fade_length: 220.0,
    zoom_in_fade_factor: 0.0,
    playback_rate: 0.15,
    playback_rate_cutoff: 500.0,
    playback_rate_length: 200.0,
};

/// Effective-file distance-fog constants consumed by
/// `standardfuncsgfx.fxh::CalculateDistanceFogFactor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherDistanceFogDefines {
    pub color: [f32; 3],
    pub begin: f32,
    pub end: f32,
    pub max: f32,
}

pub const VANILLA_WEATHER_DISTANCE_FOG: WeatherDistanceFogDefines = WeatherDistanceFogDefines {
    color: [0.12, 0.28, 0.6],
    begin: 1.0,
    end: 150.0,
    max: 0.35,
};

/// Exact role-specific alpha and particle playback-rate outputs.
///
/// `0x14166ec50` applies the near-zoom branch only to the fading/cloud entity.
/// The primary precipitation entity remains fully visible until the shared
/// far-distance fade applies to both roles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherVisibilityLaw {
    pub primary_alpha: f32,
    pub fading_alpha: f32,
    pub playback_rate: f32,
}

pub fn weather_visibility_law(
    camera_distance: f32,
    defines: &WeatherFadeDefines,
) -> Result<WeatherVisibilityLaw> {
    if !camera_distance.is_finite()
        || camera_distance < 0.0
        || ![
            defines.distance_cutoff,
            defines.distance_fade_length,
            defines.zoom_in_cutoff,
            defines.zoom_in_fade_length,
            defines.zoom_in_fade_factor,
            defines.playback_rate,
            defines.playback_rate_cutoff,
            defines.playback_rate_length,
        ]
        .iter()
        .all(|value| value.is_finite())
        || defines.distance_fade_length <= 0.0
        || defines.zoom_in_fade_length <= 0.0
        || defines.playback_rate_length <= 0.0
        || !(0.0..=1.0).contains(&defines.zoom_in_fade_factor)
        || !(0.0..=1.0).contains(&defines.playback_rate)
    {
        return Err("weather visibility inputs/defines are invalid".into());
    }
    let near_band_end = defines.zoom_in_cutoff + defines.zoom_in_fade_length;
    let far_band_start = defines.distance_cutoff - defines.distance_fade_length;
    if far_band_start < near_band_end {
        return Err("weather near/far visibility bands overlap".into());
    }
    let near_fading_alpha = if camera_distance < defines.zoom_in_cutoff {
        defines.zoom_in_fade_factor
    } else if camera_distance < near_band_end {
        let t = (camera_distance - defines.zoom_in_cutoff) / defines.zoom_in_fade_length;
        defines.zoom_in_fade_factor + t * (1.0 - defines.zoom_in_fade_factor)
    } else {
        1.0
    };
    let far_alpha = if camera_distance <= far_band_start {
        1.0
    } else if camera_distance <= defines.distance_cutoff {
        (defines.distance_cutoff - camera_distance) / defines.distance_fade_length
    } else {
        0.0
    };
    let full_rate_end = defines.playback_rate_cutoff - defines.playback_rate_length;
    let playback_rate = if camera_distance <= full_rate_end {
        1.0
    } else if camera_distance < defines.playback_rate_cutoff {
        let remaining =
            (defines.playback_rate_cutoff - camera_distance) / defines.playback_rate_length;
        remaining * (1.0 - defines.playback_rate) + defines.playback_rate
    } else {
        defines.playback_rate
    };
    Ok(WeatherVisibilityLaw {
        primary_alpha: far_alpha.clamp(0.0, 1.0),
        fading_alpha: (near_fading_alpha * far_alpha).clamp(0.0, 1.0),
        playback_rate: playback_rate.clamp(0.0, 1.0),
    })
}

/// Exact-a729 camera alpha multiplier for the primary precipitation role.
///
/// This compatibility helper cannot represent the separate fading/cloud role;
/// production callers consume [`WeatherVisibilityLaw`] directly.
#[deprecated(note = "use weather_visibility_law for role-specific primary/fading visibility")]
pub fn weather_camera_fade(camera_distance: f32, defines: &WeatherFadeDefines) -> Result<f32> {
    Ok(weather_visibility_law(camera_distance, defines)?.primary_alpha)
}

/// Exact `standardfuncsgfx.fxh` distance-fog factor.
pub fn weather_distance_fog_factor(
    camera_position: [f32; 3],
    world_position: [f32; 3],
    defines: &WeatherDistanceFogDefines,
) -> Result<f32> {
    if camera_position
        .iter()
        .chain(world_position.iter())
        .chain(defines.color.iter())
        .chain([&defines.begin, &defines.end, &defines.max])
        .any(|value| !value.is_finite())
        || ((defines.end * defines.end) - (defines.begin * defines.begin)).abs() <= f32::EPSILON
    {
        return Err("weather distance-fog inputs/defines are invalid".into());
    }
    let diff = [
        camera_position[0] - world_position[0],
        camera_position[1] - world_position[1],
        camera_position[2] - world_position[2],
    ];
    let squared_distance = diff.iter().map(|value| value * value).sum::<f32>();
    if squared_distance <= f32::EPSILON {
        return Err("weather distance-fog camera and world positions coincide".into());
    }
    let inverse_length = squared_distance.sqrt().recip();
    let directional = 1.0 - (diff[1] * inverse_length).abs();
    let begin_squared = defines.begin * defines.begin;
    let end_squared = defines.end * defines.end;
    let distance = ((squared_distance - begin_squared) / (end_squared - begin_squared))
        .min(defines.max)
        .clamp(0.0, 1.0);
    Ok(distance * directional)
}

/// Exact `lerp(vColor, FOG_COLOR, factor)` application.
pub fn apply_weather_distance_fog(
    color: [f32; 3],
    factor: f32,
    defines: &WeatherDistanceFogDefines,
) -> Result<[f32; 3]> {
    if color
        .iter()
        .chain(defines.color.iter())
        .any(|value| !value.is_finite())
        || !factor.is_finite()
        || !(0.0..=1.0).contains(&factor)
    {
        return Err("weather distance-fog color/factor is invalid".into());
    }
    Ok([
        color[0] + (defines.color[0] - color[0]) * factor,
        color[1] + (defines.color[1] - color[1]) * factor,
        color[2] + (defines.color[2] - color[2]) * factor,
    ])
}

pub fn weather_selection_word(seed_words: [u32; 2]) -> u32 {
    let mut value = seed_words[1].wrapping_add(seed_words[0].wrapping_mul(0xb529_7a4d));
    value = (value >> 8 ^ value).wrapping_add(0x68e3_1da4);
    value = (value.wrapping_mul(0x100) ^ value).wrapping_mul(0x1b56_c4e9);
    value = (value >> 8 ^ value).wrapping_mul(0x92d6_8ca2);
    (value >> 8 ^ value) & 0x7fff_ffff
}

pub fn weather_selected_candidate_index(
    selection_seed: &mut Option<[u32; 2]>,
    candidate_count: usize,
) -> Result<Option<usize>> {
    if candidate_count == 0 {
        return Ok(None);
    }
    let seed = selection_seed
        .as_mut()
        .ok_or("nonempty weather variant vector has no presentation-independent seed")?;
    let selection_word = weather_selection_word(*seed);
    seed[0] = seed[0].wrapping_add(1);
    Ok(Some(selection_word as usize % candidate_count))
}

// -------------------------------------------------------------------------
// Parsers (pure string -> struct; resolver wrappers below).
// -------------------------------------------------------------------------

/// Parse `common/weather.txt`'s `weather_effects` block.
pub fn weather_effects_from_source(source: &str) -> Result<WeatherEffectsTable> {
    let block = extract_braced_block(source, "weather_effects")
        .ok_or("common/weather.txt: missing weather_effects block")?;
    let mut entries = Vec::new();
    // Iterate the type keys inside the block; each is `<key> = { weather_effect
    // = { ... } ... }`.
    let mut rest: &str = &block;
    while let Some((key, inner, after)) = next_braced_entry(rest) {
        if let Some(ty) = WeatherType::from_key(&key) {
            for effect in each_braced_value(&inner, "weather_effect") {
                let entity = braced_field(&effect, "entity").ok_or_else(|| {
                    format!("weather_effects.{key}: weather_effect missing entity")
                })?;
                let fading_entity = braced_field(&effect, "fading_entity");
                let small = braced_field(&effect, "small")
                    .map(|v| v.trim() == "yes")
                    .unwrap_or(false);
                entries.push((
                    ty,
                    WeatherEffectBinding {
                        entity,
                        fading_entity,
                        small,
                    },
                ));
            }
        }
        rest = after;
    }
    Ok(WeatherEffectsTable { entries })
}

/// Resolve the entity name (`<name>_clouds_entity`) to its `particle` name via
/// `gfx/entities/weather_entities.asset` (`entity -> event.particle`).
pub fn entity_particle_name(entities_source: &str, entity_name: &str) -> Option<String> {
    entity_particle_names(entities_source, entity_name)
        .into_iter()
        .next()
}

/// Resolve every authored `event.particle` reference for one entity in event
/// order. Production uses the whole vector; the singular helper above remains
/// for compatibility with the older cloud-law inspection API.
pub fn entity_particle_names(entities_source: &str, entity_name: &str) -> Vec<String> {
    entity_default_state_events(entities_source, entity_name)
        .map(|state| {
            state
                .ordered_events
                .into_iter()
                .flat_map(|event| {
                    event
                        .ordered_particle_effects
                        .into_iter()
                        .map(|effect| effect.particle_token)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Count exact named entity definitions so callers can reject ambiguous
/// effective-VFS registration rather than accepting the first match.
pub fn entity_definition_count(entities_source: &str, entity_name: &str) -> usize {
    each_braced_value(entities_source, "entity")
        .filter(|block| braced_field(block, "name").as_deref() == Some(entity_name))
        .count()
}

/// One exact authored row in the selected weather entity state.
///
/// The index counts every `event = {}` row, including sound-only events.
/// Optional values preserve source omission; no parser or engine default is
/// manufactured here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherAuthoredEvent {
    pub authored_event_index: u32,
    pub ordered_particle_effects: Vec<WeatherAuthoredParticleEffect>,
    pub node_token: Option<String>,
    pub time_f32_bits: Option<u32>,
    pub keep_particle: Option<bool>,
    pub trigger_once: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherAuthoredParticleEffect {
    pub authored_effect_index: u32,
    pub particle_token: String,
}

/// Exact default state selected by one weather entity definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherEntityStateEvents {
    pub state_name: String,
    pub ordered_events: Vec<WeatherAuthoredEvent>,
}

/// Parse the unique entity's exact `default_state` and its ordered event rows.
///
/// Weather entities are created in their authored default state by the exact
/// weather producer. Returning an error for an absent/ambiguous state keeps
/// XMAP19/XMAP20 from guessing which event program the engine selected.
pub fn entity_default_state_events(
    entities_source: &str,
    entity_name: &str,
) -> Result<WeatherEntityStateEvents> {
    let entities = each_braced_value(entities_source, "entity")
        .filter(|block| braced_field(block, "name").as_deref() == Some(entity_name))
        .collect::<Vec<_>>();
    if entities.len() != 1 {
        return Err(format!(
            "weather entity {entity_name:?} resolved {} times",
            entities.len()
        )
        .into());
    }
    let entity = &entities[0];
    let state_name = braced_field(entity, "default_state")
        .ok_or_else(|| format!("weather entity {entity_name:?} has no default_state"))?;
    let states = each_braced_value(entity, "state")
        .filter(|block| braced_field(block, "name").as_deref() == Some(state_name.as_str()))
        .collect::<Vec<_>>();
    if states.len() != 1 {
        return Err(format!(
            "weather entity {entity_name:?} default state {state_name:?} resolved {} times",
            states.len()
        )
        .into());
    }
    let mut ordered_events = Vec::new();
    for (index, event) in each_braced_value(&states[0], "event").enumerate() {
        reject_duplicate_event_fields(&event, entity_name, index)?;
        let time_f32_bits = match braced_field(&event, "time") {
            Some(raw) => {
                let value = raw.parse::<f32>().map_err(|_| {
                    format!("weather entity {entity_name:?} event {index} has invalid time {raw:?}")
                })?;
                if !value.is_finite() || value < 0.0 {
                    return Err(format!(
                        "weather entity {entity_name:?} event {index} has invalid time {raw:?}"
                    )
                    .into());
                }
                Some(value.to_bits())
            }
            None => None,
        };
        ordered_events.push(WeatherAuthoredEvent {
            authored_event_index: u32::try_from(index)?,
            ordered_particle_effects: authored_particle_effects(&event, entity_name, index)?,
            node_token: braced_field(&event, "node"),
            time_f32_bits,
            keep_particle: optional_pdx_bool(
                braced_field(&event, "keep_particle"),
                entity_name,
                index,
                "keep_particle",
            )?,
            trigger_once: optional_pdx_bool(
                braced_field(&event, "trigger_once"),
                entity_name,
                index,
                "trigger_once",
            )?,
        });
    }
    if ordered_events.is_empty() {
        return Err(format!(
            "weather entity {entity_name:?} default state {state_name:?} has no events"
        )
        .into());
    }
    Ok(WeatherEntityStateEvents {
        state_name,
        ordered_events,
    })
}

fn reject_duplicate_event_fields(event: &str, entity_name: &str, event_index: usize) -> Result<()> {
    let mut seen = BTreeSet::new();
    for (key, _) in top_level_assignments(event)? {
        if matches!(
            key.as_str(),
            "node" | "time" | "keep_particle" | "trigger_once"
        ) && !seen.insert(key.clone())
        {
            return Err(format!(
                "weather entity {entity_name:?} event {event_index} repeats field {key:?}"
            )
            .into());
        }
    }
    Ok(())
}

fn authored_particle_effects(
    event: &str,
    entity_name: &str,
    event_index: usize,
) -> Result<Vec<WeatherAuthoredParticleEffect>> {
    let assignments = top_level_assignments(event)?;
    let mut effect_index = 0u32;
    let mut particles = Vec::new();
    for (key, value) in assignments {
        match key.as_str() {
            "particle" => {
                let particle_token = value.ok_or_else(|| {
                    format!(
                        "weather entity {entity_name:?} event {event_index} has a non-scalar particle effect"
                    )
                })?;
                if particle_token.trim().is_empty() {
                    return Err(format!(
                        "weather entity {entity_name:?} event {event_index} has a blank particle token"
                    )
                    .into());
                }
                particles.push(WeatherAuthoredParticleEffect {
                    authored_effect_index: effect_index,
                    particle_token,
                });
                effect_index = effect_index
                    .checked_add(1)
                    .ok_or("weather authored effect ordinal overflow")?;
            }
            "sound" => {
                if value.is_some() {
                    return Err(format!(
                        "weather entity {entity_name:?} event {event_index} has a non-braced sound effect"
                    )
                    .into());
                }
                effect_index = effect_index
                    .checked_add(1)
                    .ok_or("weather authored effect ordinal overflow")?;
            }
            _ => {}
        }
    }
    Ok(particles)
}

/// Return top-level `key = value` assignments while treating a braced value as
/// one value. This is deliberately smaller than a Clausewitz parser, but it
/// preserves exact source order, repeated keys, comments, strings, and nested
/// braces for the authored weather event grammar.
fn top_level_assignments(source: &str) -> Result<Vec<(String, Option<String>)>> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut out = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() {
            match bytes[index] {
                b'#' => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                byte if (byte as char).is_whitespace() => index += 1,
                _ => break,
            }
        }
        if index >= bytes.len() {
            break;
        }
        let key_start = index;
        while index < bytes.len()
            && ((bytes[index] as char).is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        if key_start == index {
            return Err("weather event has an invalid top-level token".into());
        }
        let key = source[key_start..index].to_string();
        while index < bytes.len() && (bytes[index] as char).is_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            return Err(format!("weather event field {key:?} is missing '='").into());
        }
        index += 1;
        while index < bytes.len() && (bytes[index] as char).is_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return Err(format!("weather event field {key:?} has no value").into());
        }
        let value = match bytes[index] {
            b'{' => {
                index = skip_balanced_value(source, index)?;
                None
            }
            b'"' => {
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += 1;
                }
                if index >= bytes.len() {
                    return Err(
                        format!("weather event field {key:?} has an unterminated string").into(),
                    );
                }
                let value = source[start..index].to_string();
                index += 1;
                Some(value)
            }
            _ => {
                let start = index;
                while index < bytes.len()
                    && !(bytes[index] as char).is_whitespace()
                    && bytes[index] != b'#'
                {
                    index += 1;
                }
                Some(source[start..index].to_string())
            }
        };
        out.push((key, value));
    }
    Ok(out)
}

fn skip_balanced_value(source: &str, open: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut index = open;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut comment = false;
    while index < bytes.len() {
        if comment {
            if bytes[index] == b'\n' {
                comment = false;
            }
            index += 1;
            continue;
        }
        if quoted {
            match bytes[index] {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => quoted = false,
                _ => escaped = false,
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'#' => comment = true,
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("weather event braced value is unbalanced")?;
                if depth == 0 {
                    return Ok(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Err("weather event braced value is unterminated".into())
}

fn optional_pdx_bool(
    raw: Option<String>,
    entity_name: &str,
    event_index: usize,
    field: &str,
) -> Result<Option<bool>> {
    match raw.as_deref() {
        None => Ok(None),
        Some("yes") => Ok(Some(true)),
        Some("no") => Ok(Some(false)),
        Some(value) => Err(format!(
            "weather entity {entity_name:?} event {event_index} has invalid {field} {value:?}"
        )
        .into()),
    }
}

/// Resolve a `pdxparticle` name (`<name>_clouds_particle`) to its `type`
/// (`<name>_clouds_file`) via `gfx/entities/particles.gfx`.
pub fn pdxparticle_type(particles_gfx_source: &str, particle_name: &str) -> Option<String> {
    for block in each_braced_value(particles_gfx_source, "pdxparticle") {
        if braced_field(&block, "name").as_deref() == Some(particle_name) {
            return braced_field(&block, "type");
        }
    }
    None
}

/// Parse one cloud particle `.asset` (`gfx/particles/environment/*_clouds.asset`)
/// into a [`CloudRenderLaw`]. Reads the *first* `subsystem` (the cloud layer;
/// the lightning `flash` subsystem is a secondary additive burst, reported
/// separately by [`asset_subsystem_count`]).
pub fn cloud_law_from_asset(
    asset_source: &str,
    particle_file_name: &str,
) -> Result<CloudRenderLaw> {
    let subsystem = first_braced_value(asset_source, "subsystem")
        .ok_or("cloud asset: missing subsystem block")?;
    let texture_block = first_braced_value(&subsystem, "texture")
        .ok_or("cloud asset: subsystem missing texture block")?;
    let texture =
        braced_field(&texture_block, "file").ok_or("cloud asset: texture missing file")?;
    let shader_effect =
        braced_field(&texture_block, "shader").ok_or("cloud asset: texture missing shader")?;
    let blend = ParticleBlend::from_effect(&shader_effect)
        .ok_or_else(|| format!("cloud asset: unknown shader effect {shader_effect}"))?;

    let color_block = first_braced_value(&subsystem, "color")
        .ok_or("cloud asset: subsystem missing color block")?;
    let color_rgb_255 = [
        parse_first_number(&braced_field(&color_block, "x").unwrap_or_default())
            .ok_or("cloud asset: color missing x")?,
        parse_first_number(&braced_field(&color_block, "y").unwrap_or_default())
            .ok_or("cloud asset: color missing y")?,
        parse_first_number(&braced_field(&color_block, "z").unwrap_or_default())
            .ok_or("cloud asset: color missing z")?,
    ];
    // alpha can be `alpha=60,smoke_fade` or `alpha={ 180,smoke_fade 20 }`.
    let alpha_raw =
        braced_field(&color_block, "alpha").ok_or("cloud asset: color missing alpha")?;
    let (base_alpha_255, alpha_anim) = parse_alpha_field(&alpha_raw);
    // The representative static frame samples the alpha curve at its peak
    // (e.g. snow_clouds' smoke_fade peaks at 0.914, NOT 1.0).
    let alpha_anim_peak = alpha_anim
        .as_deref()
        .map(|name| animation_curve_peak(asset_source, name))
        .unwrap_or(1.0);

    let size_mean = parse_first_number(&braced_field(&subsystem, "size").unwrap_or_default())
        .ok_or("cloud asset: subsystem missing size")?;
    let life_mean = parse_first_number(&braced_field(&subsystem, "life").unwrap_or_default())
        .ok_or("cloud asset: subsystem missing life")?;
    // `sort` is optional (present as `sort="depth"` on the lightning cloud
    // layers). A bareword/quoted value; absence means no explicit depth sort.
    let sort = braced_field(&subsystem, "sort").filter(|s| !s.is_empty());

    Ok(CloudRenderLaw {
        particle_file: particle_file_name.to_string(),
        texture,
        shader_effect,
        blend,
        color_rgb_255,
        base_alpha_255,
        alpha_anim,
        alpha_anim_peak,
        size_mean,
        life_mean,
        sort,
    })
}

/// The number of `subsystem` blocks in a cloud `.asset` (2 for the lightning
/// variants: the cloud layer plus the additive `flash`).
pub fn asset_subsystem_count(asset_source: &str) -> usize {
    each_braced_value(asset_source, "subsystem").count()
}

/// Evaluate a named animation curve from a cloud `.asset` at a life fraction
/// `t` in [0,1]. Curves are piecewise-linear `x0 y0 x1 y1 ...` control points
/// (`smoke_fade`/`cloud_fade`), `op="MUL"` on the alpha. Returns the multiplier
/// (defaults to 1.0 if the named curve is absent). The engine samples this by
/// each particle's *own* age; a static overlay uses the mid-life peak by
/// passing the curve's max, or a specific `t`.
pub fn evaluate_animation_curve(asset_source: &str, anim_name: &str, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    for block in each_braced_value(asset_source, "animation") {
        if braced_field(&block, "name").as_deref() == Some(anim_name) {
            if let Some(curve) = first_braced_value(&block, "curve") {
                let nums: Vec<f32> = curve
                    .split_whitespace()
                    .filter_map(|tok| tok.parse::<f32>().ok())
                    .collect();
                return sample_piecewise_linear(&nums, t);
            }
        }
    }
    1.0
}

/// The peak (maximum) value of a named animation curve — the brightest phase of
/// the particle's life, used to draw a representative static overlay.
pub fn animation_curve_peak(asset_source: &str, anim_name: &str) -> f32 {
    for block in each_braced_value(asset_source, "animation") {
        if braced_field(&block, "name").as_deref() == Some(anim_name) {
            if let Some(curve) = first_braced_value(&block, "curve") {
                let ys: Vec<f32> = curve
                    .split_whitespace()
                    .filter_map(|tok| tok.parse::<f32>().ok())
                    .enumerate()
                    .filter_map(|(i, v)| (i % 2 == 1).then_some(v))
                    .collect();
                return ys.into_iter().fold(0.0f32, f32::max);
            }
        }
    }
    1.0
}

/// Decode the shared `map/weatherpositions.txt` rows (parsed by
/// [`crate::map_static_data::weather_positions_from_source`]) into
/// size-tagged [`WeatherEmitter`]s. Fails closed on an unrecognised size tag.
pub fn weather_emitters_from_source(source: &str) -> Result<WeatherEmitterTable> {
    let rows = crate::map_static_data::weather_positions_from_source(source)?;
    decode_weather_positions(&rows)
}

fn decode_weather_positions(
    rows: &[crate::map_static_data::WeatherPosition],
) -> Result<WeatherEmitterTable> {
    let mut emitters = Vec::with_capacity(rows.len());
    for row in rows {
        let size = WeatherPositionSize::from_tag(&row.size).ok_or_else(|| {
            format!(
                "map/weatherpositions.txt: region {} has unknown size tag {:?}",
                row.id, row.size
            )
        })?;
        emitters.push(WeatherEmitter {
            region_id: row.id,
            position: row.pos,
            size,
        });
    }
    Ok(WeatherEmitterTable { emitters })
}

/// Parse the `WEATHER_*` fade constants out of `00_graphics.lua` source.
pub fn weather_fade_defines_from_source(source: &str) -> Result<WeatherFadeDefines> {
    let get = |name: &str| -> Result<f32> {
        lua_number(source, name).ok_or_else(|| format!("00_graphics.lua: missing {name}").into())
    };
    Ok(WeatherFadeDefines {
        distance_cutoff: get("WEATHER_DISTANCE_CUTOFF")?,
        distance_fade_length: get("WEATHER_DISTANCE_FADE_LENGTH")?,
        zoom_in_cutoff: get("WEATHER_ZOOM_IN_CUTOFF")?,
        zoom_in_fade_length: get("WEATHER_ZOOM_IN_FADE_LENGTH")?,
        zoom_in_fade_factor: get("WEATHER_ZOOM_IN_FADE_FACTOR")?,
        playback_rate: get("WEATHER_PLAYBACK_RATE")?,
        playback_rate_cutoff: get("WEATHER_PLAYBACK_RATE_CUTOFF")?,
        playback_rate_length: get("WEATHER_PLAYBACK_RATE_LENGTH")?,
    })
}

/// Parse the effective compile-time distance-fog constants.
pub fn weather_distance_fog_defines_from_source(source: &str) -> Result<WeatherDistanceFogDefines> {
    let color = hlsl_static_numbers(source, "FOG_COLOR")
        .filter(|values| values.len() == 3)
        .ok_or("constants.fxh: missing three-component FOG_COLOR")?;
    let scalar = |name: &str| -> Result<f32> {
        hlsl_static_numbers(source, name)
            .and_then(|values| values.first().copied())
            .ok_or_else(|| format!("constants.fxh: missing {name}").into())
    };
    let defines = WeatherDistanceFogDefines {
        color: [color[0], color[1], color[2]],
        begin: scalar("FOG_BEGIN")?,
        end: scalar("FOG_END")?,
        max: scalar("FOG_MAX")?,
    };
    weather_distance_fog_factor([0.0, defines.end + 1.0, 0.0], [0.0, 0.0, 1.0], &defines)?;
    Ok(defines)
}

/// Count exact named `pdxparticle` registrations.
pub fn pdxparticle_registration_count(particles_gfx_source: &str, particle_name: &str) -> usize {
    each_braced_value(particles_gfx_source, "pdxparticle")
        .filter(|block| braced_field(block, "name").as_deref() == Some(particle_name))
        .count()
}

/// Count exact named `particle` definitions in one effective asset source.
pub fn particle_asset_definition_count(asset_source: &str, particle_type: &str) -> usize {
    each_braced_value(asset_source, "particle")
        .filter(|block| braced_field(block, "name").as_deref() == Some(particle_type))
        .count()
}

/// True when an effective particle asset defines the requested
/// `pdxparticle.type` name.
pub fn particle_asset_defines(asset_source: &str, particle_type: &str) -> bool {
    particle_asset_definition_count(asset_source, particle_type) != 0
}

/// Every texture file referenced by one named particle definition, in authored
/// subsystem order with duplicates removed after their first occurrence.
pub fn particle_asset_texture_files(asset_source: &str, particle_type: &str) -> Vec<String> {
    for block in each_braced_value(asset_source, "particle") {
        if braced_field(&block, "name").as_deref() == Some(particle_type) {
            let mut out = Vec::new();
            for texture in each_braced_value(&block, "texture") {
                if let Some(file) = braced_field(&texture, "file") {
                    if !out.contains(&file) {
                        out.push(file);
                    }
                }
            }
            return out;
        }
    }
    Vec::new()
}

/// Every shader Effect referenced by one named particle definition, in
/// authored subsystem order with duplicates removed after first occurrence.
pub fn particle_asset_shader_effects(asset_source: &str, particle_type: &str) -> Vec<String> {
    for block in each_braced_value(asset_source, "particle") {
        if braced_field(&block, "name").as_deref() == Some(particle_type) {
            let mut out = Vec::new();
            for texture in each_braced_value(&block, "texture") {
                if let Some(effect) = braced_field(&texture, "shader") {
                    if !out.contains(&effect) {
                        out.push(effect);
                    }
                }
            }
            return out;
        }
    }
    Vec::new()
}

// -------------------------------------------------------------------------
// Resolver wrappers (mod-aware reads; source of truth for merged pipeline).
// -------------------------------------------------------------------------

/// Load + parse `common/weather.txt` via the mod-aware resolver.
pub fn weather_effects_from_resolver(resolver: &GameFileResolver) -> Result<WeatherEffectsTable> {
    let source = resolve_to_string(resolver, WEATHER_EFFECTS_SOURCE, "weather.txt")?;
    weather_effects_from_source(&source)
}

/// Load + decode `map/weatherpositions.txt` via the mod-aware resolver, reusing
/// the shared [`crate::map_static_data`] parser.
pub fn weather_emitters_from_resolver(resolver: &GameFileResolver) -> Result<WeatherEmitterTable> {
    let rows = crate::map_static_data::weather_positions_from_resolver(resolver)?;
    decode_weather_positions(&rows)
}

/// Load + parse the `WEATHER_*` fade constants via the mod-aware resolver.
pub fn weather_fade_defines_from_resolver(
    resolver: &GameFileResolver,
) -> Result<WeatherFadeDefines> {
    let source = resolve_to_string(resolver, GRAPHICS_DEFINES_SOURCE, "00_graphics.lua")?;
    weather_fade_defines_from_source(&source)
}

/// Resolve the full cloud (`fading_entity`) render law for a `(type, size)` via
/// the whole shipped binding chain: `weather.txt` -> `weather_entities.asset`
/// -> `particles.gfx` -> `*_clouds.asset`. Returns `Ok(None)` for a type with
/// no cloud layer (`no_phenomenon`, `sandstorm`).
pub fn cloud_law_from_resolver(
    resolver: &GameFileResolver,
    weather: WeatherType,
    size: WeatherPositionSize,
) -> Result<Option<CloudRenderLaw>> {
    let effects = weather_effects_from_resolver(resolver)?;
    let Some(binding) = effects.binding(weather, size) else {
        return Ok(None);
    };
    let Some(fading_entity) = binding.fading_entity.clone() else {
        return Ok(None);
    };

    let entities_source =
        resolve_to_string(resolver, WEATHER_ENTITIES_SOURCE, "weather_entities.asset")?;
    let particle_name =
        entity_particle_name(&entities_source, &fading_entity).ok_or_else(|| {
            format!("weather_entities.asset: entity {fading_entity} has no particle event")
        })?;

    let particles_gfx = resolve_to_string(resolver, PARTICLES_GFX_SOURCE, "particles.gfx")?;
    let particle_file = pdxparticle_type(&particles_gfx, &particle_name)
        .ok_or_else(|| format!("particles.gfx: pdxparticle {particle_name} not found"))?;

    // The .asset filename is <particle_file without the _file suffix>.asset in
    // gfx/particles/environment/. E.g. rain_clouds_file -> rain_clouds.asset.
    let asset_stem = particle_file
        .strip_suffix("_file")
        .unwrap_or(&particle_file);
    let asset_rel = format!("gfx/particles/environment/{asset_stem}.asset");
    let asset_source = resolve_to_string(resolver, &asset_rel, "cloud asset")?;
    let law = cloud_law_from_asset(&asset_source, &particle_file)?;
    Ok(Some(law))
}

/// The cloud sprite texture (DXT5/BC3 DDS) for a resolved [`CloudRenderLaw`],
/// decoded to straight RGBA8 (`width, height, rgba`). `RgbaImage` is
/// crate-private, so this returns plain types usable from a `bin/` binary.
pub fn cloud_texture_rgba_from_resolver(
    resolver: &GameFileResolver,
    law: &CloudRenderLaw,
) -> Result<(usize, usize, Vec<u8>)> {
    let path = resolver.resolve_required(&law.texture, "cloud sprite")?;
    let img: RgbaImage = load_dds_rgba(&path)?;
    Ok((img.width, img.height, img.pixels))
}

/// The map world plane extent in x/z, matching SNOW1's documented
/// `provinces.bmp` resolution (`[0,5632] x [0,2048]`), used to place the
/// `map/weatherpositions.txt` emitters onto a top-down overlay.
pub const MAP_PLANE_X: f32 = 5632.0;
/// See [`MAP_PLANE_X`].
pub const MAP_PLANE_Z: f32 = 2048.0;

/// Legacy diagnostic top-down weather overlay: RGBA8
/// premultiplied-over-nothing atop a transparent map plane. It draws every
/// resolved cloud emitter at its
/// `map/weatherpositions.txt` `(x,z)` location, sprite-splatted at the asset
/// `size_mean` (map units -> pixels), tinted by `ToLinear(color)` and composited
/// with the asset's `ParticleBlend`.
///
/// This is not a production rendering or promotion authority. The shipped
/// `particle.shader` draws camera-facing billboards through the viewer-owned
/// XMAP03 `ViewProjectionMatrix`, and XMAP20 owns the full particle simulation.
/// This diagnostic is a deliberately *orthographic top-down* projection so
/// some file-derived
/// parameters (position, size, color, blend, alpha curve) can be verified
/// without inventing a camera. The per-particle `smoke_fade` phase is sampled at
/// the curve peak (the representative brightest frame); the live per-particle
/// ages are runtime state.
pub struct WeatherOverlay {
    /// Overlay width in pixels.
    pub width: usize,
    /// Overlay height in pixels.
    pub height: usize,
    /// Straight-alpha RGBA8, row-major, top-left origin.
    pub rgba: Vec<u8>,
}

/// Render a top-down [`WeatherOverlay`] for one weather type applied to every
/// region that has a `map/weatherpositions.txt` emitter (or, if
/// `only_regions` is `Some`, just those). `width`/`height` set the output raster;
/// the map plane `[0,MAP_PLANE_X] x [0,MAP_PLANE_Z]` maps linearly to it.
#[deprecated(
    note = "diagnostic mean/peak preview only; production weather is XMAP19 content plus XMAP20 simulation"
)]
pub fn render_weather_overlay(
    resolver: &GameFileResolver,
    weather: WeatherType,
    width: usize,
    height: usize,
    only_regions: Option<&[u32]>,
) -> Result<WeatherOverlay> {
    let positions = weather_emitters_from_resolver(resolver)?;

    // Resolve the two cloud laws (big/small) + their textures once.
    let mut resolved: std::collections::HashMap<
        WeatherPositionSize,
        (CloudRenderLaw, (usize, usize, Vec<u8>)),
    > = std::collections::HashMap::new();
    for size in [WeatherPositionSize::Big, WeatherPositionSize::Small] {
        if let Some(law) = cloud_law_from_resolver(resolver, weather, size)? {
            let tex = cloud_texture_rgba_from_resolver(resolver, &law)?;
            resolved.insert(size, (law, tex));
        }
    }

    // Accumulate as linear-ish straight RGBA in f32 over a transparent plane.
    let mut acc = vec![[0.0f32; 4]; width * height];

    for pos in &positions.emitters {
        if let Some(regions) = only_regions {
            if !regions.contains(&pos.region_id) {
                continue;
            }
        }
        let Some((law, tex)) = resolved.get(&pos.size) else {
            continue;
        };
        // Peak phase of the alpha curve is the representative frame — the
        // curve's actual maximum, computed from the asset when the law was
        // resolved (the shipped snow_clouds smoke_fade peaks at 0.914, not
        // 1.0).
        let anim_mul = law.alpha_anim_peak;
        let vcolor = law.linear_vertex_color(anim_mul);
        splat_sprite(
            &mut acc,
            width,
            height,
            pos.position[0],
            pos.position[2],
            law.size_mean,
            tex,
            vcolor,
            law.blend,
        );
    }

    // Convert accumulated linear color back to gamma8 for a viewable PNG.
    let mut rgba = vec![0u8; width * height * 4];
    for (i, px) in acc.iter().enumerate() {
        let g = |c: f32| (c.clamp(0.0, 1.0).powf(1.0 / TO_LINEAR_GAMMA) * 255.0 + 0.5) as u8;
        rgba[i * 4] = g(px[0]);
        rgba[i * 4 + 1] = g(px[1]);
        rgba[i * 4 + 2] = g(px[2]);
        rgba[i * 4 + 3] = (px[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }

    Ok(WeatherOverlay {
        width,
        height,
        rgba,
    })
}

/// Splat one sprite (its RGBA texture * linear vertex color) centred at world
/// `(wx, wz)` with world `size` (a square), onto the accumulation buffer using
/// the given blend. The sprite's own texel alpha multiplies the vertex alpha
/// (matching `tex2D(DiffuseMap,uv) * vColor` where the texture supplies alpha).
#[allow(clippy::too_many_arguments)]
fn splat_sprite(
    acc: &mut [[f32; 4]],
    width: usize,
    height: usize,
    wx: f32,
    wz: f32,
    size: f32,
    tex: &(usize, usize, Vec<u8>),
    vcolor: [f32; 4],
    blend: ParticleBlend,
) {
    let (tw, th, ref data) = *tex;
    if tw == 0 || th == 0 {
        return;
    }
    // World -> pixel scale.
    let px_per_wx = width as f32 / MAP_PLANE_X;
    let pz_per_wz = height as f32 / MAP_PLANE_Z;
    let half_w = size * 0.5 * px_per_wx;
    let half_h = size * 0.5 * pz_per_wz;
    let cx = wx * px_per_wx;
    let cy = wz * pz_per_wz;
    let x0 = (cx - half_w).floor().max(0.0) as usize;
    let x1 = ((cx + half_w).ceil() as i64).clamp(0, width as i64) as usize;
    let y0 = (cy - half_h).floor().max(0.0) as usize;
    let y1 = ((cy + half_h).ceil() as i64).clamp(0, height as i64) as usize;

    for py in y0..y1 {
        for px in x0..x1 {
            // Sprite UV in [0,1].
            let u = ((px as f32 + 0.5) - (cx - half_w)) / (2.0 * half_w).max(f32::EPSILON);
            let v = ((py as f32 + 0.5) - (cy - half_h)) / (2.0 * half_h).max(f32::EPSILON);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let tx = ((u * tw as f32) as usize).min(tw - 1);
            let ty = ((v * th as f32) as usize).min(th - 1);
            let toff = (ty * tw + tx) * 4;
            // Texture is straight sRGB8 RGBA; linearize RGB, keep alpha.
            let tr = (data[toff] as f32 / 255.0).powf(TO_LINEAR_GAMMA);
            let tg = (data[toff + 1] as f32 / 255.0).powf(TO_LINEAR_GAMMA);
            let tb = (data[toff + 2] as f32 / 255.0).powf(TO_LINEAR_GAMMA);
            let ta = data[toff + 3] as f32 / 255.0;
            let src = [
                tr * vcolor[0],
                tg * vcolor[1],
                tb * vcolor[2],
                ta * vcolor[3],
            ];
            let idx = py * width + px;
            let dst = [acc[idx][0], acc[idx][1], acc[idx][2]];
            let out = blend.blend_over(src, dst);
            // Track coverage alpha for the PNG (max of contributions).
            acc[idx] = [out[0], out[1], out[2], acc[idx][3].max(src[3])];
        }
    }
}

// -------------------------------------------------------------------------
// Small parsing helpers (self-contained; no external PDX parser dependency).
// -------------------------------------------------------------------------

fn resolve_to_string(resolver: &GameFileResolver, rel: &str, label: &str) -> Result<String> {
    let path = resolver.resolve_required(rel, label)?;
    read_path_to_string(&path, rel)
}

fn read_path_to_string(path: &Path, rel: &str) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|err| format!("failed to read {rel}: {err}"))?;
    // PDX text is Windows-1252/ASCII; lossy UTF-8 is safe for the tokens we read.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// First whitespace/comma-delimited number in a value string.
fn parse_first_number(raw: &str) -> Option<f32> {
    raw.split(|c: char| c.is_whitespace() || c == ',' || c == '{' || c == '}')
        .filter(|s| !s.is_empty())
        .find_map(|tok| tok.parse::<f32>().ok())
}

/// Parse an alpha field that may be `60,smoke_fade` or `{ 180,smoke_fade 20 }`:
/// returns `(base_alpha_255, Some(anim_name))`.
fn parse_alpha_field(raw: &str) -> (f32, Option<String>) {
    let cleaned = raw.replace(['{', '}'], " ");
    let base = parse_first_number(&cleaned).unwrap_or(0.0);
    let anim = cleaned
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .find(|tok| tok.parse::<f32>().is_err())
        .map(|s| s.to_string());
    (base, anim)
}

/// Sample a piecewise-linear curve encoded as flat `x0 y0 x1 y1 ...` at `t`.
fn sample_piecewise_linear(flat: &[f32], t: f32) -> f32 {
    if flat.len() < 2 {
        return 1.0;
    }
    let n = flat.len() / 2;
    // Before first / after last: clamp to endpoint y.
    if t <= flat[0] {
        return flat[1];
    }
    if t >= flat[(n - 1) * 2] {
        return flat[(n - 1) * 2 + 1];
    }
    for i in 0..n - 1 {
        let x0 = flat[i * 2];
        let y0 = flat[i * 2 + 1];
        let x1 = flat[(i + 1) * 2];
        let y1 = flat[(i + 1) * 2 + 1];
        if t >= x0 && t <= x1 {
            let span = (x1 - x0).max(f32::EPSILON);
            let f = (t - x0) / span;
            return y0 + (y1 - y0) * f;
        }
    }
    flat[(n - 1) * 2 + 1]
}

/// A Lua `NAME = <number>,` reader (tolerant of trailing comment/whitespace).
fn lua_number(source: &str, name: &str) -> Option<f32> {
    for line in source.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(name) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let value = rest
                    .trim()
                    .trim_end_matches(',')
                    .split(|c: char| c.is_whitespace() || c == ',' || c == '-' && false)
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',');
                // Capture up to the first comment/space.
                let token: String = value
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
                    .collect();
                if let Ok(v) = token.parse::<f32>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn hlsl_static_numbers(source: &str, name: &str) -> Option<Vec<f32>> {
    let matches = source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("//")
                || !line
                    .split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                    .any(|token| token == name)
            {
                return None;
            }
            let value = line.split_once('=')?.1.split("//").next()?;
            let numbers = value
                .split(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '(' | ')' | ',' | ';' | '{' | '}')
                })
                .filter_map(|token| token.trim_end_matches('f').parse::<f32>().ok())
                .collect::<Vec<_>>();
            (!numbers.is_empty()).then_some(numbers)
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

/// Extract the `{...}` body of `key = { ... }` (balanced braces).
fn extract_braced_block(source: &str, key: &str) -> Option<String> {
    let start = find_key_brace(source, key)?;
    balanced_body(source, start)
}

/// Find the byte index of the `{` that opens `key`'s block (allowing `=`/spaces
/// between the key and the brace).
fn find_key_brace(source: &str, key: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = source[search_from..].find(key) {
        let idx = search_from + rel;
        // Ensure it's a whole token (preceded by non-word char or start).
        let ok_before = idx == 0
            || !source[..idx]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after = idx + key.len();
        if ok_before {
            // Skip whitespace/`=` to the brace.
            let mut j = after;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
            }
            if j < bytes.len() && bytes[j] == b'{' {
                return Some(j);
            }
        }
        search_from = after;
    }
    None
}

/// Given the index of an opening `{`, return the body between it and its
/// matching `}`.
fn balanced_body(source: &str, brace_index: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if bytes.get(brace_index) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = brace_index;
    let body_start = brace_index + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(source[body_start..i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The first `key = { ... }` body inside `source`.
fn first_braced_value(source: &str, key: &str) -> Option<String> {
    extract_braced_block(source, key)
}

/// Iterator over every `key = { ... }` body in `source` (top-level scan, brace
/// balanced). Non-recursive: repeated keys at any depth are all yielded.
fn each_braced_value<'a>(source: &'a str, key: &'a str) -> impl Iterator<Item = String> + 'a {
    let mut from = 0usize;
    std::iter::from_fn(move || {
        while from < source.len() {
            let brace = find_key_brace(&source[from..], key)? + from;
            let body = balanced_body(source, brace)?;
            // Advance past this block's closing brace.
            let consumed = brace + body.len() + 2; // { body }
            from = consumed.min(source.len());
            return Some(body);
        }
        None
    })
}

/// The next `key = { ... }` at the top of a block, returning
/// `(key, body, remainder_after_block)`. Used to walk `weather_effects`'s type
/// entries in order.
fn next_braced_entry(source: &str) -> Option<(String, String, &str)> {
    // Find the first `<ident> = {`.
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let cc = bytes[i] as char;
                if cc.is_alphanumeric() || cc == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let ident = &source[start..i];
            // Skip ws/`=`.
            let mut j = i;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'{' {
                    if let Some(body) = balanced_body(source, j) {
                        let after = j + body.len() + 2;
                        return Some((ident.to_string(), body, &source[after.min(source.len())..]));
                    }
                }
            }
            // Not a block; continue scanning after the ident.
        } else {
            i += 1;
        }
    }
    None
}

/// Read a scalar field `key = value` (up to end of line / next token) from a
/// block body. Handles `key = "quoted"`, `key = bareword`, `key = 1.0`,
/// `key = { ... }` (returns the braced body's trimmed content).
fn braced_field(block: &str, key: &str) -> Option<String> {
    let bytes = block.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = block[search_from..].find(key) {
        let idx = search_from + rel;
        let ok_before = idx == 0
            || !block[..idx]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after = idx + key.len();
        let ok_after = block[after..]
            .chars()
            .next()
            .map(|c| c.is_whitespace() || c == '=')
            .unwrap_or(false);
        if ok_before && ok_after {
            let mut j = after;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
                if j >= bytes.len() {
                    return None;
                }
                if bytes[j] == b'{' {
                    return balanced_body(block, j).map(|s| s.trim().to_string());
                }
                if bytes[j] == b'"' {
                    let start = j + 1;
                    let end = block[start..].find('"')? + start;
                    return Some(block[start..end].to_string());
                }
                // Bareword / number: to end of whitespace.
                let start = j;
                let mut k = j;
                while k < bytes.len() && !(bytes[k] as char).is_whitespace() {
                    k += 1;
                }
                return Some(block[start..k].to_string());
            }
        }
        search_from = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn from_recording_kind_accepts_only_engine_keys() {
        assert_eq!(
            WeatherType::from_recording_kind("rain_light"),
            Some(WeatherType::RainLight)
        );
        assert_eq!(
            WeatherType::from_recording_kind("no_phenomenon"),
            Some(WeatherType::NoPhenomenon)
        );
        assert_eq!(WeatherType::from_recording_kind("rain"), None);
        assert_eq!(WeatherType::from_recording_kind("storm"), None);
        assert_eq!(WeatherType::from_recording_kind("volcano"), None);
    }

    // --- Real vanilla-file fixtures (verbatim excerpts from the shipped game). ---

    const WEATHER_TXT: &str = r#"
weather_effects = {
  rain_light = {
  	weather_effect = { entity = rain_entity fading_entity = rain_clouds_entity }
  	weather_effect = { entity = rain_small_entity fading_entity = rain_small_clouds_entity small = yes }
  }
  rain_heavy = {
  	weather_effect = { entity = lightning_storm_entity fading_entity = lightning_storm_clouds_entity }
  	weather_effect = { entity = lightning_storm_small_entity fading_entity = lightning_storm_small_clouds_entity small = yes }
  }
  snow = {
  	weather_effect = { entity = snow_entity fading_entity = snow_clouds_entity }
  	weather_effect = { entity = snow_small_entity fading_entity = snow_small_clouds_entity small = yes }
  }
  blizzard = {
  	weather_effect = { entity = snow_storm_entity fading_entity = snow_storm_clouds_entity }
  	weather_effect = { entity = snow_storm_small_entity fading_entity = snow_storm_small_clouds_entity small = yes }
  }
  sandstorm = {
  	weather_effect = { entity = sandstorm_entity }
  }
}
"#;

    const WEATHER_ENTITIES: &str = r#"
entity = {
	name = "rain_clouds_entity"
	cull_radius = 10
	default_state = "idle"
	state = { name = "idle" state_time = 2 looping = no
		event = { time = 0 particle = "rain_clouds_particle" keep_particle = yes }
	}
}
entity = {
	name = "snow_clouds_entity"
	cull_radius = 10
	default_state = "idle"
	state = { name = "idle" state_time = 2 looping = no
		event = { time = 0 particle = "snow_clouds_particle" keep_particle = yes }
	}
}
"#;

    const PARTICLES_GFX: &str = r#"
	pdxparticle = {
		name = "rain_clouds_particle"
		type = "rain_clouds_file"
		scale = 1
	}
	pdxparticle = {
		name = "snow_clouds_particle"
		type = "snow_clouds_file"
		scale = 1
	}
"#;

    // Verbatim rain_clouds.asset (ParticleAdditive, color 110/120/150, alpha 180).
    const RAIN_CLOUDS_ASSET: &str = r#"particle={
	name="rain_clouds_file"
	subsystem={
		name="clouds"
		max_amount=25
		emitter_type="sphere"
		billboard=yes
		texture={
			file="gfx/particles/cloud_6.dds"
			x=1
			y=1
			shader="ParticleAdditive"
		}
		color={
			x=110.000
			y=120.000
			z=150.000
			alpha={ 180.000,smoke_fade 20.000 }
		}
		life={ 2.600 0.700 }
		emission=8.000
		size={ 60.500 20.000 }
	}
	animation={
		name="smoke_fade"
		curve={
			0.000 0.100 0.300 0.100 0.500 1.000 0.800 1.000 1.000 0.000
		}
		op="MUL"
		time="life"
	}
}
"#;

    // Verbatim snow_clouds.asset (ParticleAlphaBlend, color 200/200/200, alpha 60).
    const SNOW_CLOUDS_ASSET: &str = r#"particle={
	name="snow_clouds_file"
	subsystem={
		name="clouds"
		max_amount=20
		texture={
			file="gfx/particles/snow_cloud.dds"
			shader="ParticleAlphaBlend"
		}
		color={
			x=200.000
			y=200.000
			z=200.000
			alpha=60.000,smoke_fade
		}
		life={ 1.600 0.700 }
		size={ 60.500 10.000 }
	}
	animation={
		name="smoke_fade"
		curve={
			0.000 0.000 0.321 0.750 0.635 0.914 1.000 0.000
		}
		op="MUL"
		time="life"
	}
}
"#;

    // Verbatim lightning_storm_clouds.asset excerpt: two subsystems.
    const LIGHTNING_CLOUDS_ASSET: &str = r#"particle={
	name="lightning_storm_clouds_file"
	subsystem={
		name="cloud"
		sort="depth"
		texture={ file="gfx/particles/cloud_6.dds" shader="ParticleAlphaBlend" }
		color={ x=160.000 y=160.000 z=160.000 alpha={ 150.000,cloud_fade 10.000 } }
		life={ 10.000 2.500 }
		size={ 60.500 15.500 }
	}
	subsystem={
		name="flash"
		texture={ file="gfx/particles/cloud_6.dds" shader="ParticleAdditive" }
		color={ x=220.000 y=220.000 z=255.000 alpha={ 140.000,smoke_fade 20.000 } }
		life={ 0.700 0.100 }
		size={ 50.000 20.000 }
	}
	animation={ name="cloud_fade" curve={ 0.000 0.100 0.100 1.000 0.800 1.000 1.000 0.000 } op="MUL" time="life" }
}
"#;

    const GRAPHICS_LUA: &str = r#"
	WEATHER_DISTANCE_CUTOFF = 1500, -- At what distance weather effects are hidden
	WEATHER_DISTANCE_FADE_LENGTH = 400, -- How far the fade out distance should be
	WEATHER_ZOOM_IN_CUTOFF = 358, -- At what distance weather effects are faded out the most when zooming in
	WEATHER_ZOOM_IN_FADE_LENGTH = 220, -- How far the zoom in fade out distance should be
	WEATHER_ZOOM_IN_FADE_FACTOR = 0.0, -- How much the weather effects should fade out when maximum zoomed in
	WEATHER_PLAYBACK_RATE = 0.15,
	WEATHER_PLAYBACK_RATE_CUTOFF = 500,
	WEATHER_PLAYBACK_RATE_LENGTH = 200,
"#;

    const WEATHER_POSITIONS: &str = "1;2781.24;9.90;1571.49;small\n1;2746.16;10.10;1548.63;small\n7;2941.69;10.50;1516.84;big\n";

    fn path_from_rel(root: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|part| !part.is_empty())
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = path_from_rel(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    #[test]
    fn weather_type_keys_round_trip() {
        for ty in [
            WeatherType::NoPhenomenon,
            WeatherType::RainLight,
            WeatherType::RainHeavy,
            WeatherType::Snow,
            WeatherType::Blizzard,
            WeatherType::Sandstorm,
        ] {
            assert_eq!(WeatherType::from_key(ty.key()), Some(ty));
        }
        assert_eq!(WeatherType::from_key("arctic_water"), None);
        assert_eq!(WeatherType::from_key("mud"), None);
    }

    #[test]
    fn parses_weather_effects_bindings() {
        let table = weather_effects_from_source(WEATHER_TXT).expect("parse");
        // rain_light big -> rain_clouds_entity; small -> rain_small_clouds_entity.
        let big = table
            .binding(WeatherType::RainLight, WeatherPositionSize::Big)
            .expect("rain big");
        assert_eq!(big.entity, "rain_entity");
        assert_eq!(big.fading_entity.as_deref(), Some("rain_clouds_entity"));
        assert!(!big.small);

        let small = table
            .binding(WeatherType::RainLight, WeatherPositionSize::Small)
            .expect("rain small");
        assert_eq!(small.entity, "rain_small_entity");
        assert_eq!(
            small.fading_entity.as_deref(),
            Some("rain_small_clouds_entity")
        );
        assert!(small.small);

        // sandstorm has one row, no fading_entity; small falls back to it.
        let sand = table
            .binding(WeatherType::Sandstorm, WeatherPositionSize::Small)
            .expect("sandstorm");
        assert_eq!(sand.entity, "sandstorm_entity");
        assert_eq!(sand.fading_entity, None);
    }

    #[test]
    fn measured_empty_weather_effects_table_is_valid() {
        let table =
            weather_effects_from_source("weather_effects = { }").expect("authored empty table");
        assert!(table.rows().is_empty());
        assert!(table
            .bindings(WeatherType::RainLight, WeatherPositionSize::Big)
            .is_empty());
    }

    #[test]
    fn resolves_entity_and_pdxparticle_chain() {
        assert_eq!(
            entity_particle_name(WEATHER_ENTITIES, "rain_clouds_entity").as_deref(),
            Some("rain_clouds_particle")
        );
        assert_eq!(
            pdxparticle_type(PARTICLES_GFX, "rain_clouds_particle").as_deref(),
            Some("rain_clouds_file")
        );
        assert_eq!(
            entity_particle_name(WEATHER_ENTITIES, "nonexistent_entity"),
            None
        );
    }

    #[test]
    fn preserves_all_event_and_intra_event_effect_ordinals() {
        let source = r#"
entity = {
    name = "combined_weather_entity"
    default_state = "idle"
    state = {
        name = "idle"
        event = {
            time = 0
            sound = { soundeffect = "before" }
            particle = "first_particle"
            sound = { soundeffect = "after" }
            particle = "second_particle"
            keep_particle = yes
        }
        event = { trigger_once = no sound = { soundeffect = "only_sound" } }
    }
}
"#;
        let state =
            entity_default_state_events(source, "combined_weather_entity").expect("parse state");
        assert_eq!(state.state_name, "idle");
        assert_eq!(state.ordered_events.len(), 2);
        assert_eq!(state.ordered_events[0].authored_event_index, 0);
        assert_eq!(
            state.ordered_events[0].time_f32_bits,
            Some(0.0f32.to_bits())
        );
        assert_eq!(state.ordered_events[0].keep_particle, Some(true));
        assert_eq!(
            state.ordered_events[0].ordered_particle_effects,
            vec![
                WeatherAuthoredParticleEffect {
                    authored_effect_index: 1,
                    particle_token: "first_particle".into(),
                },
                WeatherAuthoredParticleEffect {
                    authored_effect_index: 3,
                    particle_token: "second_particle".into(),
                },
            ]
        );
        assert_eq!(state.ordered_events[1].authored_event_index, 1);
        assert!(state.ordered_events[1].ordered_particle_effects.is_empty());
        assert_eq!(state.ordered_events[1].trigger_once, Some(false));
    }

    #[test]
    fn parses_rain_clouds_asset_render_law() {
        let law = cloud_law_from_asset(RAIN_CLOUDS_ASSET, "rain_clouds_file").expect("parse asset");
        assert_eq!(law.texture, "gfx/particles/cloud_6.dds");
        assert_eq!(law.shader_effect, "ParticleAdditive");
        assert_eq!(law.blend, ParticleBlend::Additive);
        assert_eq!(law.color_rgb_255, [110.0, 120.0, 150.0]);
        assert_eq!(law.base_alpha_255, 180.0);
        assert_eq!(law.alpha_anim.as_deref(), Some("smoke_fade"));
        // rain smoke_fade peaks at 1.0.
        assert!((law.alpha_anim_peak - 1.0).abs() < 1e-6);
        assert!((law.size_mean - 60.5).abs() < 1e-6);
        assert!((law.life_mean - 2.6).abs() < 1e-6);
        // rain clouds declare no depth sort.
        assert_eq!(law.sort, None);
    }

    #[test]
    fn parses_snow_clouds_bare_alpha_syntax() {
        // snow_clouds uses `alpha=60.000,smoke_fade` (no braces).
        let law = cloud_law_from_asset(SNOW_CLOUDS_ASSET, "snow_clouds_file").expect("parse");
        assert_eq!(law.blend, ParticleBlend::AlphaBlend);
        assert_eq!(law.color_rgb_255, [200.0, 200.0, 200.0]);
        assert_eq!(law.base_alpha_255, 60.0);
        assert_eq!(law.alpha_anim.as_deref(), Some("smoke_fade"));
        // snow smoke_fade curve (0.000 0.000 0.321 0.750 0.635 0.914 1.000
        // 0.000) peaks at 0.914 — NOT 1.0; the static overlay must use it.
        assert!((law.alpha_anim_peak - 0.914).abs() < 1e-6);
    }

    #[test]
    fn lightning_asset_has_two_subsystems_and_reads_cloud_layer_first() {
        assert_eq!(asset_subsystem_count(LIGHTNING_CLOUDS_ASSET), 2);
        let law = cloud_law_from_asset(LIGHTNING_CLOUDS_ASSET, "lightning_storm_clouds_file")
            .expect("law");
        // First subsystem is the alpha-blended cloud layer, not the additive flash.
        assert_eq!(law.blend, ParticleBlend::AlphaBlend);
        assert_eq!(law.color_rgb_255, [160.0, 160.0, 160.0]);
        assert_eq!(law.base_alpha_255, 150.0);
        assert_eq!(law.alpha_anim.as_deref(), Some("cloud_fade"));
        // cloud_fade (0.1 -> 1.0 -> 1.0 -> 0.0) peaks at 1.0.
        assert!((law.alpha_anim_peak - 1.0).abs() < 1e-6);
        // The lightning cloud layer declares depth sorting.
        assert_eq!(law.sort.as_deref(), Some("depth"));
    }

    #[test]
    fn to_linear_vertex_color_matches_hand_computed() {
        // rain_clouds: rgb 110/120/150, alpha 180. ToLinear = (c/255)^2.2, alpha /255.
        let law = cloud_law_from_asset(RAIN_CLOUDS_ASSET, "rain_clouds_file").unwrap();
        let c = law.linear_vertex_color(1.0);
        let lin = |v: f32| (v / 255.0f32).powf(2.2);
        assert!((c[0] - lin(110.0)).abs() < 1e-6, "r {}", c[0]);
        assert!((c[1] - lin(120.0)).abs() < 1e-6, "g {}", c[1]);
        assert!((c[2] - lin(150.0)).abs() < 1e-6, "b {}", c[2]);
        assert!((c[3] - 180.0 / 255.0).abs() < 1e-6, "a {}", c[3]);
        // Alpha scales linearly with the animation multiplier; RGB does not.
        let half = law.linear_vertex_color(0.5);
        assert!((half[3] - (180.0 / 255.0) * 0.5).abs() < 1e-6);
        assert!((half[0] - c[0]).abs() < 1e-6);
    }

    #[test]
    fn blend_modes_match_d3d_equations() {
        let src = [0.4f32, 0.5, 0.6, 0.5];
        let dst = [0.2f32, 0.2, 0.2];
        // AlphaBlend: src*0.5 + dst*0.5
        let ab = ParticleBlend::AlphaBlend.blend_over(src, dst);
        assert!((ab[0] - (0.4 * 0.5 + 0.2 * 0.5)).abs() < 1e-6);
        // Additive: src*0.5 + dst
        let add = ParticleBlend::Additive.blend_over(src, dst);
        assert!((add[0] - (0.4 * 0.5 + 0.2)).abs() < 1e-6);
        // PreAlphaBlend: src + dst*0.5
        let pre = ParticleBlend::PreAlphaBlend.blend_over(src, dst);
        assert!((pre[0] - (0.4 + 0.2 * 0.5)).abs() < 1e-6);
    }

    #[test]
    fn animation_curve_sampling_and_peak() {
        // rain smoke_fade curve: 0->0.1, 0.3->0.1, 0.5->1.0, 0.8->1.0, 1.0->0.0
        assert!(
            (evaluate_animation_curve(RAIN_CLOUDS_ASSET, "smoke_fade", 0.0) - 0.1).abs() < 1e-6
        );
        assert!(
            (evaluate_animation_curve(RAIN_CLOUDS_ASSET, "smoke_fade", 0.5) - 1.0).abs() < 1e-6
        );
        // Midpoint 0.4 between (0.3,0.1) and (0.5,1.0): 0.1 + 0.9*0.5 = 0.55
        let mid = evaluate_animation_curve(RAIN_CLOUDS_ASSET, "smoke_fade", 0.4);
        assert!((mid - 0.55).abs() < 1e-6, "mid={mid}");
        // Peak of the curve is 1.0.
        assert!((animation_curve_peak(RAIN_CLOUDS_ASSET, "smoke_fade") - 1.0).abs() < 1e-6);
        // Missing curve -> 1.0.
        assert!((evaluate_animation_curve(RAIN_CLOUDS_ASSET, "nope", 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn decodes_weather_emitters_rows() {
        let table = weather_emitters_from_source(WEATHER_POSITIONS).expect("parse positions");
        assert_eq!(table.emitters.len(), 3);
        let first = table.emitters[0];
        assert_eq!(first.region_id, 1);
        assert!((first.position[0] - 2781.24).abs() < 1e-3);
        assert!((first.position[1] - 9.90).abs() < 1e-3);
        assert!((first.position[2] - 1571.49).abs() < 1e-3);
        assert_eq!(first.size, WeatherPositionSize::Small);
        // for_region filters correctly.
        assert_eq!(table.for_region(1).count(), 2);
        assert_eq!(table.for_region(7).count(), 1);
        assert_eq!(table.for_region(999).count(), 0);
        assert_eq!(table.emitters[2].size, WeatherPositionSize::Big);
    }

    #[test]
    fn weather_emitters_reject_unknown_size_tag() {
        // Shared parser accepts any 5-column row; our decoder fails closed on
        // an unrecognised size tag.
        assert!(weather_emitters_from_source("1;2.0;3.0;4.0;huge").is_err());
        // Shared parser itself rejects a missing size column (< 5 fields).
        assert!(weather_emitters_from_source("1;2.0;3.0").is_err());
    }

    #[test]
    fn parses_fade_defines_and_matches_vanilla_constant() {
        let d = weather_fade_defines_from_source(GRAPHICS_LUA).expect("parse defines");
        assert_eq!(d, VANILLA_WEATHER_FADE_DEFINES);
    }

    #[test]
    fn exact_camera_visibility_and_playback_branch_boundaries() {
        let d = VANILLA_WEATHER_FADE_DEFINES;
        let at = |distance| weather_visibility_law(distance, &d).expect("exact law");
        assert_eq!(at(357.0).primary_alpha, 1.0);
        assert_eq!(at(357.0).fading_alpha, 0.0);
        assert_eq!(at(358.0).primary_alpha, 1.0);
        assert_eq!(at(358.0).fading_alpha, 0.0);
        assert_eq!(at(468.0).primary_alpha, 1.0);
        assert!((at(468.0).fading_alpha - 0.5).abs() < 1e-6);
        assert_eq!(at(578.0).primary_alpha, 1.0);
        assert_eq!(at(578.0).fading_alpha, 1.0);
        assert_eq!(at(1100.0).primary_alpha, 1.0);
        assert_eq!(at(1100.0).fading_alpha, 1.0);
        assert!((at(1300.0).primary_alpha - 0.5).abs() < 1e-6);
        assert!((at(1300.0).fading_alpha - 0.5).abs() < 1e-6);
        assert_eq!(at(1500.0).primary_alpha, 0.0);
        assert_eq!(at(1500.0).fading_alpha, 0.0);
        assert_eq!(at(1501.0).primary_alpha, 0.0);
        assert_eq!(at(1501.0).fading_alpha, 0.0);
        assert_eq!(at(300.0).playback_rate, 1.0);
        assert!((at(400.0).playback_rate - 0.575).abs() < 1e-6);
        assert_eq!(at(500.0).playback_rate, 0.15);
        assert_eq!(weather_camera_fade(900.0, &d).expect("fade"), 1.0);
    }

    #[test]
    fn particle_shader_effects_follow_authored_subsystem_order() {
        assert_eq!(
            particle_asset_shader_effects(RAIN_CLOUDS_ASSET, "rain_clouds_file"),
            vec!["ParticleAdditive"]
        );
        let multi = r#"
particle = {
  name = "weather_mod_file"
  subsystem = { texture = { file = "a.dds" shader = "ModWeatherAlpha" } }
  subsystem = { texture = { file = "b.dds" shader = "ModWeatherAdd" } }
  subsystem = { texture = { file = "c.dds" shader = "ModWeatherAlpha" } }
}
"#;
        assert_eq!(
            particle_asset_shader_effects(multi, "weather_mod_file"),
            vec!["ModWeatherAlpha", "ModWeatherAdd"]
        );
    }

    #[test]
    fn particle_definition_count_rejects_same_file_duplicates_upstream() {
        let duplicate = r#"
particle = { name = "weather_mod_file" }
particle = { name = "weather_mod_file" }
"#;
        assert_eq!(
            particle_asset_definition_count(duplicate, "weather_mod_file"),
            2
        );
        assert!(particle_asset_defines(duplicate, "weather_mod_file"));
    }

    #[test]
    fn exact_distance_fog_parses_and_applies_shipped_law() {
        let source = r#"
static const float3 FOG_COLOR = float3( 0.12, 0.28, 0.6 );
static const float FOG_BEGIN = 1.0f;
static const float FOG_END = 150.0f;
static const float FOG_MAX = 0.35f;
// static const float FOG_MAX = 1000.7f;
"#;
        let defines =
            weather_distance_fog_defines_from_source(source).expect("distance fog defines");
        assert_eq!(defines, VANILLA_WEATHER_DISTANCE_FOG);
        let factor = weather_distance_fog_factor([0.0, 100.0, 100.0], [0.0, 0.0, 0.0], &defines)
            .expect("distance fog factor");
        let expected_distance = ((20_000.0f32 - 1.0) / (22_500.0 - 1.0)).min(0.35);
        let expected_direction = 1.0 - (100.0f32 / 20_000.0f32.sqrt()).abs();
        assert!((factor - expected_distance * expected_direction).abs() < 1e-6);
        let fogged =
            apply_weather_distance_fog([1.0, 1.0, 1.0], factor, &defines).expect("fog color");
        assert!(fogged[0] < 1.0 && fogged[1] < 1.0 && fogged[2] < 1.0);
    }

    #[test]
    fn end_to_end_resolver_chain_resolves_rain_cloud_law() {
        let root = temp_test_dir("weather_law_e2e");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, WEATHER_EFFECTS_SOURCE, WEATHER_TXT);
        write_text(&vanilla, WEATHER_ENTITIES_SOURCE, WEATHER_ENTITIES);
        write_text(&vanilla, PARTICLES_GFX_SOURCE, PARTICLES_GFX);
        write_text(
            &vanilla,
            "gfx/particles/environment/rain_clouds.asset",
            RAIN_CLOUDS_ASSET,
        );
        write_text(&vanilla, GRAPHICS_DEFINES_SOURCE, GRAPHICS_LUA);
        write_text(&vanilla, WEATHER_POSITIONS_SOURCE, WEATHER_POSITIONS);

        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let law =
            cloud_law_from_resolver(&resolver, WeatherType::RainLight, WeatherPositionSize::Big)
                .expect("resolve chain")
                .expect("rain has a cloud layer");
        assert_eq!(law.particle_file, "rain_clouds_file");
        assert_eq!(law.texture, "gfx/particles/cloud_6.dds");
        assert_eq!(law.blend, ParticleBlend::Additive);

        // sandstorm has no cloud layer -> Ok(None).
        let none =
            cloud_law_from_resolver(&resolver, WeatherType::Sandstorm, WeatherPositionSize::Big)
                .expect("sandstorm resolve");
        assert!(none.is_none());

        // Positions + fade defines resolve too.
        let positions = weather_emitters_from_resolver(&resolver).expect("positions");
        assert_eq!(positions.for_region(1).count(), 2);
        let defines = weather_fade_defines_from_resolver(&resolver).expect("defines");
        assert_eq!(defines, VANILLA_WEATHER_FADE_DEFINES);
    }

    #[test]
    fn mod_override_of_weather_positions_wins() {
        let root = temp_test_dir("weather_law_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, WEATHER_POSITIONS_SOURCE, WEATHER_POSITIONS);
        // Mod replaces the file with a single big row for region 42.
        write_text(
            &mod_root,
            WEATHER_POSITIONS_SOURCE,
            "42;100.0;10.0;200.0;big\n",
        );

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let positions = weather_emitters_from_resolver(&resolver).expect("positions");
        assert_eq!(positions.emitters.len(), 1);
        assert_eq!(positions.emitters[0].region_id, 42);
        assert_eq!(positions.emitters[0].size, WeatherPositionSize::Big);
    }
}
