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
//! region (the extractor feed supplies the type; this module never invents it),
//! how does the engine draw the overlay — 100% from shipped files, with every
//! parameter cited or explicitly `UNTRACED`.
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
//!   simulation params (spawn/motion; not needed to draw one static frame, kept
//!   for fidelity/UNTRACED accounting).
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
//! `ParticleAlphaBlend`, absent on `ParticleAdditive`) applies
//! `ApplyDistanceFog` to RGB before return (`particle.shader:189-191`); with no
//! captured fog constants that stage is `UNTRACED` (identity here).
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
//! The engine fades the whole weather layer by camera height. The *constants*
//! are cited exactly; the *formula* that combines them is engine C++ with no
//! shipped-text source, so `weather_camera_fade` is a documented reconstruction
//! (`UNTRACED` formula, cited constants). Constants:
//! `WEATHER_DISTANCE_CUTOFF=1500` (hidden beyond),
//! `WEATHER_DISTANCE_FADE_LENGTH=400`, `WEATHER_ZOOM_IN_CUTOFF=358`,
//! `WEATHER_ZOOM_IN_FADE_LENGTH=220`, `WEATHER_ZOOM_IN_FADE_FACTOR=0.0`.
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
//! spawn positions, the fade constants. Runtime state (NOT in any file, must
//! come from the extractor feed / a capture moment): the per-region *current
//! weather type* and its intensity/phase, the live particle ages (hence exact
//! per-particle `smoke_fade` alpha), the camera height (hence the fade factor),
//! and the animated turbulence offsets. This module draws the fully-traced
//! static overlay for a *given* type and leaves the rest `UNTRACED` with a
//! named capture plan (see `documents/WEATHER_LAW.md`).

use crate::image_load::load_dds_rgba;
use crate::mod_resolver::GameFileResolver;
use crate::types::RgbaImage;
use crate::Result;
use std::path::Path;

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

/// `standardfuncsgfx.fxh:48-56` `ToLinear` gamma exponent.
pub const TO_LINEAR_GAMMA: f32 = 2.2;

/// The weather *type* keys used by `map/strategicregions/*.txt` weather periods
/// and keyed by `common/weather.txt` `weather_effects`. Only the types with a
/// particle overlay are represented here (`no_phenomenon` draws nothing;
/// `arctic_water`/`mud` are ground post-effects, not airborne particles).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
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
}

/// The `big`/`small` variant of a weather emitter position
/// (`map/weatherpositions.txt` column 5). `big` selects the primary
/// `weather_effect`, `small` selects the `small = yes` entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
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

/// The `WEATHER_*` camera-fade constants (`00_graphics.lua:1008-1015`). The
/// values are cited exactly; the fade *formula* combining them is engine C++
/// (see [`weather_camera_fade`]).
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
};

/// `UNTRACED` fade reconstruction from the cited constants. The engine's exact
/// combination of the distance-cutoff and zoom-in bands is C++ with no
/// shipped-text source; this is the documented best reconstruction:
/// a far fade that ramps 1 -> 0 over `[cutoff-fade_length, cutoff]`, and a near
/// fade that ramps `1 -> zoom_in_fade_factor` as camera distance drops through
/// `[zoom_in_cutoff, zoom_in_cutoff - zoom_in_fade_length]`. Returns a 0..1
/// visibility multiplier on the whole weather layer. The comment names this
/// UNTRACED so a merge reviewer sees the constants are real but the shape is not
/// pinned by a file.
pub fn weather_camera_fade(camera_distance: f32, defines: &WeatherFadeDefines) -> f32 {
    // Far fade: fully hidden at/after distance_cutoff, full below the band start.
    let far = if camera_distance >= defines.distance_cutoff {
        0.0
    } else if camera_distance <= defines.distance_cutoff - defines.distance_fade_length {
        1.0
    } else {
        (defines.distance_cutoff - camera_distance) / defines.distance_fade_length.max(f32::EPSILON)
    };

    // Near (zoom-in) fade: at/above zoom_in_cutoff -> full; at/below the band
    // bottom -> zoom_in_fade_factor; linear between.
    let band_bottom = defines.zoom_in_cutoff - defines.zoom_in_fade_length;
    let near = if camera_distance >= defines.zoom_in_cutoff {
        1.0
    } else if camera_distance <= band_bottom {
        defines.zoom_in_fade_factor
    } else {
        let t = (camera_distance - band_bottom) / defines.zoom_in_fade_length.max(f32::EPSILON);
        defines.zoom_in_fade_factor + (1.0 - defines.zoom_in_fade_factor) * t
    };

    (far * near).clamp(0.0, 1.0)
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
    if entries.is_empty() {
        return Err("common/weather.txt: weather_effects had no recognised weather types".into());
    }
    Ok(WeatherEffectsTable { entries })
}

/// Resolve the entity name (`<name>_clouds_entity`) to its `particle` name via
/// `gfx/entities/weather_entities.asset` (`entity -> event.particle`).
pub fn entity_particle_name(entities_source: &str, entity_name: &str) -> Option<String> {
    // Find the `entity = { ... name = "<entity_name>" ... }` block and read its
    // `event = { particle = "<name>" ... }`.
    for entity_block in each_braced_value(entities_source, "entity") {
        if braced_field(&entity_block, "name").as_deref() == Some(entity_name) {
            // The particle is nested in state = { event = { particle = "..." } }.
            if let Some(p) = field_after_token(&entity_block, "particle") {
                return Some(p);
            }
        }
    }
    None
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
    })
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

/// A rendered top-down weather overlay: RGBA8 premultiplied-over-nothing atop a
/// transparent map plane. This is the LAW made visible for the orchestrator's
/// eyeball — it draws every resolved cloud emitter at its
/// `map/weatherpositions.txt` `(x,z)` location, sprite-splatted at the asset
/// `size_mean` (map units -> pixels), tinted by `ToLinear(color)` and composited
/// with the asset's `ParticleBlend`.
///
/// NOTE (honest): the shipped `particle.shader` draws camera-facing billboards
/// through the runtime `ViewProjectionMatrix` (a per-frame capture/camera CB,
/// an UNTRACED runtime input owned by the camera lane). This overlay is a
/// deliberately *orthographic top-down* projection so the law's file-derived
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
        let anim_mul = law
            .alpha_anim
            .as_deref()
            .map(|name| {
                // Peak phase of the alpha curve is the representative frame.
                // (Re-read from the asset would need re-resolution; the curve
                // peak for the shipped clouds is 1.0, so use the base alpha.)
                let _ = name;
                1.0f32
            })
            .unwrap_or(1.0);
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

/// Read the first `token = "value"` / `token = value` occurrence anywhere in a
/// block (used for the deeply nested `particle = "..."` in an entity's state).
fn field_after_token(block: &str, token: &str) -> Option<String> {
    braced_field(block, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::{Path, PathBuf};

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
    fn parses_rain_clouds_asset_render_law() {
        let law = cloud_law_from_asset(RAIN_CLOUDS_ASSET, "rain_clouds_file").expect("parse asset");
        assert_eq!(law.texture, "gfx/particles/cloud_6.dds");
        assert_eq!(law.shader_effect, "ParticleAdditive");
        assert_eq!(law.blend, ParticleBlend::Additive);
        assert_eq!(law.color_rgb_255, [110.0, 120.0, 150.0]);
        assert_eq!(law.base_alpha_255, 180.0);
        assert_eq!(law.alpha_anim.as_deref(), Some("smoke_fade"));
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
    fn camera_fade_hides_beyond_cutoff_and_full_in_band() {
        let d = VANILLA_WEATHER_FADE_DEFINES;
        // Beyond distance cutoff -> hidden.
        assert_eq!(weather_camera_fade(2000.0, &d), 0.0);
        // Comfortably inside both bands (e.g. 900): far=1, near=1 -> 1.
        assert!((weather_camera_fade(900.0, &d) - 1.0).abs() < 1e-6);
        // In the far fade band: halfway (1300 of [1100,1500]) -> 0.5.
        assert!((weather_camera_fade(1300.0, &d) - 0.5).abs() < 1e-6);
        // Fully zoomed in (below band bottom 138) -> zoom_in_fade_factor (0.0).
        assert_eq!(weather_camera_fade(50.0, &d), 0.0);
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
